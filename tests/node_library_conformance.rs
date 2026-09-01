//! Conformance + acceptance tests for the E5/I01 node library (#125).
//!
//! Drives the `intake`, `repo-discovery`, and `planning` capabilities through
//! the generic nine-gate `NodeRunner` (lite.node.v1 contracts, lite.policy.v1
//! authorization, redaction, journal durability) — the same machinery the node
//! conformance kit exercises. These tests prove: intake fails closed on
//! ambiguous / unauthorized scope; discovery records files/languages/tests/
//! constraints as evidence; planning is typed and linked to discovery
//! evidence; none of the nodes may mutate the target repository; and the
//! conformance categories (policy, bypass, evidence durability) hold.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use serde_json::json;

use prometheos_lite::workflow::node_contracts::{NodeManifestV1, OutcomeKind};
use prometheos_lite::workflow::node_library::{
    CAP_DISCOVERY, CAP_INTAKE, CAP_PLANNING, DiscoveryResultV1, IntakeTaskManifestV1, ScopedPlanV1,
    intake_discovery_planning_registry, node_manifest,
};
use prometheos_lite::workflow::node_runner::{NodeRunOutcome, NodeRunRequest, NodeRunner};
use prometheos_lite::workflow::node_validation::{
    CAP_DIAGNOSTIC, CAP_TEST_DISCOVERY, CAP_VALIDATION, CommandRunV1, DiagnosticReportV1,
    FailureKindV1, TestDiscoveryResultV1, ValidationResultV1, diagnostic_node_manifest,
    diagnostic_registry, node_manifest as validation_node_manifest, test_discovery_registry,
    validation_node_manifest as validation_pipeline_manifest, validation_registry,
};
use prometheos_lite::workflow::policy::LocalRestrictions;

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://fixture".to_string()],
        writable_scopes: vec![],
        denied_providers: vec![],
        forbidden_paths: vec![],
        token_budget_ceiling: None,
        max_attempts: 3,
        escalation_target: "local".to_string(),
    }
}

fn runner() -> NodeRunner {
    NodeRunner::new(intake_discovery_planning_registry())
}

/// Execute a node through the nine-gate pipeline (runner is mutable for the
/// idempotency cache).
fn run_node(
    runner: &mut NodeRunner,
    manifest: &NodeManifestV1,
    caps: &LocalRestrictions,
    capability: &str,
    args: serde_json::Value,
    key: &str,
) -> Result<NodeRunOutcome> {
    runner.execute(NodeRunRequest {
        manifest,
        local_restrictions: caps,
        capability: capability.to_string(),
        args,
        idempotency_key: key.to_string(),
        known_secrets: vec![],
    })
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git available in test environment");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_output(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git available in test environment");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Build a small git repository to discover: two source files + one test file.
fn fixture_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "ci@example.com"]);
    git(root, &["config", "user.name", "ci"]);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: u32, b: u32) -> u32 { a + b }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib_test.rs"),
        "#[test] fn t() { assert_eq!(add(1, 2), 3); }\n",
    )
    .unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
    dir
}

// ---------------------------------------------------------------------------
// Intake acceptance
// ---------------------------------------------------------------------------

#[test]
fn intake_rejects_ambiguous_objective_safely() {
    let mut runner = runner();
    let caps = restrictions();
    let m = node_manifest("node-intake", CAP_INTAKE);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_INTAKE,
        json!({"objective": "make it better"}),
        "intake-amb",
    );
    assert!(
        res.is_err(),
        "ambiguous objective must be rejected, got {res:?}"
    );
}

#[test]
fn intake_rejects_unauthorized_scope_safely() {
    let mut runner = runner();
    let caps = restrictions();
    let m = node_manifest("node-intake", CAP_INTAKE);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_INTAKE,
        json!({"objective": "refactor /etc/passwd"}),
        "intake-unauth",
    );
    assert!(
        res.is_err(),
        "absolute-path scope must be rejected, got {res:?}"
    );
}

#[test]
fn intake_accepts_concrete_objective() {
    let mut runner = runner();
    let caps = restrictions();
    let m = node_manifest("node-intake", CAP_INTAKE);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_INTAKE,
        json!({"objective": "implement retry backoff for the http client"}),
        "intake-ok",
    )
    .expect("concrete objective accepted");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);
    let manifest: IntakeTaskManifestV1 = serde_json::from_str(&outcome.output).unwrap();
    assert!(manifest.authorized);
    assert!(manifest.task_id.starts_with("task-"));
}

