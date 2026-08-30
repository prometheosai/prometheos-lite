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
use chrono::Datelike;
use serde_json::json;

use prometheos_lite::workflow::node_contracts::{NodeManifestV1, OutcomeKind};
use prometheos_lite::workflow::node_implementation::{
    CAP_IMPLEMENT, CAP_REPAIR, DiagnosisV1, ImplementationResultV1, RepairResultV1,
    implementation_repair_registry, node_manifest,
};
use prometheos_lite::workflow::node_library::ScopedPlanV1;
use prometheos_lite::workflow::node_runner::{NodeRunOutcome, NodeRunRequest, NodeRunner};
use prometheos_lite::workflow::now_iso;
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::workspace::{
    ADAPTER_REVISION, AdapterKind, GitWorktreeWorkspace, RecoveryOutcome,
    WORKSPACE_REF_SCHEMA_VERSION, WORKSPACE_REF_SCHEMA_VERSION_V1, WORKSPACE_SCHEMA_VERSION,
    WorkspaceAdapter, WorkspaceManifestV1, WorkspaceMode, WorkspaceRefError, WorkspaceRefV1,
};

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

fn init_repo(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "ci@ci"]);
    git(root, &["config", "user.name", "ci"]);
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

fn full_manifest(
    workspace_id: &str,
    repo: &Path,
    base_revision: &str,
    mode: WorkspaceMode,
) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: base_revision.to_string(),
        branch: None,
        mode,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: format!("lock-{workspace_id}"),
        created_at: now_iso(),
        content_digest: None,
    }
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

// ---------------------------------------------------------------------------
// E5/I02 repair: P1/P2 findings — focused tests covering the four regressions
// called out in the post-merge review of #126.
// ---------------------------------------------------------------------------

#[test]
fn repair_p1_1_implement_rejects_path_traversal_in_plan_id() {
    // P1.1: a plan_id containing path traversal must be rejected before any
    // workspace acquisition or filesystem write.
    let dir = fixture_repo();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let mut bad_plan = build_plan("plan-ok", &base_revision, "evidence-x", 1);
    bad_plan.plan_id = "../../../etc/evil".to_string();
    let ws_parent = tempfile::tempdir().unwrap();

    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl-p11", CAP_IMPLEMENT);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": serde_json::to_string(&bad_plan).unwrap(),
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "p11",
    );
    assert!(
        res.is_err(),
        "implement must reject path-traversal plan_id (P1.1), got Ok"
    );
    let msg = res.err().unwrap().to_string();
    assert!(
        msg.contains("unsafe") || msg.contains("..") || msg.contains("path"),
        "error must cite the unsafe plan_id, got: {msg}"
    );
}

#[test]
fn repair_p1_1_repair_rejects_path_traversal_in_diagnosis_id() {
    let dir = fixture_repo();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let bad_diagnosis = DiagnosisV1 {
        diagnosis_id: "../escape".to_string(),
        failing_target: "src/main.rs".to_string(),
        message: "msg".to_string(),
        base_revision,
    };
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-repair-p11", CAP_REPAIR);
    let res = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_REPAIR,
        json!({
            "diagnosis": serde_json::to_string(&bad_diagnosis).unwrap(),
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "p11-repair",
    );
    assert!(
        res.is_err(),
        "repair must reject path-traversal diagnosis_id (P1.1), got Ok"
    );
}

#[test]
fn repair_p1_2_authority_mismatch_is_rejected_fail_closed() {
    // P1.2: repo_identity must canonicalize to source_repo's toplevel. The
    // node builds a manifest whose repo_identity is `dir.path()` (the real
    // fixture), so this test exercises the positive path; we then build a
    // SECOND fixture and confirm that attempting to bind the node's
    // adapter to the wrong source_repo (constructed manually) fails closed.
    let dir_a = fixture_repo();
    let dir_b = fixture_repo();
    let base_a = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir_a.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // Construct a plan whose plan_id is clean (P1.1 still passes) but where
    // we will deliberately build a worktree adapter pointed at dir_b while
    // the manifest claims dir_a. This must fail closed inside acquire().
    let plan = build_plan("plan-p12", &base_a, "evidence-p12", 1);
    let ws_parent = tempfile::tempdir().unwrap();
    let adapter = prometheos_lite::workflow::workspace::GitWorktreeWorkspace {
        parent_dir: ws_parent.path().to_path_buf(),
        // BUG injection: source_repo is dir_b, but the node's manifest will
        // carry dir_a as repo_identity. The new authority check must catch it.
        source_repo: dir_b.path().to_path_buf(),
    };
    let manifest = prometheos_lite::workflow::workspace::WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: "impl-plan-p12".to_string(),
        adapter: prometheos_lite::workflow::workspace::AdapterKind::GitWorktree,
        adapter_revision: "lite.workspace.adapter.v1".to_string(),
        repo_identity: dir_a.path().to_string_lossy().to_string(),
        base_revision: plan.discovery_revision.clone(),
        branch: None,
        mode: prometheos_lite::workflow::workspace::WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-p12".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        content_digest: None,
    }
    .sealed();
    let err = adapter
        .acquire(&manifest)
        .expect_err("acquire must fail closed on repo identity mismatch (P1.2)");
    let msg = err.to_string();
    assert!(
        msg.contains("repo identity mismatch") || msg.contains("mismatch"),
        "expected identity-mismatch error, got: {msg}"
    );
}

