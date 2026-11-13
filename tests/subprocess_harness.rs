//! Governed subprocess harness (issue #154, slice E) — discriminating tests.
//!
//! Each test drives the REAL subprocess path: an [`ExecutionHarness`]
//! adapter whose `run` invokes the declared capability through
//! [`HarnessDispatch::call`] (the only sanctioned effect path), a
//! [`SubprocessHarness`] registration in the [`HarnessRegistry`], and the
//! [`CapabilityRegistry`] handler that spawns allowlisted executables
//! inside a governed workspace with deadlines, cancellation, and output
//! caps. Fixtures are harmless and deterministic (system version commands,
//! sleep, marker creation); nothing depends on live commercial APIs.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDeclarationV1, HarnessDispatch, HarnessOutcome, HarnessRegistry,
    HarnessStep, canonical_harness_declarations,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::subprocess_harness::{
    SUBPROCESS_EXEC_CAPABILITY, SUBPROCESS_HARNESS_ID, SubprocessConfig, SubprocessHarness,
    subprocess_capability,
};

mod common;

const NODE_ID: &str = "subprocess-step";

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed subprocess fixture",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .unwrap()
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://x".into()],
        writable_scopes: vec!["work://y".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 3,
        escalation_target: "human-review".into(),
    }
}

/// A governed runner with the subprocess capability declared under the
/// execute cap. `permit_caps` controls the governance grant; the registry
/// always declares the real capability.
fn runner_for(
    config: SubprocessConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        SUBPROCESS_EXEC_CAPABILITY,
        subprocess_capability(config, workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn harness() -> SubprocessHarness {
    SubprocessHarness::new()
}

fn dispatch<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, vec![], runner)
}

/// A governed dispatch carrying request-scoped known secrets — the
/// authoritative redaction boundary for a run.
fn dispatch_with_secrets<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
    secrets: Vec<String>,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, secrets, runner)
}

fn step(args: serde_json::Value) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: SUBPROCESS_EXEC_CAPABILITY.to_string(),
        args,
    }
}

/// Cross-platform sleeper: long enough that only a deadline/cancel can
/// stop it. Returns (executable, argv).
fn sleeper() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        ("sleep", vec!["30".to_string()])
    }
}

/// A fixture that would create `marker.txt` in the child cwd if it ran.
/// Used to prove refusal happened before any effect.
fn marker_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "New-Item -ItemType File -Path marker.txt".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        ("touch", vec!["marker.txt".to_string()])
    }
}

/// A fixture whose direct child spawns a detached grandchild that sleeps
/// and then writes `grandchild-marker.txt` in its cwd. If tree containment
/// is absent, the grandchild survives the parent's termination and the
/// marker appears after the test's run returns.
fn grandchild_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Process powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 3; New-Item -ItemType File -Path grandchild-marker.txt'".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "sh",
            vec![
                "-c".to_string(),
                "( sleep 3; touch grandchild-marker.txt ) & sleep 30".to_string(),
            ],
        )
    }
}

/// A fixture printing credential-shaped output on stdout.
fn credential_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Write-Output 'Authorization: Bearer test-secret-deep-41'".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "printf",
            vec![
                "%s".to_string(),
                "Authorization: Bearer test-secret-deep-41".to_string(),
            ],
        )
    }
}

/// A Windows-only fixture that tries to write a token through an arbitrary
/// raw handle value. Used to prove an unrelated inheritable handle created
/// by the parent is NOT usable by the child.
#[cfg(windows)]
fn sentinel_probe_fixture(sentinel: i64) -> (&'static str, Vec<String>) {
    // One-liner executed by powershell: compiles a tiny P/Invoke wrapper
    // and calls WriteFile on the raw handle value supplied by the test.
    // Prints nothing when the write fails (the expected, governed case).
    let script = format!(
        "$code='using System;using System.Runtime.InteropServices;public static class SentinelProbe{{[DllImport(\"kernel32.dll\",SetLastError=true)]public static extern bool WriteFile(IntPtr hFile,byte[] buffer,uint nNumberOfBytesToWrite,out uint lpNumberOfBytesWritten,IntPtr lpOverlapped);}}' ; Add-Type -TypeDefinition $code ; $written=[uint32]0 ; $bytes=[System.Text.Encoding]::ASCII.GetBytes('SENTINEL_TOKEN') ; [SentinelProbe]::WriteFile([IntPtr]{sentinel},$bytes,[uint32]$bytes.Length,[ref]$written,[IntPtr]::Zero) | Out-Null"
    );
    (
        "powershell",
        vec!["-NoProfile".to_string(), "-Command".to_string(), script],
    )
}