// ---------------------------------------------------------------------------
// Discovery acceptance
// ---------------------------------------------------------------------------

#[test]
fn discovery_records_files_languages_tests_constraints() {
    let dir = fixture_repo();
    let mut runner = runner();
    let caps = restrictions();
    let m = node_manifest("node-discovery", CAP_DISCOVERY);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_DISCOVERY,
        json!({"repoRoot": dir.path().to_str().unwrap()}),
        "discovery-1",
    )
    .expect("discovery completes");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let d: DiscoveryResultV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(d.file_count, 3);
    assert!(d.languages.iter().any(|l| l == "rust"));
    assert!(d.test_files.iter().any(|p| p.contains("lib_test.rs")));
    assert!(!d.constraints.is_empty());
    assert!(!d.fact_batch_digest.is_empty());
    assert!(
        !outcome.result.evidence_refs.is_empty(),
        "discovery must emit evidence"
    );
    assert!(
        outcome
            .result
            .evidence_refs
            .iter()
            .any(|e| e.artifact_kind.contains("node-output")),
        "discovery emits a node-output evidence ref (read-only)"
    );
}

// ---------------------------------------------------------------------------
// Planning acceptance (typed + linked to discovery evidence)
// ---------------------------------------------------------------------------

#[test]
fn planning_is_typed_and_linked_to_discovery() {
    let dir = fixture_repo();
    let mut runner = runner();
    let caps = restrictions();

    let dm = node_manifest("node-discovery", CAP_DISCOVERY);
    let discovery = run_node(
        &mut runner,
        &dm,
        &caps,
        CAP_DISCOVERY,
        json!({"repoRoot": dir.path().to_str().unwrap()}),
        "plan-disc",
    )
    .expect("discovery completes");
    let d: DiscoveryResultV1 = serde_json::from_str(&discovery.output).unwrap();

    let pm = node_manifest("node-planning", CAP_PLANNING);
    let plan = run_node(
        &mut runner,
        &pm,
        &caps,
        CAP_PLANNING,
        json!({"objective": "implement retry backoff", "discoveryEvidence": discovery.output}),
        "plan-1",
    )
    .expect("planning links discovery");
    assert_eq!(plan.result.outcome, OutcomeKind::Completed);

    let p: ScopedPlanV1 = serde_json::from_str(&plan.output).unwrap();
    assert_eq!(
        p.discovery_revision, d.revision,
        "plan must cite discovery revision"
    );
    assert_eq!(
        p.discovery_evidence_id, d.fact_batch_digest,
        "plan must link discovery evidence digest"
    );
    assert!(!p.steps.is_empty());
    assert!(
        p.steps
            .iter()
            .all(|s| s.evidence_ref == d.fact_batch_digest)
    );
}

// ---------------------------------------------------------------------------
// Conformance categories
// ---------------------------------------------------------------------------

#[test]
fn policy_denies_write_outside_granted_writable_scope() {
    // Local policy grants no writable scope; a manifest that claims a writable
    // scope must be denied before any effect runs (lite.policy.v1 gate 3).
    let deny = LocalRestrictions {
        readable_scopes: vec!["repo://fixture".to_string()],
        writable_scopes: vec![],
        denied_providers: vec![],
        forbidden_paths: vec![],
        token_budget_ceiling: None,
        max_attempts: 3,
        escalation_target: "local".to_string(),
    };
    let unauthorized_write = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": "node-intake-write",
            "purpose": "intake",
            "inputs": [{"name": "objective", "typeRef": "core.String", "required": true}],
            "outputs": [{"name": "task", "typeRef": "lite.node.intake.TaskManifest", "required": true}],
            "readableScopes": ["repo://fixture"],
            "writableScopes": ["repo://fixture"],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .unwrap();
    let mut runner = runner();
    let res = run_node(
        &mut runner,
        &unauthorized_write,
        &deny,
        CAP_INTAKE,
        json!({"objective": "implement retry backoff for the http client"}),
        "policy-deny",
    );
    assert!(
        res.is_err(),
        "write outside granted writable scope must be denied"
    );
}

