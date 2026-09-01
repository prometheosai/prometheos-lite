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
    ADAPTER_REVISION, AdapterKind, ExistingReadOnlyWorkspace, GitWorktreeWorkspace,
    RecoveryOutcome, WORKSPACE_REF_SCHEMA_VERSION, WORKSPACE_REF_SCHEMA_VERSION_V1,
    WORKSPACE_SCHEMA_VERSION, WorkspaceAdapter, WorkspaceManifestV1, WorkspaceMode,
    WorkspaceRefError, WorkspaceRefV1, stable_repo_identity_digest,
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
fn repair_p1_3_emitted_ref_carries_original_base_and_authenticates_manifest() {
    // P1.3: the emitted workspace_ref must carry the ORIGINAL acquisition base
    // (manifest.base_revision) so recovery can bind the reference back to the
    // originating manifest. headRevision = committed HEAD for on-disk
    // revalidation. Round 4: the ref authenticates ONLY the originating
    // manifest (contentDigest = manifest digest); any other manifest —
    // even field-identical — must be refused.
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
    // P1.3 invariants: baseRevision = original acquisition base (NOT committed HEAD).
    assert_eq!(
        parsed.base_revision, base_revision,
        "baseRevision must be the original acquisition base, not the committed HEAD"
    );
    assert_eq!(
        parsed.head_revision.as_deref(),
        Some(r.revision.as_str()),
        "headRevision must equal the committed HEAD"
    );
    // contentDigest = the ORIGINATING manifest's digest (attestation). It
    // must be a 64-char lowercase hex SHA-256, i.e. the canonical manifest
    // digest format. (The misleading per-field reference compute_digest()
    // was removed in repair round 4; refs no longer digest themselves.)
    assert!(
        parsed.content_digest.len() == 64
            && parsed
                .content_digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "contentDigest must be a canonical manifest digest, got: {}",
        parsed.content_digest
    );
    // Recovery: the adapter needs the source_repo for authority binding.
    let adapter = prometheos_lite::workflow::workspace::GitWorktreeWorkspace {
        parent_dir: ws_parent.path().to_path_buf(),
        source_repo: dir.path().to_path_buf(),
    };
    let acquired_root = ws_parent.path().join("impl-plan-p13").join("worktree");
    assert!(acquired_root.exists(), "worktree must still be on disk");
    // Repair round 4: the reference authenticates ONLY its originating
    // manifest. A re-created manifest — even one with matching base/identity
    // fields — is NOT the originating manifest (its created_at differs), so
    // recovery must REFUSE it with a manifest-attestation error. (Positive
    // recovery with the persisted originating manifest is covered by
    // `recovery_with_original_manifest_succeeds_when_head_revision_set` and
    // the conformance kit; the node does not re-emit its manifest here.)
    let mut foreign_manifest = full_manifest(
        &parsed.workspace_id,
        dir.path(),
        &parsed.base_revision, // matching base — but not the origin manifest
        WorkspaceMode::Writable,
    );
    foreign_manifest.created_at = "2000-01-01T00:00:00Z".to_string(); // definitely foreign
    let foreign_manifest = foreign_manifest.sealed();
    let err = adapter
        .recover(&acquired_root, &parsed, &foreign_manifest, None)
        .expect_err("recovery must refuse a manifest that is not the originating manifest");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest-attestation refusal, got: {err}"
    );
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
    // when headRevision is present, BOTH checks apply:
    //   1. reference.baseRevision == manifest.baseRevision (base binding)
    //   2. actual HEAD == reference.headRevision (on-disk pin)
    // The manifest's base_revision is the acquisition base; the ref carries
    // that same base so recovery binds the reference to the originating
    // manifest. content_digest = manifest's digest (attestation).
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

    // Build a post-write ref: baseRevision = original base (manifest binding),
    // headRevision = post-commit (on-disk pin), content_digest = manifest digest.
    let ref_with_head = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION.to_string(),
        workspace_id: "recovery-original-manifest".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: pre_commit.clone(), // original base (manifest binding)
        mode: WorkspaceMode::Writable,
        content_digest: manifest.compute_digest(), // manifest's digest
        head_revision: Some(post_commit.clone()),
    };

    // Recover with the ORIGINAL manifest (base_revision = pre-commit).
    // Both checks pass: ref.baseRevision == manifest.baseRevision AND
    // actual HEAD == ref.headRevision.
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
    // The ref must attest to the STALE manifest (the one that will be
    // supplied to recovery) so that the attestation check PASSES and the
    // revision staleness is the sole rejection reason.
    let ref_no_head = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION_V1.to_string(),
        workspace_id: "stale-manifest".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: current.clone(),
        mode: WorkspaceMode::Writable,
        content_digest: String::new(), // set below after the manifest exists
        head_revision: None,           // pre-write ref
    };

    // Manifest with stale base_revision (the old commit).
    let stale_manifest = full_manifest(
        "stale-manifest",
        &repo,
        &base, // stale: does not match current HEAD
        WorkspaceMode::Writable,
    );
    let ref_no_head = WorkspaceRefV1 {
        content_digest: stale_manifest.compute_digest(),
        ..ref_no_head
    };

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
    // Round 4: stable-name identity binds through the named remote's
    // normalized URL digest. The remote's NAME is deliberately unrelated to
    // the identity ("arbitrary-alias") to prove the alias itself is never
    // consulted — only the digest of the URL it points to.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    let trusted_url = "https://github.com/trusted-org/trusted-repo.git";
    // Remote alias name intentionally unrelated to the identity value.
    git(&repo, &["remote", "add", "arbitrary-alias", trusted_url]);

    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "stable-name-test".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: stable_repo_identity_digest(trusted_url),
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

    // Must succeed: a remote's URL resolves to the declared identity digest.
    let acquired = adapter.acquire(&manifest).unwrap();
    assert_eq!(acquired.manifest.base_revision, base);
}