fn version_args() -> serde_json::Value {
    serde_json::json!({"exe": "cargo", "argv": ["--version"]})
}

/// A fixture printing a deterministic burst of lines and exiting quickly.
fn burst_payload_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "1..2000 | ForEach-Object { Write-Output \"PAYLOAD-$_\" }".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "sh",
            vec![
                "-c".to_string(),
                "i=1; while [ $i -le 2000 ]; do printf \"PAYLOAD-%s\\n\" $i; i=$((i+1)); done"
                    .to_string(),
            ],
        )
    }
}

fn expected_burst_payload() -> String {
    #[cfg(windows)]
    let nl = "\r\n";
    #[cfg(not(windows))]
    let nl = "\n";
    let mut out = (1..=2000u32)
        .map(|i| format!("PAYLOAD-{i}"))
        .collect::<Vec<_>>()
        .join(nl);
    out.push_str(nl);
    out
}

/// A fixture printing an arbitrary opaque secret token on stdout.
fn opaque_secret_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Write-Output 'OPAQUE-CANARY-1234-no-regex-shape'".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "printf",
            vec![
                "%s".to_string(),
                "OPAQUE-CANARY-1234-no-regex-shape".to_string(),
            ],
        )
    }
}

/// Decode a lowercase-hex field from the subprocess result document.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks(2) {
        let hex = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(hex, 16).ok()?);
    }
    Some(out)
}

/// Decode a lowercase-hex field from the subprocess result document.
fn stdout_of(output: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(output).expect("result JSON parses");
    let hex = parsed["stdoutHex"].as_str().expect("stdoutHex field");
    let mut bytes = Vec::new();
    for pair in hex.as_bytes().chunks(2) {
        let v = u8::from_str_radix(std::str::from_utf8(pair).expect("hex pairs"), 16)
            .expect("hex pair");
        bytes.push(v);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 1 — A permitted invocation succeeds and journals the evidence.
#[tokio::test]
async fn permitted_invocation_succeeds() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = SubprocessConfig::default();
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect("run succeeds")
    };

    assert_eq!(outcome.harness_id, SUBPROCESS_HARNESS_ID);
    assert!(
        stdout_of(&outcome.output).contains("cargo"),
        "output carries version: {}",
        outcome.output
    );
    assert_eq!(outcome.events.len(), 2);
    outcome.verify().expect("sealed outcome verifies");
    assert!(is_hex64(&outcome.evidence_entry_digest));
    // Journal advanced exactly one terminal entry for this step.
    assert_eq!(runner.journal().len(), 1);
    assert_eq!(runner.journal()[0].identity_key, "step-1");
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// 2 — An undeclared capability refuses before any effect exists.
#[tokio::test]
async fn undeclared_capability_refuses_before_effects() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = marker_fixture();
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let mut bad = step(serde_json::json!({"exe": exe, "argv": argv}));
    bad.capability = "harness.subprocess.other".to_string();

    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(bad, &mut d)
            .await
            .expect_err("undeclared capability refuses")
    };
    assert!(
        err.to_string().contains("outside its declaration"),
        "got: {err}"
    );
    // No gate-9 journal entry exists and no marker file was created:
    // nothing ran.
    assert!(runner.journal().is_empty());
    assert!(!root.path().join("marker.txt").exists());
}

/// 3 — Execution without the governance grant refuses before effects.
#[tokio::test]
async fn unauthorized_execution_refuses_before_effects() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = marker_fixture();
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    // The permit governs the node but grants a DIFFERENT capability.
    let mut runner = runner_for(config, root.path().to_path_buf(), &["other.cap"]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect_err("unauthorized execution refuses")
    };
    assert!(
        err.to_string().contains("SOMA-AUTH"),
        "governance refusal got: {err}"
    );
    assert!(runner.journal().is_empty());
    assert!(!root.path().join("marker.txt").exists());
}

