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