#[test]
fn governed_path_bypass_is_blocked_for_undeclared_capability() {
    let unauth = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": "evil",
            "purpose": "not-a-registered-capability",
            "inputs": [{"name": "objective", "typeRef": "core.String", "required": true}],
            "outputs": [{"name": "out", "typeRef": "core.String", "required": true}],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .unwrap();
    let mut runner = runner();
    let caps = restrictions();
    let res = run_node(
        &mut runner,
        &unauth,
        &caps,
        "not-a-registered-capability",
        json!({"objective": "implement x"}),
        "bypass",
    );
    assert!(res.is_err(), "undeclared capability must not run");
}

// ---------------------------------------------------------------------------
// E5/I03 (issue #127) — test discovery
// ---------------------------------------------------------------------------

fn validation_runner() -> NodeRunner {
    NodeRunner::new(test_discovery_registry())
}

/// Build a small git repository with the named manifest files at the root.
fn fixture_repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "ci@example.com"]);
    git(root, &["config", "user.name", "ci"]);
    for (path, contents) in files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, contents).unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
    dir
}

#[test]
fn test_discovery_records_why_for_each_command() {
    // Issue #127 acceptance criterion 1: test discovery records why each
    // command was selected.
    let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
    let mut runner = validation_runner();
    let caps = restrictions();
    let m = validation_node_manifest("node-test-discovery");
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_TEST_DISCOVERY,
        json!({"repoRoot": dir.path().to_str().unwrap()}),
        "disc-why",
    )
    .expect("test-discovery completes");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let d: TestDiscoveryResultV1 = serde_json::from_str(&outcome.output).unwrap();
    // Cargo rule + meta-assertion.
    assert!(d.commands.len() >= 2);
    for c in &d.commands {
        assert!(
            !c.why.is_empty(),
            "every discovered command must carry a non-empty `why` (got {:?})",
            c
        );
        assert!(
            !c.source.is_empty(),
            "every discovered command must carry a non-empty `source` (got {:?})",
            c
        );
    }
    // The cargo rule's `why` must reference the manifest that triggered it.
    let cargo = d
        .commands
        .iter()
        .find(|c| c.command == "cargo")
        .expect("cargo command present");
    assert!(
        cargo.why.contains("Cargo.toml"),
        "cargo rule's `why` must cite the manifest: {}",
        cargo.why
    );
}

#[test]
fn test_discovery_emits_deterministic_evidence_refs() {
    // Each command's `evidence_ref` is a 64-char lowercase hex digest of
    // `{command, args, why, source}` and is stable across runs.
    let dir = fixture_repo_with(&[
        ("Cargo.toml", "[package]\nname = \"x\"\n"),
        ("package.json", r#"{"name":"x","scripts":{"test":"true"}}"#),
    ]);
    let mut runner = validation_runner();
    let caps = restrictions();
    let m = validation_node_manifest("node-test-discovery");

    let a = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_TEST_DISCOVERY,
        json!({"repoRoot": dir.path().to_str().unwrap()}),
        "disc-det-a",
    )
    .expect("first run");
    let b = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_TEST_DISCOVERY,
        json!({"repoRoot": dir.path().to_str().unwrap()}),
        "disc-det-b",
    )
    .expect("second run");

    let pa: TestDiscoveryResultV1 = serde_json::from_str(&a.output).unwrap();
    let pb: TestDiscoveryResultV1 = serde_json::from_str(&b.output).unwrap();
    assert_eq!(pa.discovery_digest, pb.discovery_digest);
    for (ca, cb) in pa.commands.iter().zip(pb.commands.iter()) {
        assert_eq!(ca.evidence_ref, cb.evidence_ref);
        assert_eq!(ca.evidence_ref.len(), 64);
        assert!(ca.evidence_ref.chars().all(|x| x.is_ascii_hexdigit()));
    }
}

// ---------------------------------------------------------------------------
// E5/I03 (issue #127) — validation
// ---------------------------------------------------------------------------

fn validation_pipeline_runner() -> NodeRunner {
    NodeRunner::new(validation_registry())
}