/// 4 — An escaping or missing workspace refuses without any fallback.
#[tokio::test]
async fn invalid_workspace_refuses_without_source_checkout_fallback() {
    let base = tempfile::tempdir().expect("base tempdir");
    let workspace = base.path().join("ws");
    std::fs::create_dir(&workspace).expect("workspace dir");
    let source_checkout = base.path().join("src");
    std::fs::create_dir(&source_checkout).expect("source dir");
    let (exe, argv) = marker_fixture();
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(config, workspace.clone(), &[SUBPROCESS_EXEC_CAPABILITY]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    // cwd escapes the governed workspace ("../src" resolves outside it).
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"exe": exe, "argv": argv, "cwd": "../src"})),
                &mut d,
            )
            .await
            .expect_err("escaping cwd refuses")
    };
    assert!(err.to_string().contains("escapes"), "got: {err}");
    assert!(runner.journal().is_empty());
    // The would-be fallback location gained no marker: nothing ran anywhere.
    assert!(!source_checkout.join("marker.txt").exists());
    assert!(!workspace.join("marker.txt").exists());

    // A missing governed root refuses just as closedly (no other directory
    // is substituted for it).
    let missing_root = base.path().join("does-not-exist");
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(config, missing_root, &[SUBPROCESS_EXEC_CAPABILITY]);
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect_err("missing workspace refuses")
    };
    assert!(err.to_string().contains("missing"), "got: {err}");
    assert!(runner.journal().is_empty());
    assert!(!source_checkout.join("marker.txt").exists());
}

/// 5a — Timeout kills and reaps the child promptly.
#[tokio::test]
async fn timeout_terminates_safely() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = sleeper();
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .timeout_ms(5_000);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let started = Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"exe": exe, "argv": argv, "timeoutMs": 300})),
                &mut d,
            )
            .await
            .expect_err("sleeper must time out")
    };
    let elapsed = started.elapsed();
    assert!(err.to_string().contains("timed out"), "got: {err}");
    assert!(elapsed < Duration::from_secs(15), "wall time: {elapsed:?}");
}

/// 5b — Cancellation kills and reaps the child promptly.
#[tokio::test]
async fn cancellation_terminates_safely() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = sleeper();
    let cancel = Arc::new(AtomicBool::new(false));
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .timeout_ms(60_000)
        .cancel(cancel.clone());
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let started = Instant::now();
    let err = std::thread::scope(|scope| {
        let harness_ref = &harness;
        let mut d = dispatch(harness_ref, &m, &r, &mut runner);
        let handle = scope.spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(
                    harness_ref.run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d),
                )
        });
        std::thread::sleep(Duration::from_millis(200));
        cancel.store(true, Ordering::Relaxed);
        handle.join().expect("run thread")
    })
    .expect_err("cancellation must refuse the command");
    let elapsed = started.elapsed();
    assert!(err.to_string().contains("cancelled"), "got: {err}");
    assert!(elapsed < Duration::from_secs(15), "wall time: {elapsed:?}");
}

/// 6 — Output beyond the configured budget is a typed refusal (R3).
/// The budget binds the bytes the child may write, and the final size
/// check after exit closes the fast-exit bypass: nothing is reported as
/// success once the breach is observed.
#[tokio::test]
async fn output_limits_are_enforced() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = SubprocessConfig::default().max_output_bytes(16);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    // `cargo --version` writes ~38 bytes: a fast exit, over the 16-byte
    // budget. It must not be reported as (truncated) success.
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect_err("budget breach must refuse")
    };
    assert!(
        err.to_string().contains("budget"),
        "typed breach got: {err}"
    );

    // Within budget the same output is retained and succeeds.
    let outcome = {
        let config = SubprocessConfig::default();
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[SUBPROCESS_EXEC_CAPABILITY],
        );
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect("run succeeds")
    };
    let parsed: serde_json::Value = serde_json::from_str(&outcome.output).expect("output is JSON");
    assert!(
        stdout_of(&outcome.output).contains("cargo"),
        "retained: {}",
        parsed["stdoutHex"]
    );
    assert_eq!(parsed["success"], true);
}