#[test]
fn repair_p1_3_emitted_ref_carries_committed_head_and_recovers() {
    // P1.3: the emitted workspace_ref must carry the post-write committed
    // HEAD (headRevision set, baseRevision == newHEAD) and the on-disk
    // worktree's actual HEAD must equal the ref's pinned value so that
    // WorkspaceAdapter::recover() accepts the workspace.
    let dir = fixture_repo();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let plan = build_plan("plan-p13", &base_revision, "evidence-p13", 2);
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl-p13", CAP_IMPLEMENT);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": serde_json::to_string(&plan).unwrap(),
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "p13",
    )
    .expect("implement completes");
    let r: ImplementationResultV1 = serde_json::from_str(&outcome.output).unwrap();
    let parsed: prometheos_lite::workflow::workspace::WorkspaceRefV1 =
        prometheos_lite::workflow::workspace::WorkspaceRefV1::parse_json(&r.workspace_ref)
            .expect("workspace_ref round-trips");
    // P1.3 invariants
    assert_eq!(
        parsed.head_revision.as_deref(),
        Some(r.revision.as_str()),
        "headRevision must equal the committed HEAD"
    );
    assert_eq!(
        parsed.base_revision, r.revision,
        "baseRevision must be the post-write HEAD so recover() can pin"
    );
    // Digest must verify against the new struct contents.
    assert_eq!(
        parsed.content_digest,
        parsed.compute_digest(),
        "content_digest must be self-consistent after the new headRevision"
    );
    // The worktree sits at the committed HEAD, so recover() must accept it.
    let adapter = prometheos_lite::workflow::workspace::GitWorktreeWorkspace {
        parent_dir: ws_parent.path().to_path_buf(),
        source_repo: dir.path().to_path_buf(),
    };
    let acquired_root = ws_parent.path().join("impl-plan-p13").join("worktree");
    assert!(acquired_root.exists(), "worktree must still be on disk");
    // We need a manifest whose base_revision matches; the ref's
    // base_revision is the committed HEAD, so a minimal sealed manifest
    // that just declares identity + mode + adapter is enough to drive
    // recover(). The adapter's own recover() check uses ref.headRevision
    // (or ref.baseRevision as fallback) AND manifest.base_revision.
    // recover() expects a full WorkspaceManifestV1; build one with
    // base_revision = committed HEAD so the stale-revision check passes.
    let full_manifest = prometheos_lite::workflow::workspace::WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: parsed.workspace_id.clone(),
        adapter: parsed.adapter,
        adapter_revision: parsed.adapter_revision.clone(),
        repo_identity: parsed.repo_identity.clone(),
        base_revision: parsed.base_revision.clone(),
        branch: None,
        mode: parsed.mode,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: format!("lock-{}", parsed.workspace_id),
        created_at: chrono::Utc::now().to_rfc3339(),
        content_digest: None,
    }
    .sealed();
    let recovered = adapter
        .recover(&acquired_root, &parsed, &full_manifest, None)
        .expect("recover should not error");
    match recovered {
        prometheos_lite::workflow::workspace::RecoveryOutcome::Resumed(aw) => {
            assert_eq!(
                aw.revision, r.revision,
                "recovered HEAD must equal the committed HEAD"
            );
        }
        other => panic!("expected Resumed, got {other:?}"),
    }
}

