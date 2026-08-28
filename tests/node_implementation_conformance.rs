//! Conformance + acceptance tests for the E5/I02 implementation/repair nodes (#126).
//!
//! Drives the `implement` and `repair` capabilities through the generic
//! nine-gate `NodeRunner` (lite.node.v1 + lite.policy.v1 + journal durability).
//! These tests prove: the executor nodes acquire a `GitWorktreeWorkspace`
//! pinned to the cited base revision, record durable change artifacts, commit
//! in the isolated worktree (so the source checkout is never touched), link
//! the result back to the planning / diagnosis evidence, and are denied when
//! the local policy does not grant their declared writable scope.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use serde_json::json;

use prometheos_lite::workflow::node_contracts::{NodeManifestV1, OutcomeKind};
use prometheos_lite::workflow::node_implementation::{
    CAP_IMPLEMENT, CAP_REPAIR, DiagnosisV1, ImplementationResultV1, RepairResultV1,
    implementation_repair_registry, node_manifest,
};
use prometheos_lite::workflow::node_library::ScopedPlanV1;
use prometheos_lite::workflow::node_runner::{NodeRunOutcome, NodeRunRequest, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;

fn restrictions_with_write() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://fixture".to_string()],
        writable_scopes: vec!["repo://fixture".to_string()],
        denied_providers: vec![],
        forbidden_paths: vec![],
        token_budget_ceiling: None,
        max_attempts: 1,
        escalation_target: "local".to_string(),
    }
}

fn restrictions_readonly() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://fixture".to_string()],
        writable_scopes: vec![],
        denied_providers: vec![],
        forbidden_paths: vec![],
        token_budget_ceiling: None,
        max_attempts: 1,
        escalation_target: "local".to_string(),
    }
}

fn runner() -> NodeRunner {
    NodeRunner::new(implementation_repair_registry())
}

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

fn build_plan(plan_id: &str, base_revision: &str, evidence: &str, steps: usize) -> ScopedPlanV1 {
    ScopedPlanV1 {
        schema_version: "1.0.0".to_string(),
        plan_id: plan_id.to_string(),
        objective: "implement retry backoff for the http client".to_string(),
        discovery_revision: base_revision.to_string(),
        discovery_evidence_id: evidence.to_string(),
        steps: (1..=steps)
            .map(|i| prometheos_lite::workflow::node_library::PlanStepV1 {
                step: i as u32,
                title: format!("step {i}"),
                targets: vec!["src/main.rs".to_string()],
                evidence_ref: evidence.to_string(),
            })
            .collect(),
        assumptions: vec![],
    }
}

// ---------------------------------------------------------------------------
// Implement acceptance
// ---------------------------------------------------------------------------

#[test]
fn implement_acquires_worktree_commits_and_links_plan() {
    let dir = fixture_repo();
    let root = dir.path();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let plan = build_plan("plan-impl-1", &base_revision, "evidence-digest-1", 3);
    let plan_json = serde_json::to_string(&plan).unwrap();
    let ws_parent = tempfile::tempdir().unwrap();
    let ws_parent_str = ws_parent.path().to_str().unwrap().to_string();

    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl", CAP_IMPLEMENT);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": plan_json,
            "repoRoot": root.to_str().unwrap(),
            "workspaceParent": ws_parent_str,
        }),
        "impl-1",
    )
    .expect("implement completes under granted write scope");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let r: ImplementationResultV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(r.plan_id, "plan-impl-1");
    assert_eq!(r.discovery_evidence_id, "evidence-digest-1");
    assert_ne!(
        r.revision, base_revision,
        "commit must produce a new revision"
    );
    assert!(!r.changed_files.is_empty(), "must record changed files");
    assert_eq!(r.changes.len(), 3, "one change artifact per plan step");
    assert!(
        r.changed_files
            .iter()
            .all(|p| p.starts_with("prometheos/changes/"))
    );
    // workspace_ref must be a valid, parseable WorkspaceRefV1
    let _ref: prometheos_lite::workflow::workspace::WorkspaceRefV1 =
        prometheos_lite::workflow::workspace::WorkspaceRefV1::parse_json(&r.workspace_ref)
            .expect("workspace_ref round-trips through WorkspaceRefV1");
    // evidence refs: must include a node-output artifact (governed journal entry)
    assert!(
        outcome
            .result
            .evidence_refs
            .iter()
            .any(|e| e.artifact_kind.contains("node-output"))
    );
}