/// 6b — A fast large burst of output cannot bypass the budget check.
#[tokio::test]
async fn output_burst_cannot_bypass_budget() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = credential_fixture();
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .max_output_bytes(8);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect_err("burst over budget must refuse")
    };
    assert!(
        err.to_string().contains("budget"),
        "typed breach got: {err}"
    );
}

/// 7 — Nonzero exit is a typed failure, never a success.
#[tokio::test]
async fn nonzero_exit_is_not_reported_as_success() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = SubprocessConfig::default();
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"exe": "cargo", "argv": ["--definitely-not-a-flag"]})),
                &mut d,
            )
            .await
            .expect_err("nonzero exit must refuse")
    };
    assert!(err.to_string().contains("code"), "typed failure got: {err}");
}

/// 8 — Durable evidence survives process termination.
#[tokio::test]
async fn durable_evidence_survives_process_termination() {
    let root = tempfile::tempdir().expect("tempdir");
    let store = tempfile::tempdir().expect("evidence dir");
    let outcome = {
        let config = SubprocessConfig::default();
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[SUBPROCESS_EXEC_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect("run succeeds")
        // runner, harness, and dispatch all drop here — the "process" ends.
    };

    let path = store.path().join("outcome.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&outcome).expect("serializes"),
    )
    .expect("persist");
    drop(outcome);

    let reloaded: HarnessOutcome =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("reload"))
            .expect("outcome parses");
    reloaded.verify().expect("reloaded evidence verifies");
    assert!(is_hex64(&reloaded.evidence_entry_digest));
    assert_eq!(reloaded.events.len(), 2);
}

/// 9 — Incompatible checkpoint bindings refuse.
#[tokio::test]
async fn stale_or_incompatible_resume_evidence_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let outcome = {
        let config = SubprocessConfig::default();
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[SUBPROCESS_EXEC_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect("run succeeds")
    };
    let harness = harness();
    let digest = outcome.evidence_entry_digest.clone();

    // Fresh checkpoint binds and resumes.
    let checkpoint = futures::executor::block_on(harness.checkpoint(&outcome, &digest))
        .expect("checkpoint exports");
    let continuation = futures::executor::block_on(harness.resume(&checkpoint, &digest))
        .expect("matching resume succeeds");
    assert_eq!(continuation.harness_id, SUBPROCESS_HARNESS_ID);

    // Stale portable digest refuses.
    let err = futures::executor::block_on(harness.resume(&checkpoint, &"0".repeat(64)))
        .expect_err("stale digest refuses");
    assert!(err.to_string().contains("portable"), "got: {err}");

    // Incompatible harness revision refuses.
    let mut wrong_rev = checkpoint.clone();
    wrong_rev.harness_revision = "v9".to_string();
    wrong_rev.content_digest = None;
    let wrong_rev = wrong_rev.sealed();
    let err = futures::executor::block_on(harness.resume(&wrong_rev, &digest))
        .expect_err("wrong revision refuses");
    assert!(err.to_string().contains("revision"), "got: {err}");

    // Tampered checkpoint (digest no longer verifies) refuses.
    let mut tampered = checkpoint.clone();
    tampered.portable_state_digest = "1".repeat(64);
    let err = futures::executor::block_on(harness.resume(&tampered, &"1".repeat(64)))
        .expect_err("tampered checkpoint refuses");
    assert!(err.to_string().contains("contentDigest"), "got: {err}");
}