#[test]
fn authority_binding_rejects_stable_name_with_no_matching_remote() {
    // Stable-name identity must NOT bind when no remote's URL digest matches,
    // even if a remote with the identity's string as its NAME exists (the
    // remote name is not the identifier). But here we go further: no remote
    // at all means nothing to resolve — rejection must still fail closed.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // No remote added — the digest has nothing to resolve against.
    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "stable-no-match".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: stable_repo_identity_digest(
            "https://github.com/somewhere-else/never-added.git",
        ),
        base_revision: base.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-stable-no-match".to_string(),
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
fn authority_binding_rejects_origin_alias_reuse_across_unrelated_repos() {
    // Round 4 regression (P1): unrelated repositories commonly have a remote
    // named "origin". The stable-name identity must NOT bind via the remote
    // NAME — it must resolve to the URL's digest. A source repo whose
    // "origin" points to an entirely different repository must be rejected
    // even when the declared digest matches a DIFFERENT repository's
    // "origin" remote.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // The attacker/tampered repo's "origin" points at THEIR repository.
    let attacker_url = "https://github.com/evil-org/evil-repo.git";
    git(&repo, &["remote", "add", "origin", attacker_url]);

    // But the manifest declares the digest of the TRUSTED repository.
    // Alias reuse ("origin" exists) must not satisfy binding.
    let trusted_digest =
        stable_repo_identity_digest("https://github.com/trusted-org/trusted-repo.git");
    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: "alias-reuse".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: trusted_digest,
        base_revision: base.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: "lock-alias-reuse".to_string(),
        created_at: now_iso(),
        content_digest: None,
    };

    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };

    let err = adapter.acquire(&manifest).unwrap_err();
    assert!(
        err.to_string().contains("repo identity mismatch"),
        "expected repo identity mismatch error, got: {err}"
    );
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

