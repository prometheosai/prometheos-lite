//! Governed native harness (issue #154, slice F) — discriminating tests.
//!
//! The production [`NativeHarness`] performs one real, bounded,
//! read-only inspection through [`HarnessDispatch`] — a workspace file's
//! verified size and SHA-256 digest — inside a real `#171` git-worktree
//! workspace. These tests prove the happy path, every declared refusal, and
//! that the native harness registers alongside the production subprocess
//! harness as a second distinct real adapter.

use prometheos_lite::workflow::artifact_integrity::sha256_hex;
use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessOutcome, HarnessRegistry, HarnessStep,
    canonical_harness_declarations,
};
use prometheos_lite::workflow::native_harness::{
    NATIVE_HARNESS_ID, NATIVE_INSPECT_CAPABILITY, NATIVE_REVISION, NativeHarness,
    NativeInspectConfig, native_capability,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::subprocess_harness::{SUBPROCESS_HARNESS_ID, SubprocessHarness};
use prometheos_lite::workflow::workspace::{
    AdapterKind, GitWorktreeWorkspace, WORKSPACE_SCHEMA_VERSION, WorkspaceAdapter,
    WorkspaceManifestV1, WorkspaceMode,
};

mod common;

const NODE_ID: &str = "native-inspect-step";
const CANARY: &str = "test-secret-deep-41";

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed native inspection fixture",
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

/// A governed runner with the native capability declared under the execute
/// cap, bound to `workspace_root`. `permit_caps` controls the governance
/// grant; the registry always declares the real capability.
fn runner_for(
    config: NativeInspectConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        native_capability(config, workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn dispatch<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, vec![], runner)
}

fn dispatch_with_secrets<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
    secrets: Vec<String>,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, secrets, runner)
}

fn step(path: impl Into<String>) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: serde_json::json!({"path": path.into()}),
    }
}

/// A real `#171` worktree workspace over a throwaway git repo, plus a
/// governed fixture file committed into it. Returns (adapter, manifest,
/// acquired worktree).
fn workspace_fixture() -> (
    tempfile::TempDir,
    GitWorktreeWorkspace,
    WorkspaceManifestV1,
    prometheos_lite::workflow::workspace::AcquiredWorkspace,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "c1"]);
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let rev = String::from_utf8_lossy(&head.stdout).trim().to_string();

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };
    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.into(),
        workspace_id: "ws-native".into(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: prometheos_lite::workflow::workspace::ADAPTER_REVISION.into(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: rev,
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["work://x".into()],
        resource_lock_id: "lock-native".into(),
        created_at: "2026-08-25T00:00:00Z".into(),
        content_digest: None,
    }
    .sealed();
    let acquired = adapter.acquire(&manifest).expect("worktree acquired");
    (dir, adapter, manifest, acquired)
}

async fn run_inspect(
    harness: &dyn ExecutionHarness,
    step: HarnessStep,
    runner: &mut NodeRunner,
) -> anyhow::Result<HarnessOutcome> {
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(harness, &m, &r, runner);
    harness.run(step, &mut d).await
}

// ---------------------------------------------------------------------------
// Positive: real governed inspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inspection_through_dispatch_reports_digest_and_size() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let payload = b"deterministic native inspection payload\n";
    std::fs::write(acquired.root.join("hello.txt"), payload).unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let outcome = run_inspect(&harness, step("hello.txt"), &mut runner)
        .await
        .expect("inspection succeeds");
    assert_eq!(outcome.harness_id, NATIVE_HARNESS_ID);

    let parsed: serde_json::Value = serde_json::from_str(&outcome.output).expect("output is JSON");
    assert_eq!(parsed["path"], serde_json::json!("hello.txt"));
    assert_eq!(
        parsed["sha256"],
        serde_json::json!(sha256_hex(payload)),
        "SHA-256 must match the committed bytes"
    );
    assert_eq!(
        parsed["sizeBytes"],
        serde_json::json!(payload.len() as u64),
        "size must be the exact byte count"
    );
    assert_eq!(parsed["contentReturned"], serde_json::json!(false));

    // Evidence + event correlation.
    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.events[0].correlation_id, "step-1");
    assert_eq!(outcome.events[0].kind, "started");
    assert_eq!(outcome.events[1].kind, "completed");
    assert!(!outcome.evidence_entry_digest.is_empty());
    outcome.verify().expect("outcome verifies");

    // The workspace was never mutated: file bytes identical, no new files.
    assert_eq!(
        std::fs::read(acquired.root.join("hello.txt")).unwrap(),
        payload
    );
    let mut names: Vec<_> = std::fs::read_dir(&acquired.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            ".git".to_string(),
            "a.txt".to_string(),
            "hello.txt".to_string()
        ]
    );
}