#[test]
fn implement_does_not_mutate_source_checkout() {
    let dir = fixture_repo();
    let root = dir.path();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let plan = build_plan("plan-impl-2", &base_revision, "evidence-digest-2", 1);
    let ws_parent = tempfile::tempdir().unwrap();

    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl", CAP_IMPLEMENT);
    let _ = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": serde_json::to_string(&plan).unwrap(),
            "repoRoot": root.to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "impl-isolation",
    )
    .expect("implement completes");

    // Source repo's tracked files must be unchanged: no `prometheos/` directory
    // and HEAD revision must still equal the original base_revision.
    let current_rev = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(
        current_rev, base_revision,
        "source checkout must not be mutated by the executor"
    );
    assert!(
        !root.join("prometheos").exists(),
        "prometheos/ must live only inside the worktree, not the source repo"
    );
    // `git status` must be clean on the source.
    let status_out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status_out.stdout)
        .trim()
        .to_string();
    assert!(
        status.is_empty(),
        "source checkout must be clean after implement; got: {status}"
    );
}

// ---------------------------------------------------------------------------
// Repair acceptance
// ---------------------------------------------------------------------------

#[test]
fn repair_records_corrective_change_linked_to_diagnosis() {
    let dir = fixture_repo();
    let root = dir.path();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let diagnosis = DiagnosisV1 {
        diagnosis_id: "diag-1".to_string(),
        failing_target: "src/main.rs".to_string(),
        message: "expected retry on transient errors".to_string(),
        base_revision: base_revision.clone(),
    };
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-repair", CAP_REPAIR);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_REPAIR,
        json!({
            "diagnosis": serde_json::to_string(&diagnosis).unwrap(),
            "repoRoot": root.to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "repair-1",
    )
    .expect("repair completes under granted write scope");
    assert_eq!(outcome.result.outcome, OutcomeKind::Completed);

    let r: RepairResultV1 = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(r.diagnosis_ref, "diag-1");
    assert_eq!(r.failing_target, "src/main.rs");
    assert_ne!(r.revision, base_revision);
    assert_eq!(
        r.changed_files,
        vec!["prometheos/repairs/diag-1.repair.json".to_string()]
    );
    assert!(!r.corrective_summary.is_empty());
    // workspace_ref parseable
    let _ref: prometheos_lite::workflow::workspace::WorkspaceRefV1 =
        prometheos_lite::workflow::workspace::WorkspaceRefV1::parse_json(&r.workspace_ref)
            .expect("workspace_ref round-trips through WorkspaceRefV1");
    // evidence refs
    assert!(
        outcome
            .result
            .evidence_refs
            .iter()
            .any(|e| e.artifact_kind.contains("node-output"))
    );
}

// ---------------------------------------------------------------------------
// Conformance categories (policy, bypass, evidence)
// ---------------------------------------------------------------------------

#[test]
fn policy_denies_write_when_local_writable_scope_is_empty() {
    let dir = fixture_repo();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let plan = build_plan("plan-deny", &base_revision, "evidence-deny", 1);

    let mut runner = runner();
    let caps = restrictions_readonly();
    let m = node_manifest("node-impl-deny", CAP_IMPLEMENT);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": serde_json::to_string(&plan).unwrap(),
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": tempfile::tempdir().unwrap().path().to_str().unwrap(),
        }),
        "impl-deny",
    );
    assert!(
        res.is_err(),
        "implement must be denied when local writable scope is empty (fail closed)"
    );
}

#[test]
fn implement_rejects_unparseable_plan() {
    let dir = fixture_repo();
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl-bad", CAP_IMPLEMENT);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": "not-json",
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "impl-bad",
    );
    assert!(res.is_err(), "malformed plan input must be rejected");
}