#[test]
fn schema_v1_rejects_head_revision_field() {
    // Finding 5: a ref payload declaring schema_version 1.0.0 with the
    // headRevision field (introduced in 1.1.0) must be rejected so a legacy
    // 1.0.0 reader with deny_unknown_fields fails closed. A 1.0.0 payload
    // with headRevision is a schema violation.
    let ref_json = r#"{
        "schemaVersion": "1.0.0",
        "workspaceId": "test-reject-v1-head",
        "adapter": "git-worktree",
        "adapterRevision": "lite.workspace.adapter.v1",
        "repoIdentity": "/tmp/repo",
        "baseRevision": "abc123",
        "mode": "writable",
        "contentDigest": "digest123",
        "headRevision": "def456"
    }"#;
    let err = WorkspaceRefV1::parse_json(ref_json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("1.0.0 must not contain headRevision"),
        "expected schema 1.0.0 headRevision rejection, got: {msg}"
    );
}

#[test]
fn recovery_rejects_base_revision_mismatch() {
    // Finding 2: recovery MUST require reference.baseRevision == manifest.baseRevision
    // even when headRevision is set, to bind the reference to the originating manifest.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    // Acquire and create a worktree.
    let manifest = full_manifest("base-mismatch-test", &repo, &base, WorkspaceMode::Writable);
    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };
    let acquired = adapter.acquire(&manifest).unwrap();
    let actual_head = git_output(&acquired.root, &["rev-parse", "HEAD"]);

    // Build a ref where baseRevision differs from the manifest's baseRevision.
    // headRevision = actual HEAD (so the on-disk check passes), but the base
    // binding must still fail.
    let ref_wrong_base = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION.to_string(),
        workspace_id: "base-mismatch-test".to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: "0000000000000000000000000000000000000000".to_string(), // WRONG base
        mode: WorkspaceMode::Writable,
        content_digest: manifest.compute_digest(),
        head_revision: Some(actual_head.clone()),
    };

    // Recovery must reject: baseRevision mismatch even when head matches.
    let result = adapter.recover(&acquired.root, &ref_wrong_base, &manifest, None);
    match result.unwrap() {
        RecoveryOutcome::Rejected(e) => {
            assert!(
                e.to_string().contains("StaleRevision") || e.to_string().contains("stale"),
                "expected stale/base-mismatch error, got: {e}"
            );
        }
        other => panic!("expected Rejected for base mismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Repair round 4 regressions: reference → manifest authentication
// ---------------------------------------------------------------------------
//
// A WorkspaceRefV1 must attest to ONE manifest: every durable identity field
// must match AND contentDigest must equal that manifest's digest. Any
// substitution — of the digest itself or of workspace_id / repo_identity /
// adapter_revision / mode — must HARD-FAIL (not Rejected, not remappable).

/// Shared fixture: acquire a real worktree and build the honest reference
/// (`manifest.to_reference()`), returning everything a substitution test
/// needs. Each mutation of the honest ref is then checked to fail closed.
fn acquired_and_honest_ref(
    tag: &str,
) -> (
    tempfile::TempDir,
    GitWorktreeWorkspace,
    std::path::PathBuf,
    WorkspaceManifestV1,
    WorkspaceRefV1,
) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base = git_output(&repo, &["rev-parse", "HEAD"]);

    let manifest = full_manifest(tag, &repo, &base, WorkspaceMode::Writable);
    let adapter = GitWorktreeWorkspace {
        parent_dir: dir.path().join("wt"),
        source_repo: repo.clone(),
    };
    let acquired = adapter.acquire(&manifest).unwrap();
    let reference = manifest.to_reference();
    (dir, adapter, acquired.root, manifest, reference)
}

#[test]
fn recovery_rejects_substituted_manifest_digest() {
    // Finding (round 4): contentDigest is the manifest attestation — a ref
    // whose digest does not match the supplied manifest must be refused.
    let (_dir, adapter, root, manifest, mut reference) = acquired_and_honest_ref("sub-digest");
    reference.content_digest = "f".repeat(64);
    let err = adapter
        .recover(&root, &reference, &manifest, None)
        .expect_err("substituted contentDigest must hard-fail");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest attestation refusal, got: {err}"
    );
}