#[test]
fn repair_p2_emitted_timestamp_round_trips_through_chrono() {
    // P2: the audit timestamps embedded in ImplementationResultV1 must be
    // valid RFC3339 datetimes parseable by chrono (the broken hand-rolled
    // `chrono_like_iso` produced strings that did not round-trip).
    let dir = fixture_repo();
    let base_revision = {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let plan = build_plan("plan-p2", &base_revision, "evidence-p2", 1);
    let ws_parent = tempfile::tempdir().unwrap();
    let mut runner = runner();
    let caps = restrictions_with_write();
    let m = node_manifest("node-impl-p2", CAP_IMPLEMENT);
    let outcome = run_node(
        &mut runner,
        &m,
        &caps,
        CAP_IMPLEMENT,
        json!({
            "plan": serde_json::to_string(&plan).unwrap(),
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
        }),
        "p2",
    )
    .expect("implement completes");
    let r: ImplementationResultV1 = serde_json::from_str(&outcome.output).unwrap();
    // Every change must have an applied_at that parses as a real datetime.
    for c in &r.changes {
        let parsed = chrono::DateTime::parse_from_rfc3339(&c.applied_at)
            .unwrap_or_else(|e| panic!("applied_at not RFC3339 ({}): {}", c.applied_at, e));
        // Must be within a sane window of "now" (between 2024 and 2100).
        let year = parsed.year();
        assert!(
            (2024..=2100).contains(&year),
            "applied_at year {year} out of plausible range"
        );
    }
}

// ===========================================================================
// Repair round 2 regressions (post-#197 findings)
// ===========================================================================

#[test]
fn recovery_with_original_manifest_succeeds_when_head_revision_set() {
    // P1 regression: recover() previously compared committed HEAD against the
    // manifest's base_revision even when headRevision was set. With the fix,
    // when headRevision is present, the manifest's base_revision is NOT
    // compared against the actual HEAD (it may legitimately carry the
    // pre-write base as an audit trail).
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let pre_commit = git_output(&repo, &["rev-parse", "HEAD"]);

    // Acquire with the pre-commit manifest.
    let manifest = full_manifest(
        "recovery-original-manifest",
        &repo,
        &pre_commit,
        WorkspaceMode::Writable,
    );
    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };
    let acquired = adapter.acquire(&manifest).unwrap();

    // Commit something into the worktree.
    std::fs::write(acquired.root.join("new.txt"), "written").unwrap();
    git(&acquired.root, &["add", "."]);
    git(&acquired.root, &["commit", "-q", "-m", "write in worktree"]);
    let post_commit = git_output(&acquired.root, &["rev-parse", "HEAD"]);
    assert_ne!(pre_commit, post_commit, "worktree must have committed");

    // Build a post-write ref at 1.1.0 with headRevision = post-commit.
    let mut ref_with_head = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION.to_string(),
        workspace_id: "recovery-original-manifest".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: pre_commit.clone(), // pre-write base (audit trail)
        mode: WorkspaceMode::Writable,
        content_digest: String::new(),
        head_revision: Some(post_commit.clone()),
    };
    ref_with_head.content_digest = ref_with_head.compute_digest();

    // Recover with the ORIGINAL manifest (base_revision = pre-commit).
    // This must succeed because headRevision overrides the manifest check.
    let outcome = adapter.recover(
        &acquired.root,
        &ref_with_head,
        &manifest, // original: base_revision = pre-commit
        None,
    );
    match outcome.unwrap() {
        RecoveryOutcome::Resumed(a) => {
            assert_eq!(a.revision, post_commit);
        }
        other => panic!("expected Resumed, got {:?}", other),
    }
}

#[test]
fn recovery_rejects_stale_when_head_revision_absent_and_manifest_disagrees() {
    // Complementary regression: when headRevision is absent, recovery must
    // still enforce the manifest.base_revision == actual HEAD check (pre-write
    // recovery). A ref without headRevision and a manifest whose base is stale
    // must be rejected.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // Make a second commit to move HEAD.
    std::fs::write(repo.join("second.txt"), "second").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "second"]);
    let current = git_output(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(base, current);

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };

    // Acquire a real worktree pinned to current HEAD.
    let acquire_manifest =
        full_manifest("stale-manifest", &repo, &current, WorkspaceMode::Writable);
    let acquired = adapter.acquire(&acquire_manifest).unwrap();

    // Ref without headRevision, base_revision = current.
    let ref_no_head = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION_V1.to_string(),
        workspace_id: "stale-manifest".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: current.clone(),
        mode: WorkspaceMode::Writable,
        content_digest: String::new(),
        head_revision: None, // pre-write ref
    };

    // Manifest with stale base_revision (the old commit).
    let stale_manifest = full_manifest(
        "stale-manifest",
        &repo,
        &base, // stale: does not match current HEAD
        WorkspaceMode::Writable,
    );

    let outcome = adapter.recover(&acquired.root, &ref_no_head, &stale_manifest, None);
    match outcome.unwrap() {
        RecoveryOutcome::Rejected(e) => {
            assert!(matches!(e, WorkspaceRefError::StaleRevision));
        }
        other => panic!("expected Rejected(StaleRevision), got {:?}", other),
    }
}

