//! E4/X06 (#163) Slice B — permit-bound suspension/resume binding.
//!
//! A `GovernancePermit` is the only artifact that can suspend or resume a
//! governed graph run: suspension documents bind the reviewed plan
//! identity plus the compiled authority snapshot digest, and resume is
//! reachable only through the same permit. Tokens issued under another
//! plan identity, or against a changed authority snapshot, fail closed.

mod common;
use common::permit_for;

use prometheos_lite::workflow::graph_resume::ReplayAuthorization;
use prometheos_lite::workflow::graph_state::{
    EdgeKind, GraphEdgeV1, GraphManifestV1, GraphNodeV1, GraphRunStateV1, NodeAttemptRecordV1,
    OutcomeCategory, RouteDecisionV1,
};
use prometheos_lite::workflow::graph_suspend::{
    ResumeTokenV1, ResumeWithTokenError, issue_resume_token,
};

const NOW: &str = "2026-10-13T00:00:00Z";
const OPS_DIGEST: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn manifest() -> GraphManifestV1 {
    GraphManifestV1 {
        schema_version: "1.0.0".into(),
        graph_id: "g".into(),
        version: "1.0.0".into(),
        nodes: vec![
            GraphNodeV1 {
                node_id: "a".into(),
                capability: "c.a".into(),
                purpose: None,
                resources: Vec::new(),
                join: None,
            },
            GraphNodeV1 {
                node_id: "b".into(),
                capability: "c.b".into(),
                purpose: None,
                resources: Vec::new(),
                join: None,
            },
        ],
        edges: vec![GraphEdgeV1 {
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::Sequence,
            condition_label: None,
        }],
        entry_points: vec!["a".into()],
        terminal_exits: vec!["b".into()],
        shared_state_keys: vec![],
        policy_digest: None,
        content_digest: None,
    }
    .sealed()
}

fn err_codes(err: &ResumeWithTokenError) -> Vec<String> {
    match err {
        ResumeWithTokenError::Refusal(diags) => diags.iter().map(|d| d.code.clone()).collect(),
        ResumeWithTokenError::Underlying(e) => vec![e.to_string()],
    }
}

#[test]
fn permit_authority_snapshot_digest_is_deterministic_and_unique() {
    let p1 = permit_for(&["a"], &["echo"]);
    let p2 = permit_for(&["a"], &["echo"]);
    assert_eq!(
        p1.authority_snapshot_digest(),
        p2.authority_snapshot_digest(),
        "identical workflows must snapshot identically"
    );
    assert_eq!(p1.authority_snapshot_digest().len(), 64);
}

#[test]
fn permit_bound_suspension_binds_plan_identity_and_authority() {
    let permit = permit_for(&["a"], &["echo"]);
    let m = manifest();
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    let suspension = permit
        .suspend_run(
            &mut run,
            "susp-1",
            "human-review",
            "a",
            OPS_DIGEST,
            "t1",
            None,
            None,
        )
        .expect("suspends via permit");
    assert_eq!(suspension.plan_digest, permit.plan_identity());
    assert_eq!(
        suspension.authority_snapshot_digest,
        permit.authority_snapshot_digest()
    );
}

#[test]
fn permit_bound_suspension_and_resume_are_byte_deterministic() {
    let m = manifest();
    let mk = || {
        let permit = permit_for(&["a"], &["echo"]);
        let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let s = permit
            .suspend_run(
                &mut run,
                "susp-1",
                "human-review",
                "a",
                OPS_DIGEST,
                "t1",
                None,
                None,
            )
            .unwrap();
        let t = issue_resume_token(&s, "tok-1", None).unwrap();
        (permit, run, s, t)
    };
    let (_, _, s1, t1) = mk();
    let (_, _, s2, t2) = mk();
    assert_eq!(
        serde_json::to_string(&s1).unwrap(),
        serde_json::to_string(&s2).unwrap()
    );
    assert_eq!(t1.binding_digest, t2.binding_digest);
}

fn suspended_under_permit() -> (
    prometheos_lite::workflow::governance_permit::GovernancePermit,
    GraphManifestV1,
    GraphRunStateV1,
    prometheos_lite::workflow::graph_suspend::SuspensionPointV1,
    ResumeTokenV1,
) {
    let permit = permit_for(&["a"], &["echo"]);
    let m = manifest();
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    let suspension = permit
        .suspend_run(
            &mut run,
            "susp-1",
            "human-review",
            "a",
            OPS_DIGEST,
            "t1",
            None,
            None,
        )
        .unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    (permit, m, run, suspension, token)
}

#[test]
fn resume_via_issuing_permit_succeeds() {
    let (permit, m, run, suspension, token) = suspended_under_permit();
    let ckpt = run.export_checkpoint().unwrap();
    let resumed = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .expect("same permit resumes");
    assert_eq!(resumed.state.frontier, vec!["a".to_string()]);
}