/// 10 — The real declaration is in the generated catalog, and catalog
/// drift is detectable.
#[test]
fn catalog_contains_subprocess_declaration_and_drift_is_detectable() {
    struct StaticHarness {
        declaration: HarnessDeclarationV1,
    }

    #[async_trait::async_trait]
    impl ExecutionHarness for StaticHarness {
        fn declaration(&self) -> &HarnessDeclarationV1 {
            &self.declaration
        }
        async fn run(
            &self,
            _step: HarnessStep,
            _dispatch: &mut HarnessDispatch<'_>,
        ) -> anyhow::Result<HarnessOutcome> {
            anyhow::bail!("static probe is not executable")
        }
    }

    let decls = canonical_harness_declarations();
    assert!(
        decls.iter().any(|d| d.harness_id == SUBPROCESS_HARNESS_ID),
        "canonical inventory must include the subprocess harness"
    );

    let mut registry = HarnessRegistry::new();
    for d in &decls {
        registry
            .register(Box::new(StaticHarness {
                declaration: d.clone(),
            }))
            .expect("production declarations register");
    }
    let catalog = registry.catalog();
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == SUBPROCESS_HARNESS_ID)
        .expect("catalog carries the subprocess entry");
    assert_eq!(
        entry.declared_capabilities,
        vec![SUBPROCESS_EXEC_CAPABILITY.to_string()]
    );
    assert!(catalog.verify_against_declarations(&decls).is_empty());

    // Any declaration change moves the catalog digest: drift is detectable.
    let mut drifted = decls.clone();
    for d in drifted.iter_mut() {
        if d.harness_id == SUBPROCESS_HARNESS_ID {
            d.declared_capabilities = vec!["harness.subprocess.other".to_string()];
            d.claimed_capabilities = vec!["harness.subprocess.other".to_string()];
        }
        d.content_digest = None;
        let sealed = d.clone().sealed();
        *d = sealed;
    }
    let mut drifted_registry = HarnessRegistry::new();
    for d in &drifted {
        drifted_registry
            .register(Box::new(StaticHarness {
                declaration: d.clone(),
            }))
            .expect("drifted declaration still registers");
    }
    let drifted_catalog = drifted_registry.catalog();
    assert_ne!(
        drifted_catalog.content_digest, catalog.content_digest,
        "a capability change must move the catalog digest"
    );
    assert!(
        !drifted_catalog
            .verify_against_declarations(&decls)
            .is_empty(),
        "drifted catalog must not verify against the canonical declarations"
    );
}

/// A fixture whose direct child exits immediately after spawning a
/// detached grandchild that sleeps and then writes
/// `grandchild-marker.txt` in its cwd. The spawn is suspend-contained,
/// so the grandchild starts inside the job even if the parent exits the
/// first instant it runs.
fn fast_exit_grandchild_fixture() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Process powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 3; New-Item -ItemType File -Path grandchild-marker.txt'".to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "sh",
            vec![
                "-c".to_string(),
                "( sleep 3; touch grandchild-marker.txt ) & true".to_string(),
            ],
        )
    }
}

/// R1 — process-tree containment: a direct child that spawns a detached
/// grandchild attempting a delayed workspace mutation is contained. After
/// the timeout returns, the grandchild must not fire: terminate the whole
/// owned tree, not just the direct child.
#[tokio::test]
async fn grandchild_tree_terminated_on_timeout() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = grandchild_fixture();
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .timeout_ms(5_000);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let marker = root.path().join("grandchild-marker.txt");
    assert!(!marker.exists(), "marker must not pre-exist");

    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"exe": exe, "argv": argv, "timeoutMs": 300})),
                &mut d,
            )
            .await
            .expect_err("direct child must time out")
    };
    assert!(err.to_string().contains("timed out"), "got: {err}");

    // The grandchild's delayed mutation was scheduled for ~3s after its
    // start. Well past that point, no mutation may have occurred.
    std::thread::sleep(Duration::from_secs(5));
    assert!(
        !marker.exists(),
        "grandchild survived the timeout and mutated the workspace"
    );
}