#[tokio::test]
async fn inspection_is_deterministic() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    std::fs::write(acquired.root.join("same.txt"), "same bytes\n").unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let first = run_inspect(&harness, step("same.txt"), &mut runner)
        .await
        .expect("first run");
    let second = run_inspect(&harness, step("same.txt"), &mut runner)
        .await
        .expect("second run");
    assert_eq!(
        first.output, second.output,
        "identical inputs must produce identical results"
    );
}

/// File content never leaves the handler: a known secret inside the file
/// cannot surface in the evidence document, and the request-scoped secret
/// list reaches the handler through the governed dispatch.
#[tokio::test]
async fn known_secret_in_file_content_never_reaches_evidence() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let payload = format!("credential: {CANARY}\n");
    std::fs::write(acquired.root.join("secret.txt"), &payload).unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let m = manifest();
    let r = restrictions();
    let outcome = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![CANARY.to_string()]);
        harness
            .run(step("secret.txt"), &mut d)
            .await
            .expect("inspection succeeds")
    };
    assert!(
        !outcome.output.contains(CANARY),
        "evidence must never contain the secret value"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("output stays parseable JSON");
    assert_eq!(
        parsed["sha256"],
        serde_json::json!(sha256_hex(payload.as_bytes()))
    );
}

// ---------------------------------------------------------------------------
// Negative: authority and binding refusals
// ---------------------------------------------------------------------------

/// A capability outside the harness declaration is refused before the
/// runner is reached: no governed call is issued.
#[tokio::test]
async fn undeclared_capability_refuses_before_runner() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let bogus = HarnessStep {
        step_id: "step-1".to_string(),
        capability: "harness.native.bogus".to_string(),
        args: serde_json::json!({"path": "hello.txt"}),
    };
    let err = harness.run(bogus, &mut d).await.unwrap_err().to_string();
    assert!(
        err.contains("outside its declaration"),
        "undeclared capability must refuse at dispatch, got: {err}"
    );
}

/// The subprocess capability cannot be smuggled through the native
/// adapter, and the native capability cannot be smuggled through the
/// subprocess adapter: read scope never becomes write/exec authority.
#[tokio::test]
async fn cross_capability_smuggling_refuses() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let evil = HarnessStep {
        step_id: "step-1".to_string(),
        capability: "harness.subprocess.exec".to_string(),
        args: serde_json::json!({"exe": "cargo", "argv": ["--version"]}),
    };
    assert!(harness.run(evil, &mut d).await.is_err());
}

/// Without a governance grant for the capability, the NodeRunner refuses
/// before the handler executes (no result, no digest).
#[tokio::test]
async fn missing_capability_authorization_refuses_before_execution() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    std::fs::write(acquired.root.join("hello.txt"), "x\n").unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[], // permit grants NO capabilities
    );
    let harness = NativeHarness::new();
    let err = run_inspect(&harness, step("hello.txt"), &mut runner)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("SOMA-AUTH-0001") && err.contains("does not grant"),
        "ungranted capability must be refused by the runner, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Negative: path and resource refusals
// ---------------------------------------------------------------------------

async fn refused_step(path: &str, config: NativeInspectConfig) -> String {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let _ = std::fs::create_dir_all(acquired.root.join("subdir"));
    std::fs::write(acquired.root.join("hello.txt"), "x\n").unwrap();
    let mut runner = runner_for(config, acquired.root.clone(), &[NATIVE_INSPECT_CAPABILITY]);
    let harness = NativeHarness::new();
    run_inspect(&harness, step(path.to_string()), &mut runner)
        .await
        .unwrap_err()
        .to_string()
}

#[tokio::test]
async fn traversal_path_refuses() {
    let err = refused_step("../outside.txt", NativeInspectConfig::default()).await;
    assert!(err.contains("escapes the workspace"), "got: {err}");
}

#[tokio::test]
async fn absolute_path_refuses() {
    let err = refused_step("/etc/passwd", NativeInspectConfig::default()).await;
    assert!(err.contains("must be relative"), "got: {err}");
}

#[tokio::test]
async fn missing_file_refuses() {
    let err = refused_step("nope.txt", NativeInspectConfig::default()).await;
    assert!(err.contains("does not exist"), "got: {err}");
}

#[tokio::test]
async fn directory_input_refuses() {
    let err = refused_step("subdir", NativeInspectConfig::default()).await;
    assert!(err.contains("not a regular file"), "got: {err}");
}