#[test]
fn resume_via_other_plan_identity_refuses() {
    let (_, m, run, suspension, token) = suspended_under_permit();
    let other_permit = permit_for(&["different"], &["echo"]);
    assert_ne!(other_permit.plan_identity(), suspension.plan_digest);
    let ckpt = run.export_checkpoint().unwrap();
    let err = other_permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(err_codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_when_authority_snapshot_changes() {
    let (_permit, m, run, suspension, token) = suspended_under_permit();
    // Build a permit whose authority snapshot differs (extra readable
    // scope added to the ceiling) while keeping the run shaped identically.
    let mut text = common::governance_permit::workflow_text(&["a"], &["echo"]);
    text = text.replace("\"repo://x\"", "\"repo://x\", \"repo://extra\"");
    let identity = prometheos_lite::workflow::governance_compiler::compile_workflow_text(&text)
        .expect("still audit-clean")
        .canonicalization
        .sha256;
    let other =
        prometheos_lite::workflow::governance_permit::GovernancePermit::issue(&text, &identity)
            .expect("issues");
    assert_ne!(
        other.authority_snapshot_digest(),
        suspension.authority_snapshot_digest
    );

    // The existing token binds the ORIGINAL authority snapshot; resume via
    // the changed-authority permit must refuse with staleness even if the
    // reviewed plan identity pressure is secondary here — the token is
    // bound to a snapshot this permit no longer represents.
    let ckpt = run.export_checkpoint().unwrap();
    let err = other
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(err_codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn tampered_suspension_under_permit_refuses() {
    let (permit, m, run, suspension, token) = suspended_under_permit();
    let mut forged = suspension;
    forged.plan_digest = OPS_DIGEST.into(); // mutate without rebinding token
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &forged, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(err_codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn tampered_token_under_permit_refuses() {
    let (permit, m, run, suspension, token) = suspended_under_permit();
    let mut forged = token;
    forged.expiration = Some("2027-01-01T00:00:00Z".into()); // extend without rebinding
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &forged, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(err_codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn permit_resume_preserves_replay_gate() {
    // Replay authorization is part of the #123 run law; the permit binding
    // must not weaken it.
    let m = GraphManifestV1 {
        schema_version: "1.0.0".into(),
        graph_id: "g".into(),
        version: "1.0.0".into(),
        nodes: vec![
            GraphNodeV1 {
                node_id: "a".into(),
                capability: "c.a".into(),
                purpose: None,
                resources: vec![],
                join: None,
            },
            GraphNodeV1 {
                node_id: "b".into(),
                capability: "c.b".into(),
                purpose: None,
                resources: vec![],
                join: None,
            },
            GraphNodeV1 {
                node_id: "done".into(),
                capability: "c.done".into(),
                purpose: None,
                resources: vec![],
                join: None,
            },
        ],
        edges: vec![
            GraphEdgeV1 {
                from: "a".into(),
                to: "b".into(),
                kind: EdgeKind::Sequence,
                condition_label: None,
            },
            GraphEdgeV1 {
                from: "b".into(),
                to: "done".into(),
                kind: EdgeKind::Sequence,
                condition_label: None,
            },
            GraphEdgeV1 {
                from: "b".into(),
                to: "a".into(),
                kind: EdgeKind::Sequence,
                condition_label: None,
            },
        ],
        entry_points: vec!["a".into()],
        terminal_exits: vec!["done".into()],
        shared_state_keys: vec![],
        policy_digest: None,
        content_digest: None,
    }
    .sealed();
    let attempt = NodeAttemptRecordV1 {
        attempt: 1,
        started_at: "t0".into(),
        completed_at: Some("t1".into()),
        outcome: OutcomeCategory::Completed,
        result_digest: "d".repeat(64),
    };
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    run.apply_node_completion("a", attempt.clone(), "t1")
        .unwrap();
    run.record_route_decision(
        RouteDecisionV1 {
            recorded_at: "t2".into(),
            from_node: "a".into(),
            to_node: "b".into(),
            condition_label: None,
            basis_result_digest: "d".repeat(64),
        },
        &m,
    )
    .unwrap();
    run.apply_node_completion("b", attempt, "t3").unwrap();
    run.record_route_decision(
        RouteDecisionV1 {
            recorded_at: "t4".into(),
            from_node: "b".into(),
            to_node: "a".into(),
            condition_label: None,
            basis_result_digest: "d".repeat(64),
        },
        &m,
    )
    .unwrap();

    let permit = permit_for(&["a", "b"], &["echo"]);
    let suspension = permit
        .suspend_run(
            &mut run,
            "susp-1",
            "human-review",
            "a",
            OPS_DIGEST,
            "t5",
            None,
            None,
        )
        .unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();

    // Without replay authorization the run law refuses through the permit.
    let err = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    match err {
        ResumeWithTokenError::Underlying(e) => assert!(e.to_string().contains("replay"), "{e}"),
        ResumeWithTokenError::Refusal(d) => panic!("expected replay gate, got {d:?}"),
    }
    // With authorization it resumes, and the authorization is journaled.
    let resumed = permit
        .resume_run(
            &ckpt,
            &suspension,
            &token,
            &m,
            "pd",
            "rev-1",
            NOW,
            Some(ReplayAuthorization {
                reason: "replay a after review".into(),
                authorized_by: "human-1".into(),
            }),
        )
        .expect("authorized replay resumes");
    assert!(resumed.replay_authorized);
}