#[test]
fn validation_never_mutates_source_repository() {
    // Issue #127 acceptance criterion 2: Validation never mutates the
    // original repository. The validation node runs commands inside an
    // isolated GitWorktreeWorkspace; the source checkout's HEAD and
    // working-tree status must be identical before and after the run.
    let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
    let head_before = git_output(dir.path(), &["rev-parse", "HEAD"]);
    let status_before = git_output(dir.path(), &["status", "--porcelain"]);
    assert!(status_before.is_empty(), "fixture must start clean");

    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = validation_pipeline_runner();
    let caps = restrictions();
    let m = validation_pipeline_manifest("node-validation-isolated");

    // Use a cross-platform, deterministic command. `git rev-parse HEAD`
    // proves the worktree is pinned to baseRevision and produces a
    // non-empty stdout we can compare; we deliberately do NOT depend on
    // `cargo` or any other toolchain.
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_VALIDATION,
        json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head_before,
            "commands": [
                {"command": "git", "args": ["rev-parse", "HEAD"]},
            ],
        }),
        "val-isolated",
    )
    .expect("validation completes against the worktree");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let v: ValidationResultV1 = serde_json::from_str(&outcome.output).unwrap();
    // Worktree head equals the pinned base revision (we never committed).
    assert_eq!(v.worktree_head_revision, head_before);
    // One run was executed.
    assert_eq!(v.runs.len(), 1);
    let r = &v.runs[0];
    assert_eq!(r.command, "git");
    assert_eq!(r.exit_code, Some(0));
    assert!(!r.timed_out);
    assert_eq!(r.evidence_ref.len(), 64);
    // The git command's stdout_tail is the worktree head, which equals
    // the source HEAD (same commit) — but the SOURCE repository is
    // unchanged.
    let head_via_worktree = r.stdout_tail.trim().to_string();
    assert_eq!(head_via_worktree, head_before);

    // Source repository invariants.
    let head_after = git_output(dir.path(), &["rev-parse", "HEAD"]);
    let status_after = git_output(dir.path(), &["status", "--porcelain"]);
    assert_eq!(
        head_before, head_after,
        "source HEAD must not move during validation"
    );
    assert_eq!(
        status_before, status_after,
        "source working tree must be untouched"
    );
    assert!(
        status_after.is_empty(),
        "source working tree must remain clean"
    );
}

#[test]
fn validation_records_exit_code_and_evidence_for_failing_command() {
    // Failing commands are recorded honestly: non-zero exit code, captured
    // stderr, and a per-run evidence_ref that ties the run to its output.
    let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
    let head = git_output(dir.path(), &["rev-parse", "HEAD"]);
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = validation_pipeline_runner();
    let caps = restrictions();
    let m = validation_pipeline_manifest("node-validation-fail");
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_VALIDATION,
        json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [
                {"command": "git", "args": ["log", "--not-a-real-flag"]},
            ],
        }),
        "val-fail",
    )
    .expect("validation completes even when the command fails");
    let v: ValidationResultV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(v.runs.len(), 1);
    let r = &v.runs[0];
    assert_eq!(r.command, "git");
    // git exits 129 on unknown flag; what matters is non-zero + evidence
    // capture, not the precise code (which can vary by git version).
    assert!(matches!(r.exit_code, Some(c) if c != 0));
    assert!(!r.timed_out);
    assert!(!r.stderr_tail.is_empty(), "stderr must be captured");
    assert_eq!(r.evidence_ref.len(), 64);
    // The overall result still Completed — the node records failures; it
    // does not abort on a non-zero command.
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);
}

// ---------------------------------------------------------------------------
// E5/I03 (issue #127) — diagnostic
// ---------------------------------------------------------------------------

fn diagnostic_pipeline_runner() -> NodeRunner {
    NodeRunner::new(diagnostic_registry())
}