#[tokio::test]
async fn file_size_budget_refuses() {
    let config = NativeInspectConfig::default().max_file_bytes(1);
    let err = refused_step("hello.txt", config).await;
    assert!(err.contains("size budget"), "got: {err}");
}

#[tokio::test]
async fn output_budget_refuses() {
    let config = NativeInspectConfig::default().max_output_bytes(16);
    let err = refused_step("hello.txt", config).await;
    assert!(err.contains("output budget"), "got: {err}");
}

/// A symlink inside the workspace pointing outside of it is refused: the
/// canonicalized target must stay inside the workspace root.
#[tokio::test]
async fn symlink_escape_refuses() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    let outside = _dir.path().join("outside.txt");
    std::fs::write(&outside, "outside secret\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, acquired.root.join("link.txt")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside, acquired.root.join("link.txt")).unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let err = run_inspect(&harness, step("link.txt"), &mut runner)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("escapes the workspace"), "got: {err}");
    // The outside file was not read into evidence.
    assert!(!err.contains("outside secret"));
}

/// Unsupported argument keys are refused by the handler.
#[tokio::test]
async fn unsupported_argument_refuses() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    std::fs::write(acquired.root.join("hello.txt"), "x\n").unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let bad = HarnessStep {
        step_id: "step-1".to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: serde_json::json!({"path": "hello.txt", "write": true}),
    };
    let err = harness.run(bad, &mut d).await.unwrap_err().to_string();
    assert!(err.contains("unsupported argument"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Checkpoint / resume (portable, stateless)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checkpoint_and_resume_bind_portable_state_only() {
    let (_dir, _adapter, _manifest, acquired) = workspace_fixture();
    std::fs::write(acquired.root.join("hello.txt"), "x\n").unwrap();
    let mut runner = runner_for(
        NativeInspectConfig::default(),
        acquired.root.clone(),
        &[NATIVE_INSPECT_CAPABILITY],
    );
    let harness = NativeHarness::new();
    let outcome = run_inspect(&harness, step("hello.txt"), &mut runner)
        .await
        .expect("inspection succeeds");
    let digest = sha256_hex(b"portable-state");
    let checkpoint = harness
        .checkpoint(&outcome, &digest)
        .await
        .expect("checkpoint exports");
    checkpoint.verify().expect("checkpoint verifies");
    assert_eq!(checkpoint.harness_id, NATIVE_HARNESS_ID);
    assert_eq!(checkpoint.harness_revision, NATIVE_REVISION);

    // Same digest resumes; a different portable state refuses.
    harness
        .resume(&checkpoint, &digest)
        .await
        .expect("resume with matching state");
    assert!(harness.resume(&checkpoint, &"0".repeat(64)).await.is_err());

    // A checkpoint from another harness cannot resume here.
    let mut forged = checkpoint.clone();
    forged.harness_id = "lite.harness.subprocess".to_string();
    let forged = forged.sealed();
    assert!(harness.resume(&forged, &digest).await.is_err());
}

// ---------------------------------------------------------------------------
// Registry + catalog: two real production adapters
// ---------------------------------------------------------------------------

fn production_registry() -> HarnessRegistry {
    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(NativeHarness::new()))
        .expect("native adapter registers");
    registry
        .register(Box::new(SubprocessHarness::new()))
        .expect("subprocess adapter registers");
    registry
}

#[test]
fn production_registry_holds_two_real_adapters() {
    let registry = production_registry();
    let catalog = registry.catalog();
    let ids: Vec<&str> = catalog
        .entries
        .iter()
        .map(|e| e.harness_id.as_str())
        .collect();
    assert_eq!(ids, vec![NATIVE_HARNESS_ID, SUBPROCESS_HARNESS_ID]);
    let native = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == NATIVE_HARNESS_ID)
        .unwrap();
    assert_eq!(
        native.family,
        prometheos_lite::workflow::execution_harness::HarnessFamily::Native
    );
    assert_eq!(
        native.claimed_capabilities,
        vec![NATIVE_INSPECT_CAPABILITY.to_string()]
    );
    let sub = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == SUBPROCESS_HARNESS_ID)
        .unwrap();
    assert_eq!(
        sub.family,
        prometheos_lite::workflow::execution_harness::HarnessFamily::Subprocess
    );
    // The catalog entries match their implementation-owned declarations.
    let decls = canonical_harness_declarations();
    assert!(decls.iter().any(|d| d.harness_id == NATIVE_HARNESS_ID));
    assert!(decls.iter().any(|d| d.harness_id == SUBPROCESS_HARNESS_ID));
    let prod: Vec<_> = decls
        .into_iter()
        .filter(|d| d.harness_id == NATIVE_HARNESS_ID || d.harness_id == SUBPROCESS_HARNESS_ID)
        .collect();
    let findings = catalog.verify_against_declarations(&prod);
    assert!(
        findings.is_empty(),
        "catalog must match declarations: {findings:?}"
    );
}