/// R3.1 — an immediately-exiting parent's delayed grandchild mutation
/// must not happen: containment is installed before any child code runs,
/// and cleanup sweeps the tree on normal exit.
#[tokio::test]
async fn immediate_parent_exit_grandchild_contained() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = fast_exit_grandchild_fixture();
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .timeout_ms(5_000);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let marker = root.path().join("grandchild-marker.txt");
    assert!(!marker.exists(), "marker must not pre-exist");

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("immediate parent exits cleanly")
    };
    let parsed: serde_json::Value = serde_json::from_str(&outcome.output).expect("output is JSON");
    assert_eq!(
        parsed["success"], true,
        "child succeeded: {}",
        outcome.output
    );

    // The grandchild's delayed mutation was scheduled for ~3s after its
    // start. Well past that point, no mutation may have occurred.
    std::thread::sleep(Duration::from_secs(5));
    assert!(
        !marker.exists(),
        "grandchild of an immediately exiting parent mutated the workspace"
    );
}

/// R2 — spool safety: hostile pre-existing spool paths must not redirect
/// child output or overwrite unrelated files.
#[tokio::test]
async fn hostile_spool_precreation_does_not_redirect_or_overwrite() {
    // Pre-create exclusive-path candidates under the two naming schemes the
    // old code used (`prometheos-subprocess-{pid}-{counter}.out|.err`), plus
    // a canary file the spool code must never touch.
    let pid = std::process::id();
    let dir = std::env::temp_dir();
    let mut victims = Vec::new();
    for counter in 0..99u32 {
        for ext in ["out", "err"] {
            let path = dir.join(format!("prometheos-subprocess-{pid}-{counter}.{ext}"));
            if !path.exists() {
                let _ = std::fs::write(&path, "PREEXISTING-SPOOL-CANARY");
                victims.push(path);
            }
        }
    }
    #[cfg(unix)]
    {
        // A symlink candidate: old-style `File::create` follows it and
        // truncates the target; `create_new` must not.
        let canary = dir.join(format!("prometheos-subprocess-victim-{pid}.txt"));
        std::fs::write(&canary, "victim-canary").expect("canary writes");
        let link = dir.join(format!("prometheos-subprocess-{pid}-0.out"));
        let _ = std::os::unix::fs::symlink(&canary, &link);
        victims.push(canary);
    }

    let root = tempfile::tempdir().expect("workspace");
    let config = SubprocessConfig::default();
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(version_args()), &mut d)
            .await
            .expect("run succeeds despite pre-created candidates")
    };
    assert!(
        stdout_of(&outcome.output).contains("cargo"),
        "real child output, not redirected: {}",
        outcome.output
    );

    // Every pre-created victim must hold its original contents.
    for v in &victims {
        let content = std::fs::read_to_string(v).unwrap_or_default();
        assert!(
            content == "PREEXISTING-SPOOL-CANARY" || content == "victim-canary",
            "victim {} was overwritten: {content:?}",
            v.display()
        );
    }
    for v in &victims {
        let _ = std::fs::remove_file(v);
    }
}

/// R4 — structured diagnostic redaction: credential-shaped output must
/// remain parseable JSON after redaction, and the secret must not appear
/// in persisted evidence.
#[tokio::test]
async fn credential_diagnostics_stay_parseable_and_redacted() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = credential_fixture();
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("credential-shaped output still succeeds")
    };

    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("redacted output must remain valid JSON");
    let stdout = stdout_of(&outcome.output);
    assert!(stdout.contains("<redacted>"), "redacted: {stdout}");
    assert!(!stdout.contains("test-secret"), "leak in stdout: {stdout}");
    assert_eq!(
        parsed["success"], true,
        "child succeeded; outcome is success"
    );
    // The persisted evidence (gate-7 output = outcome.output) is canary-free.
    assert!(
        !outcome.output.contains("test-secret"),
        "canary persisted in evidence: {}",
        outcome.output
    );
}