#[test]
fn authority_binding_accepts_url_identity_matching_origin_remote() {
    // P1.2 regression: repo_identity documented as "origin URL or stable name".
    // A URL identity must bind to the source_repo's origin remote.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // Set origin remote to a URL.
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/repo.git",
        ],
    );

    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "url-identity-test".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: "https://github.com/example/repo.git".to_string(),
        base_revision: base.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-url-test".to_string(),
        created_at: now_iso(),
        content_digest: None,
    };

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };

    // Must succeed: URL matches origin remote.
    let acquired = adapter.acquire(&manifest).unwrap();
    assert_eq!(acquired.manifest.base_revision, base);
}

#[test]
fn authority_binding_rejects_url_identity_mismatch() {
    // P1.2 regression: URL identity that does NOT match origin remote must
    // be rejected.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // Set origin remote to a DIFFERENT URL.
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/other/repo.git",
        ],
    );

    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "url-mismatch".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: "https://github.com/example/repo.git".to_string(),
        base_revision: base.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-url-mismatch".to_string(),
        created_at: now_iso(),
        content_digest: None,
    };

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };

    let err = adapter.acquire(&manifest).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("repo identity mismatch"),
        "expected repo identity mismatch error, got: {msg}"
    );
}

#[test]
fn authority_binding_accepts_stable_name_identity() {
    // P1.2 regression: stable-name identity (bare label) is accepted per the
    // documented contract. The manifest's contentDigest commits the exact
    // string so silent substitution is auditable.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "stable-name-test".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: "my-stable-repo-name".to_string(),
        base_revision: base.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-stable-name".to_string(),
        created_at: now_iso(),
        content_digest: None,
    };

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };

    // Must succeed: stable-name is a label, no binding beyond the digest.
    let acquired = adapter.acquire(&manifest).unwrap();
    assert_eq!(acquired.manifest.base_revision, base);
}

#[test]
fn workspace_ref_v1_accepts_1_1_0_schema_with_head_revision() {
    // P2 regression: new ref schema 1.1.0 with headRevision must parse.
    let ref_json = r#"{
        "schemaVersion": "1.1.0",
        "workspaceId": "test-v1.1",
        "adapter": "git-worktree",
        "adapterRevision": "lite.workspace.adapter.v1",
        "repoIdentity": "my-repo",
        "baseRevision": "abc123",
        "mode": "writable",
        "contentDigest": "digest123",
        "headRevision": "def456"
    }"#;
    let parsed = WorkspaceRefV1::parse_json(ref_json).unwrap();
    assert_eq!(parsed.schema_version, "1.1.0");
    assert_eq!(parsed.head_revision.as_deref(), Some("def456"));
}

#[test]
fn workspace_ref_v1_accepts_1_0_0_schema_without_head_revision() {
    // P2 regression: old ref schema 1.0.0 (without headRevision) must still
    // parse for backward compatibility.
    let ref_json = r#"{
        "schemaVersion": "1.0.0",
        "workspaceId": "test-v1.0",
        "adapter": "git-worktree",
        "adapterRevision": "lite.workspace.adapter.v1",
        "repoIdentity": "my-repo",
        "baseRevision": "abc123",
        "mode": "writable",
        "contentDigest": "digest123"
    }"#;
    let parsed = WorkspaceRefV1::parse_json(ref_json).unwrap();
    assert_eq!(parsed.schema_version, "1.0.0");
    assert!(parsed.head_revision.is_none());
}

#[test]
fn workspace_ref_v1_rejects_unknown_schema_version() {
    // P2 regression: refs at an unsupported version MUST be rejected.
    let ref_json = r#"{
        "schemaVersion": "2.0.0",
        "workspaceId": "test-v2",
        "adapter": "git-worktree",
        "adapterRevision": "lite.workspace.adapter.v1",
        "repoIdentity": "my-repo",
        "baseRevision": "abc123",
        "mode": "writable",
        "contentDigest": "digest123"
    }"#;
    let err = WorkspaceRefV1::parse_json(ref_json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unsupported workspace ref schema version 2.0.0"),
        "expected unsupported version error, got: {msg}"
    );
}

#[test]
fn workspace_ref_v1_1_1_0_rejects_unknown_fields() {
    // P2 regression: deny_unknown_fields must still apply at 1.1.0.
    let ref_json = r#"{
        "schemaVersion": "1.1.0",
        "workspaceId": "test-v1.1-unknown",
        "adapter": "git-worktree",
        "adapterRevision": "lite.workspace.adapter.v1",
        "repoIdentity": "my-repo",
        "baseRevision": "abc123",
        "mode": "writable",
        "contentDigest": "digest123",
        "futureField": "surprise"
    }"#;
    let err = WorkspaceRefV1::parse_json(ref_json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown field") || msg.contains("parse failed"),
        "expected unknown field or parse error, got: {msg}"
    );
}