/// Both real adapters stay independently selectable; the native adapter's
/// declaration governs the governed boundary via the same registry API.
#[tokio::test]
async fn native_and_subprocess_registries_are_independent() {
    let mut only_native = HarnessRegistry::new();
    only_native
        .register(Box::new(NativeHarness::new()))
        .unwrap();
    assert!(only_native.get(NATIVE_HARNESS_ID).is_ok());
    assert!(only_native.get(SUBPROCESS_HARNESS_ID).is_err());

    let mut only_sub = HarnessRegistry::new();
    only_sub
        .register(Box::new(SubprocessHarness::new()))
        .unwrap();
    assert!(only_sub.get(SUBPROCESS_HARNESS_ID).is_ok());
    assert!(only_sub.get(NATIVE_HARNESS_ID).is_err());
}

// ---------------------------------------------------------------------------
// Selection: workspace adapter revision compatibility (before effects)
// ---------------------------------------------------------------------------

fn selection_requirements() -> prometheos_lite::workflow::soma::capability::WorkRequirements {
    serde_json::from_value(serde_json::json!({
        "id": "reqs-native",
        "schemaVersion": prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
    }))
    .expect("requirements parse")
}

fn selection_authority() -> prometheos_lite::workflow::soma::contracts::AuthorityProfile {
    serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
        .expect("authority parses")
}

fn selection_workspace_manifest(
    adapter_revision: &str,
) -> prometheos_lite::workflow::workspace::WorkspaceManifestV1 {
    prometheos_lite::workflow::workspace::WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "ws-native-select".into(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: adapter_revision.into(),
        repo_identity: "test-repo".into(),
        base_revision: "a".repeat(40),
        branch: None,
        mode: WorkspaceMode::ReadOnly,
        writable_scopes: vec![],
        resource_lock_id: "lock-native-select".into(),
        created_at: "2026-08-25T00:00:00Z".into(),
        content_digest: None,
    }
    .sealed()
}

#[test]
fn selection_selects_native_for_compatible_workspace_adapter_revision() {
    let (_dir, _adapter, _manifest, _acquired) = workspace_fixture();
    let registry = production_registry();
    let requirements = selection_requirements();
    let authority = selection_authority();
    let workspace =
        selection_workspace_manifest(prometheos_lite::workflow::workspace::ADAPTER_REVISION);
    let selection = registry
        .select(
            &prometheos_lite::workflow::execution_harness::HarnessSelectionRequest {
                selection_id: "sel-native".to_string(),
                requirements: &requirements,
                authority_declared: &authority,
                authority_effective: &authority,
                evaluated_at: "2026-11-08T00:00:00Z".to_string(),
                substitutions: Vec::new(),
                // Two production adapters are both compatible with this
                // workspace; selection requires an explicit pin (no silent
                // substitution). We pin the native adapter.
                pinned_harness_id: Some(NATIVE_HARNESS_ID),
                workspace: Some(&workspace),
            },
        )
        .expect("compatible workspace must select a harness");
    assert_eq!(
        selection.decision_status,
        prometheos_lite::workflow::soma::capability::DecisionStatus::Compatible
    );
    selection.verify().expect("selection evidence verifies");
}

#[test]
fn selection_refuses_incompatible_workspace_adapter_revision() {
    let (_dir, _adapter, _manifest, _acquired) = workspace_fixture();
    let registry = production_registry();
    let requirements = selection_requirements();
    let authority = selection_authority();
    let workspace = selection_workspace_manifest("lite.workspace.adapter.v999");
    let err = registry
        .select(
            &prometheos_lite::workflow::execution_harness::HarnessSelectionRequest {
                selection_id: "sel-native-bad".to_string(),
                requirements: &requirements,
                authority_declared: &authority,
                authority_effective: &authority,
                evaluated_at: "2026-11-08T00:00:00Z".to_string(),
                substitutions: Vec::new(),
                pinned_harness_id: None,
                workspace: Some(&workspace),
            },
        )
        .expect_err("incompatible adapter revision must refuse before effects");
    assert!(
        err.to_string().contains("HSEL-"),
        "expected a selection refusal code, got: {err}"
    );
}