#[test]
fn diagnostic_distinguishes_code_test_environment_timeout_and_resource() {
    // Issue #127 acceptance criterion 3: "Code, test, environment, timeout,
    // and resource failures are distinguished." Feed the diagnostic node
    // a synthetic validation result with one run per failure kind and
    // assert each is correctly classified.
    let mut runs = Vec::new();
    let make = |stderr: &str, exit: Option<i32>, timed_out: bool, command: &str| CommandRunV1 {
        command: command.to_string(),
        args: vec![],
        exit_code: exit,
        duration_ms: 0,
        timed_out,
        stdout_tail: String::new(),
        stderr_tail: stderr.to_string(),
        evidence_ref: String::new(),
    };
    runs.push(make(
        "error[E0425]: cannot find value `x` in this scope",
        Some(1),
        false,
        "cargo",
    ));
    runs.push(make(
        "thread 't' panicked at 'assertion failed', src/lib.rs:1",
        Some(101),
        false,
        "cargo-test",
    ));
    runs.push(make(
        "sh: tool: command not found",
        Some(127),
        false,
        "missing",
    ));
    runs.push(make(
        "fatal error: out of memory (os error 1455)",
        Some(1),
        false,
        "rustc",
    ));
    runs.push(make("", None, true, "hung"));
    let validation = ValidationResultV1 {
        schema_version: "1.0.0".to_string(),
        repo_root: ".".to_string(),
        base_revision: "abc".to_string(),
        worktree_head_revision: "abc".to_string(),
        run_id: "v-fixture".to_string(),
        runs_digest: "d".repeat(64),
        runs,
        constraints: vec![],
    };
    let mut runner = diagnostic_pipeline_runner();
    let caps = restrictions();
    let m = diagnostic_node_manifest("node-diagnostic");
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_DIAGNOSTIC,
        json!({
            "repoRoot": ".",
            "validationEvidence": serde_json::to_string(&validation).unwrap(),
        }),
        "diag-distinguish",
    )
    .expect("diagnostic completes");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let report: DiagnosticReportV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(report.classifications.len(), 5);
    assert_eq!(report.total_runs, 5);
    assert_eq!(report.failed_runs, 5);
    let by_kind: std::collections::HashMap<FailureKindV1, usize> = report
        .summary_by_kind
        .iter()
        .map(|c| (c.kind, c.count))
        .collect();
    assert_eq!(by_kind.get(&FailureKindV1::Code), Some(&1));
    assert_eq!(by_kind.get(&FailureKindV1::Test), Some(&1));
    assert_eq!(by_kind.get(&FailureKindV1::Environment), Some(&1));
    assert_eq!(by_kind.get(&FailureKindV1::Resource), Some(&1));
    assert_eq!(by_kind.get(&FailureKindV1::Timeout), Some(&1));

    // Per-run kind matches the input we fed in (in order).
    let kinds: Vec<FailureKindV1> = report.classifications.iter().map(|c| c.kind).collect();
    assert_eq!(kinds[0], FailureKindV1::Code);
    assert_eq!(kinds[1], FailureKindV1::Test);
    assert_eq!(kinds[2], FailureKindV1::Environment);
    assert_eq!(kinds[3], FailureKindV1::Resource);
    assert_eq!(kinds[4], FailureKindV1::Timeout);

    // Each classification's `signals` cites the substring(s) that
    // triggered the rule.
    for c in &report.classifications {
        assert!(
            !c.signals.is_empty(),
            "kind {:?} must carry at least one signal",
            c.kind
        );
    }
}

#[test]
fn diagnostic_emits_evidence_backed_classifications() {
    // Issue #127 acceptance criterion 4: "Diagnostic node emits
    // evidence-backed classifications." Every classification must carry
    // a 64-char lowercase hex evidence_ref that ties the classification
    // to its inputs (command, exit code, captured output, kind, signals).
    let validation = ValidationResultV1 {
        schema_version: "1.0.0".to_string(),
        repo_root: ".".to_string(),
        base_revision: "abc".to_string(),
        worktree_head_revision: "abc".to_string(),
        run_id: "v-ev".to_string(),
        runs_digest: "d".repeat(64),
        runs: vec![CommandRunV1 {
            command: "cargo".to_string(),
            args: vec!["test".to_string()],
            exit_code: Some(101),
            duration_ms: 0,
            timed_out: false,
            stdout_tail: String::new(),
            stderr_tail: "panicked at 'assertion failed'".to_string(),
            evidence_ref: String::new(),
        }],
        constraints: vec![],
    };
    let mut runner = diagnostic_pipeline_runner();
    let caps = restrictions();
    let m = diagnostic_node_manifest("node-diagnostic-ev");
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_DIAGNOSTIC,
        json!({
            "repoRoot": ".",
            "validationEvidence": serde_json::to_string(&validation).unwrap(),
        }),
        "diag-ev",
    )
    .expect("diagnostic completes");
    let report: DiagnosticReportV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(report.classifications.len(), 1);
    let c = &report.classifications[0];
    assert_eq!(c.evidence_ref.len(), 64);
    assert!(c.evidence_ref.chars().all(|x| x.is_ascii_hexdigit()));
    // Determinism: running the same input again produces the same
    // evidence_ref.
    let outcome2 = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_DIAGNOSTIC,
        json!({
            "repoRoot": ".",
            "validationEvidence": serde_json::to_string(&validation).unwrap(),
        }),
        "diag-ev",
    )
    .expect("diagnostic completes (second run)");
    let report2: DiagnosticReportV1 = serde_json::from_str(&outcome2.output).unwrap();
    assert_eq!(
        report.classifications[0].evidence_ref,
        report2.classifications[0].evidence_ref
    );
}