#[test]
fn recovery_rejects_substituted_workspace_id() {
    let (_dir, adapter, root, manifest, mut reference) = acquired_and_honest_ref("sub-wsid");
    reference.workspace_id = "attacker-controlled-id".to_string();
    let err = adapter
        .recover(&root, &reference, &manifest, None)
        .expect_err("substituted workspaceId must hard-fail");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest attestation refusal, got: {err}"
    );
}

#[test]
fn recovery_rejects_substituted_repo_identity() {
    let (_dir, adapter, root, manifest, mut reference) = acquired_and_honest_ref("sub-repoid");
    reference.repo_identity = "https://github.com/evil-org/evil-repo.git".to_string();
    let err = adapter
        .recover(&root, &reference, &manifest, None)
        .expect_err("substituted repoIdentity must hard-fail");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest attestation refusal, got: {err}"
    );
}

#[test]
fn recovery_rejects_substituted_adapter_revision() {
    let (_dir, adapter, root, manifest, mut reference) = acquired_and_honest_ref("sub-adrev");
    reference.adapter_revision = "lite.workspace.adapter.v0".to_string();
    let err = adapter
        .recover(&root, &reference, &manifest, None)
        .expect_err("substituted adapterRevision must hard-fail");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest attestation refusal, got: {err}"
    );
}

#[test]
fn recovery_rejects_substituted_mode() {
    let (_dir, adapter, root, manifest, mut reference) = acquired_and_honest_ref("sub-mode");
    reference.mode = WorkspaceMode::ReadOnly; // manifest is Writable
    let err = adapter
        .recover(&root, &reference, &manifest, None)
        .expect_err("substituted mode must hard-fail");
    assert!(
        err.to_string().contains("does not authenticate"),
        "expected manifest attestation refusal, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Repair round 5 regression: read-only adapter must enforce the same
// manifest-schema gate and base-binding check that the git-worktree adapter
// enforces. Without these, a reference can authenticate the manifest's
// identity fields and resume against a manifest whose acquisition base
// differs from the reference's base — silently re-basing authority onto a
// different lineage.
// ---------------------------------------------------------------------------

fn readonly_fixture_repo(tag: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    std::fs::write(repo.join("init.txt"), "seed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let base_a = git_output(&repo, &["rev-parse", "HEAD"]);
    // Second commit so we have a distinct SHA to use as a wrong base.
    std::fs::write(repo.join("second.txt"), "second").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "second"]);
    let base_b = git_output(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(base_a, base_b);
    // Roll HEAD back to base_a so the checkout matches the reference we will
    // recover against (the actual-HEAD check must not fire before the
    // base-binding check in this test).
    git(&repo, &["reset", "--hard", &base_a]);
    assert_eq!(git_output(&repo, &["rev-parse", "HEAD"]), base_a);
    let _ = tag;
    (dir, repo, base_a)
}

fn readonly_manifest(workspace_id: &str, repo: &Path, base_revision: &str) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo.to_string_lossy().to_string(),
        base_revision: base_revision.to_string(),
        branch: None,
        mode: WorkspaceMode::ReadOnly,
        writable_scopes: vec![],
        resource_lock_id: format!("lock-ro-{workspace_id}"),
        created_at: now_iso(),
        content_digest: None,
    }
}