/// R3.2 — an arbitrary opaque known secret that matches NO credential
/// regex must still never appear in decoded evidence: known secrets are
/// redacted verbatim before encoding, and the encoded bytes cannot leak
/// them through gate-7 or the durable JSON.
#[tokio::test]
async fn opaque_known_secret_redacted_before_persistence() {
    let root = tempfile::tempdir().expect("tempdir");
    let canary = "OPAQUE-CANARY-1234-no-regex-shape";
    let (exe, argv) = opaque_secret_fixture();
    let config = SubprocessConfig::default()
        .allowed_executables(vec![exe.to_string()])
        .known_secrets(vec![canary.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("opaque known secret still succeeds")
    };

    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("output must remain valid JSON");
    assert_eq!(parsed["success"], true);
    let stdout = stdout_of(&outcome.output);
    assert!(
        !stdout.contains(canary),
        "canary leaked into stdout: {stdout}"
    );
    assert!(stdout.contains("<redacted>"), "redacted marker: {stdout}");
    // The persisted, gate-7-processed evidence must not expose the canary
    // in plaintext NOR in reversible encoding.
    assert!(
        !outcome.output.contains(canary),
        "canary persisted in evidence: {}",
        outcome.output
    );
    for value in [
        parsed["stdoutHex"].as_str().unwrap(),
        parsed["stderrHex"].as_str().unwrap(),
    ] {
        if let Some(decoded) = decode_hex(value) {
            let text = String::from_utf8_lossy(&decoded);
            assert!(!text.contains(canary), "canary in decoded evidence: {text}");
        }
    }
}

/// R4 — an opaque secret that exists ONLY in the governed request must not
/// reach decoded persisted evidence: the handler redacts with the
/// request's list, not an independent config list.
#[tokio::test]
async fn request_scoped_opaque_secret_redacted() {
    let root = tempfile::tempdir().expect("tempdir");
    let canary = "OPAQUE-CANARY-1234-no-regex-shape";
    let (exe, argv) = opaque_secret_fixture();
    // Deliberately NO known secrets in the registration-time config — the
    // secret arrives only via the governed dispatch.
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![canary.to_string()]);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("request-scoped opaque secret still succeeds")
    };

    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("output must remain valid JSON");
    assert_eq!(parsed["success"], true);
    let stdout = stdout_of(&outcome.output);
    assert!(
        !stdout.contains(canary),
        "canary leaked into stdout: {stdout}"
    );
    assert!(stdout.contains("<redacted>"), "redacted marker: {stdout}");
    // The persisted, gate-7-processed evidence must not expose the canary
    // in plaintext NOR in reversible encoding.
    assert!(
        !outcome.output.contains(canary),
        "canary persisted in evidence: {}",
        outcome.output
    );
    for value in [
        parsed["stdoutHex"].as_str().unwrap(),
        parsed["stderrHex"].as_str().unwrap(),
    ] {
        if let Some(decoded) = decode_hex(value) {
            let text = String::from_utf8_lossy(&decoded);
            assert!(!text.contains(canary), "canary in decoded evidence: {text}");
        }
    }
}

/// R4 — capture completion is verified before evidence is sealed: a
/// fast-exiting process cannot leave un-captured pipe data, and success
/// is only reported with finished drains.
#[tokio::test]
async fn quick_exit_capture_is_exact() {
    let root = tempfile::tempdir().expect("tempdir");
    let (exe, argv) = burst_payload_fixture();
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("burst succeeds")
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("output must be valid JSON");
    assert_eq!(
        parsed["success"], true,
        "child succeeded: {}",
        outcome.output
    );
    let stdout = stdout_of(&outcome.output);
    assert_eq!(
        stdout,
        expected_burst_payload(),
        "capture must be exact; got {} bytes",
        stdout.len()
    );
}