#[test]
fn readonly_recovery_rejects_base_revision_mismatch_against_manifest() {
    // Round 5: ExistingReadOnlyWorkspace::recover MUST require
    // reference.baseRevision == manifest.baseRevision, matching the
    // git-worktree adapter. Otherwise a reference whose base matches the
    // on-disk HEAD can resume against a manifest carrying a DIFFERENT
    // acquisition base — re-basing authority onto a different lineage.
    let (_dir, repo, base) = readonly_fixture_repo("ro-base");
    let ro_adapter = ExistingReadOnlyWorkspace::bound_to(repo.clone());

    // Honest read-only manifest pinned to the actual checkout HEAD.
    let honest_manifest = readonly_manifest("ro-base-test", &repo, &base);
    // Build a "foreign" manifest with a different base_revision. The
    // read-only adapter's `acquire` will not be called here; we exercise
    // recover() directly against a manifest the ref does NOT attest to.
    let mut foreign_manifest = honest_manifest.clone();
    foreign_manifest.base_revision = "0".repeat(40);
    // The reference carries the FOREIGN manifest's content digest so
    // verify_manifest_attestation passes — but the reference's own
    // base_revision is the actual checkout HEAD. This means:
    //   - the new base-binding check `reference.baseRevision !=
    //     manifest.baseRevision` (base != 0..0) MUST fire for rejection;
    //   - the trailing on-disk HEAD check (`actual != reference.baseRevision`)
    //     would NOT fire (actual == base == reference.baseRevision);
    //   - without the new check, recovery would Resume successfully.
    // The assertion therefore isolates the new check.
    let reference = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION.to_string(),
        workspace_id: foreign_manifest.workspace_id.clone(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: foreign_manifest.adapter_revision.clone(),
        repo_identity: foreign_manifest.repo_identity.clone(),
        base_revision: base.clone(), // matches actual HEAD, != manifest base
        mode: WorkspaceMode::ReadOnly,
        content_digest: foreign_manifest.compute_digest(), // ref attests to foreign manifest
        head_revision: None,
    };

    let outcome = ro_adapter
        .recover(&repo, &reference, &foreign_manifest, None)
        .expect("recover returns a RecoveryOutcome, not Err");
    match outcome {
        RecoveryOutcome::Rejected(e) => {
            assert!(
                matches!(e, WorkspaceRefError::StaleRevision),
                "expected Rejected(StaleRevision) for base mismatch, got {e:?}"
            );
        }
        other => panic!("expected Rejected(StaleRevision), got {other:?}"),
    }
}

#[test]
fn readonly_recovery_rejects_manifest_schema_version_mismatch() {
    // Round 5: ExistingReadOnlyWorkspace::recover MUST enforce
    // manifest.schemaVersion == WORKSPACE_SCHEMA_VERSION, matching the
    // git-worktree adapter. Without this, a manifest at a non-current schema
    // could authorize recovery through a misconfigured adapter.
    let (_dir, repo, base) = readonly_fixture_repo("ro-schema");
    let ro_adapter = ExistingReadOnlyWorkspace::bound_to(repo.clone());

    let mut bogus_manifest = readonly_manifest("ro-schema-test", &repo, &base);
    bogus_manifest.schema_version = "9.9.9".to_string();
    // Authenticate the bogus-schema manifest so the ref passes
    // verify_manifest_attestation (which does not check schema).
    let reference = WorkspaceRefV1 {
        schema_version: WORKSPACE_REF_SCHEMA_VERSION.to_string(),
        workspace_id: bogus_manifest.workspace_id.clone(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: bogus_manifest.adapter_revision.clone(),
        repo_identity: bogus_manifest.repo_identity.clone(),
        base_revision: bogus_manifest.base_revision.clone(),
        mode: WorkspaceMode::ReadOnly,
        content_digest: bogus_manifest.compute_digest(),
        head_revision: None,
    };

    let outcome = ro_adapter
        .recover(&repo, &reference, &bogus_manifest, None)
        .expect("recover returns a RecoveryOutcome, not Err");
    match outcome {
        RecoveryOutcome::Rejected(e) => {
            assert!(
                matches!(e, WorkspaceRefError::IncompatibleSchema),
                "expected Rejected(IncompatibleSchema) for bad manifest schema, got {e:?}"
            );
        }
        other => panic!("expected Rejected(IncompatibleSchema), got {other:?}"),
    }
}