/// Handle-inheritance isolation (R4-P1-C): an unrelated inheritable handle
/// created by the parent must NOT be usable to reach the child — the
/// attribute list whitelist carries only the three intended stdio handles.
/// The child attempts to write a magic token through the shared handle
/// value; the write token (or its absence) on the sentinel pipe is
/// decisive.
#[cfg(windows)]
#[tokio::test]
async fn unrelated_sentinel_handle_not_inherited() {
    use winapi::shared::minwindef::{BOOL, DWORD};
    use winapi::shared::ntdef::HANDLE;
    use winapi::um::handleapi::{CloseHandle, SetHandleInformation};
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
    use winapi::um::winbase::HANDLE_FLAG_INHERIT;

    // CreatePipe (namedpipeapi is not vendored in this crate).
    unsafe extern "system" {
        fn CreatePipe(
            hReadPipe: *mut HANDLE,
            hWritePipe: *mut HANDLE,
            lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
            dwSize: DWORD,
        ) -> BOOL;
    }

    let root = tempfile::tempdir().expect("tempdir");

    // Self-pipe with an inheritable write end. Inherited handles preserve
    // their values across CreateProcessW, so if the whitelist failed, the
    // child could write into this pipe through the same value.
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    let mut sa: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
    sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    sa.bInheritHandle = 1;
    let ok = unsafe { CreatePipe(&mut read, &mut write, &mut sa as *mut _, 0) };
    assert_ne!(ok, 0, "CreatePipe must succeed");
    unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };
    let sentinel_value = write as isize;

    let (exe, argv) = sentinel_probe_fixture(sentinel_value.try_into().unwrap());
    let config = SubprocessConfig::default().allowed_executables(vec![exe.to_string()]);
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[SUBPROCESS_EXEC_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"exe": exe, "argv": argv})), &mut d)
            .await
            .expect("sentinel probe succeeds")
    };
    let stdout = stdout_of(&outcome.output);
    // The token must never surface anywhere visible: not in the captured
    // stdout (which would mean the child wrote through a handle that
    // inherited its value or aliased a surviving writable object), and not
    // in the sentinel pipe.
    assert!(
        !stdout.contains("SENTINEL_TOKEN"),
        "sentinel token leaked into captured output: {stdout}"
    );

    // The sentinel pipe itself must have received nothing: a token here is
    // proof the child really wrote through an inherited handle. The child
    // has already exited, so a blocking read terminates at EOF after the
    // parent closes its own write copy.
    unsafe {
        CloseHandle(write);
    };
    let mut buf = vec![0u8; 64];
    let mut read_n: u32 = 0;
    let _got = unsafe {
        winapi::um::fileapi::ReadFile(
            read,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut read_n,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(
        read_n,
        0,
        "no bytes may leak into the sentinel pipe; read_n={read_n}, got={_got}, bytes={:?}",
        &buf[..read_n.min(64) as usize]
    );
    unsafe {
        CloseHandle(read);
    }
}

/// Windows handle hygiene: the process handle count does not drift across
/// repeated invocations (each run's pipes, jobs, and stdio handles are
/// closed on success and failure paths).
#[cfg(windows)]
#[tokio::test]
async fn process_handle_count_stable_over_repeated_runs() {
    use winapi::um::processthreadsapi::{GetCurrentProcess, GetProcessHandleCount};

    let count_now = || {
        let mut n: u32 = 0;
        let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut n) };
        assert_ne!(ok, 0, "GetProcessHandleCount failed");
        n
    };

    let root = tempfile::tempdir().expect("tempdir");
    let config = SubprocessConfig::default();
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let run_once = |config: SubprocessConfig| {
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[SUBPROCESS_EXEC_CAPABILITY],
        );
        futures::executor::block_on(async {
            harness
                .run(
                    step(version_args()),
                    &mut dispatch(&harness, &m, &r, &mut runner),
                )
                .await
                .expect("run succeeds")
        })
    };

    // Warm-up so one-time caches do not count as drift.
    for _ in 0..3 {
        run_once(config.clone());
    }
    let before = count_now();
    for _ in 0..=20 {
        run_once(config.clone());
    }
    let after = count_now();
    assert!(
        (after as i64) - (before as i64) <= 2,
        "process handle count drifted: {before} -> {after}"
    );
}

/// R4 — fast exits observe pipe EOF on both streams and repeated runs do
/// not leak handles (each invocation closes its parent pipe write ends on
/// every path).
#[tokio::test]
async fn eof_observed_and_repeated_runs_do_not_leak() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = SubprocessConfig::default();
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    for i in 0..=20 {
        let mut runner = runner_for(
            config.clone(),
            root.path().to_path_buf(),
            &[SUBPROCESS_EXEC_CAPABILITY],
        );
        let outcome = {
            let mut d = dispatch(&harness, &m, &r, &mut runner);
            harness
                .run(step(version_args()), &mut d)
                .await
                .expect("run succeeds")
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&outcome.output).expect("output is JSON");
        assert_eq!(
            parsed["success"], true,
            "run {i} succeeded: {}",
            outcome.output
        );
    }
}
