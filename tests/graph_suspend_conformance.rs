//! E4/X05 (#162) + E4/X06 (#163 Slice B) — suspension/resume conformance.
//!
//! Transitions are permit-only: the vendored-shaped `SuspensionPointV1` /
//! `ResumeTokenV1` primitives take a crate-private `AuthorityBinding`
//! minted by `GovernancePermit`, so no caller can suspend or resume under
//! caller-supplied digests. Tests exercise the full fail-closed ladder
//! through that permit surface.

mod common;
use common::permit_for;

use prometheos_lite::workflow::governance_permit::GovernancePermit;
use prometheos_lite::workflow::graph_resume::{ReplayAuthorization, resume_run};
use prometheos_lite::workflow::graph_state::{
    EdgeKind, GraphEdgeV1, GraphManifestV1, GraphNodeV1, GraphRunStateV1, NodeAttemptRecordV1,
    OutcomeCategory, RouteDecisionV1,
};
use prometheos_lite::workflow::graph_suspend::{
    ResumeTokenV1, ResumeWithTokenError, SuspensionPointV1, issue_resume_token,
};

const NOW: &str = "2026-10-13T00:00:00Z";
const EARLY: &str = "2026-10-12T00:00:00Z";
const INPUTS_DIGEST: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const OTHER_DIGEST: &str = "3333333333333333333333333333333333333333333333333333333333333333";

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

fn open_run(m: &GraphManifestV1) -> GraphRunStateV1 {
    GraphRunStateV1::open("r1", m, "rev-1", "pws", "pd", "t0").expect("open run")
}

fn suspend_now(
    p: &GovernancePermit,
    run: &mut GraphRunStateV1,
    id: &str,
    op: &str,
    inputs: &str,
) -> Result<SuspensionPointV1, anyhow::Error> {
    p.suspend_run(run, id, "human-review", op, inputs, "t1", None, None)
}

type Fixture = (
    GovernancePermit,
    GraphManifestV1,
    GraphRunStateV1,
    SuspensionPointV1,
    ResumeTokenV1,
);

fn fixture() -> Fixture {
    let permit = permit_for(&["a"], &["echo"]);
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_now(&permit, &mut run, "susp-1", "a", INPUTS_DIGEST).expect("suspend");
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    (permit, m, run, suspension, token)
}

fn codes(err: &ResumeWithTokenError) -> Vec<String> {
    match err {
        ResumeWithTokenError::Refusal(diags) => diags.iter().map(|d| d.code.clone()).collect(),
        ResumeWithTokenError::Underlying(e) => vec![e.to_string()],
    }
}

#[test]
fn suspension_point_round_trips_with_vendored_shape() {
    let (permit, _m, _run, suspension, _token) = fixture();
    assert_eq!(suspension.schema_version, "1.0.0");
    assert_eq!(suspension.id, "susp-1");
    assert_eq!(suspension.plan_digest, permit.plan_identity());
    assert_eq!(
        suspension.authority_snapshot_digest,
        permit.authority_snapshot_digest()
    );
    assert_eq!(suspension.resumability.operation_id, "a");
    assert_eq!(suspension.resumability.inputs_digest, INPUTS_DIGEST);
    assert_eq!(suspension.state_digest.len(), 64);
    let value = serde_json::to_value(&suspension).expect("serializes");
    let keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    for expected in [
        "id",
        "schemaVersion",
        "workflowVersion",
        "stateDigest",
        "planDigest",
        "authoritySnapshotDigest",
        "resumability",
        "provenance",
    ] {
        assert!(keys.contains(&expected), "missing member {expected}");
    }
    assert!(!keys.contains(&"expiration"), "absent when unset");
    let round: SuspensionPointV1 =
        serde_json::from_value(serde_json::to_value(&suspension).unwrap()).unwrap();
    assert_eq!(round, suspension);
    let mut v = serde_json::to_value(&suspension).unwrap();
    v.as_object_mut()
        .unwrap()
        .insert("mystery".into(), serde_json::Value::Null);
    let dj: Result<SuspensionPointV1, serde_json::Error> = serde_json::from_value(v);
    assert!(dj.is_err());
}

#[test]
fn suspend_journals_evidence_and_reseals_state() {
    let (permit, m, run, suspension, _token) = fixture();
    let _ = permit;
    assert_eq!(run.evidence_refs.len(), 1);
    let ev = &run.evidence_refs[0];
    assert_eq!(ev.id, "suspension:susp-1");
    assert_eq!(ev.artifact_kind, "suspension-point");
    assert_eq!(ev.event_digest, suspension.digest().expect("digest"));
    assert_eq!(ev.artifact_digest, suspension.state_digest);
    let ckpt = run.export_checkpoint().expect("export");
    let resumed = resume_run(&ckpt, &m, "pd", "rev-1", None).expect("resume after suspend");
    assert_eq!(resumed.state.evidence_refs.len(), 1);
}

#[test]
fn suspension_and_token_digests_are_byte_deterministic() {
    let (_, _, _, s_a, t_a) = fixture();
    let (_, _, _, s_b, t_b) = fixture();
    assert_eq!(s_a.digest().unwrap(), s_b.digest().unwrap());
    assert_eq!(
        serde_json::to_string(&s_a).unwrap(),
        serde_json::to_string(&s_b).unwrap()
    );
    assert_eq!(t_a.binding_digest, t_b.binding_digest);
    assert_eq!(t_a.suspension_digest, t_b.suspension_digest);
}

use prometheos_lite::workflow::soma::canonical::try_canonical_digest;

#[test]
fn suspension_digest_is_the_canonical_digest_of_the_document() {
    let (_permit, _m, _run, suspension, _token) = fixture();
    let value = serde_json::to_value(&suspension).unwrap();
    let digest = try_canonical_digest(&value).unwrap();
    assert_eq!(suspension.digest().unwrap(), digest);
}

#[test]
fn suspend_refuses_terminated_run() {
    let (permit, _m, mut run, ..) = fixture();
    prometheos_lite::workflow::graph_resume::cancel_run(&mut run, "stop", "t2").unwrap();
    let err = permit
        .suspend_run(
            &mut run,
            "susp-9",
            "human-review",
            "a",
            INPUTS_DIGEST,
            "t2",
            None,
            None,
        )
        .unwrap_err();
    assert!(err.to_string().contains("terminated"), "{err}");
}

#[test]
fn resume_with_valid_token_succeeds_and_preserves_frontier() {
    let (permit, m, run, suspension, token) = fixture();
    let ckpt = run.export_checkpoint().unwrap();
    let resumed = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .expect("valid token resumes");
    assert_eq!(resumed.state.frontier, vec!["a".to_string()]);
}

#[test]
fn resume_refuses_tampered_token_binding() {
    let (permit, m, run, suspension, token) = fixture();
    let mut forged = token;
    forged.plan_digest = OTHER_DIGEST.into(); // mutate without re-binding
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &forged, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn resume_refuses_token_pointing_at_other_suspension() {
    let (permit, m, run, suspension, _token) = fixture();
    // Mint a real token over a *different* suspension on the same permit.
    let mut other_run = open_run(&m);
    let other_susp = suspend_now(&permit, &mut other_run, "susp-other", "b", INPUTS_DIGEST)
        .expect("second suspension");
    let foreign = issue_resume_token(&other_susp, "tok-2", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &foreign, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn resume_refuses_expired_token() {
    let (permit, m, run, suspension, _token) = fixture();
    let expired = issue_resume_token(&suspension, "tok-x", Some("2026-10-12T23:59:59Z")).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &expired, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0002".to_string()]);
    // Bound-in expiry: an earlier clock reading accepts; a later reading
    // refuses — same token, no drift.
    let ok = permit.resume_run(&ckpt, &suspension, &expired, &m, "pd", "rev-1", EARLY, None);
    assert!(
        ok.is_ok(),
        "{}",
        ok.err().map(|e| codes(&e).join(",")).unwrap_or_default()
    );
}

#[test]
fn resume_refuses_unsupported_versions() {
    let (permit, m, run, suspension, token) = fixture();
    // A forged schemaVersion refuses as unsupported before the binding
    // recompute (the version gate runs first in the ladder).
    let mut bad_token = token;
    bad_token.schema_version = "9.9.9".into();
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &bad_token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0004".to_string()]);
}

#[test]
fn resume_refuses_under_foreign_plan_identity() {
    let (_permit, m, run, suspension, token) = fixture();
    let foreign_permit = permit_for(&["different"], &["echo"]);
    assert_ne!(foreign_permit.plan_identity(), suspension.plan_digest);
    let ckpt = run.export_checkpoint().unwrap();
    let err = foreign_permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_checkpoint_that_moved_since_suspension() {
    let (permit, m, run, suspension, token) = fixture();
    let mut stale = run;
    let attempt = NodeAttemptRecordV1 {
        attempt: 1,
        started_at: "t9".into(),
        completed_at: Some("t10".into()),
        outcome: OutcomeCategory::Completed,
        result_digest: "d".repeat(64),
    };
    stale.apply_node_completion("a", attempt, "t10").unwrap();
    let ckpt = stale.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn expiration_is_bound_into_the_token_binding() {
    let (permit, m, run, suspension, _token) = fixture();
    let issued = issue_resume_token(&suspension, "tok-e", Some("2026-10-14T00:00:00Z")).unwrap();
    let ckpt = run.export_checkpoint().unwrap();

    // Removing expiration must invalidate the binding (absence is explicit).
    let mut removed = issued.clone();
    removed.expiration = None;
    let err = permit
        .resume_run(&ckpt, &suspension, &removed, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // Extending expiration must invalidate the binding.
    let mut extended = issued;
    extended.expiration = Some("2027-01-01T00:00:00Z".into());
    let err = permit
        .resume_run(&ckpt, &suspension, &extended, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // The intact token still passes at an in-window clock reading.
    let issued = issue_resume_token(&suspension, "tok-e", Some("2026-10-14T00:00:00Z")).unwrap();
    assert!(
        permit
            .resume_run(&ckpt, &suspension, &issued, &m, "pd", "rev-1", NOW, None)
            .is_ok()
    );
}

#[test]
fn resume_refuses_pre_suspension_checkpoint() {
    let permit = permit_for(&["a"], &["echo"]);
    let m = manifest();
    let mut run = open_run(&m);
    let pre_ckpt = run.export_checkpoint().unwrap();
    let suspension = suspend_now(&permit, &mut run, "susp-1", "a", INPUTS_DIGEST).expect("suspend");
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let err = permit
        .resume_run(&pre_ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_tampered_suspension_evidence_entry() {
    let (permit, m, run, suspension, token) = fixture();
    let mut run = run;
    // Corrupt the journaled event digest and reseal the state honestly.
    run.evidence_refs[0].event_digest = OTHER_DIGEST.into();
    run.content_digest = Some(run.compute_digest());
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_duplicate_suspension_evidence_entries() {
    let (permit, m, run, suspension, token) = fixture();
    let mut run = run;
    let dup = run.evidence_refs[0].clone();
    run.evidence_refs.push(dup);
    run.content_digest = Some(run.compute_digest());
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn suspend_refuses_malformed_inputs_digest() {
    let permit = permit_for(&["a"], &["echo"]);
    let m = manifest();
    let mut run = open_run(&m);
    for (inputs, label) in [
        ("abc", "short"),
        ("not-hex", "not hex"),
        (&"F".repeat(64), "uppercase"),
    ] {
        let err = permit
            .suspend_run(
                &mut run,
                "susp-x",
                "human-review",
                "a",
                inputs,
                "t1",
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("64-hex"), "{label}: {err}");
    }
}

#[test]
fn valid_token_still_requires_replay_authorization() {
    // Manifest variant with a cycle edge b -> a.
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

    let permit = permit_for(&["a"], &["echo"]);
    let suspension = suspend_now(&permit, &mut run, "susp-1", "a", INPUTS_DIGEST).expect("suspend");
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();

    // Valid token but no replay authorization -> underlying replay gate refuses.
    let err = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    match err {
        ResumeWithTokenError::Underlying(e) => assert!(e.to_string().contains("replay"), "{e}"),
        ResumeWithTokenError::Refusal(d) => {
            panic!("token was valid; expected replay gate, got {d:?}")
        }
    }
    // Valid token + authorization -> resumes (and is journaled).
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
                reason: "replay a".into(),
                authorized_by: "op".into(),
            }),
        )
        .expect("authorized replay resumes");
    assert!(resumed.replay_authorized);
}

#[test]
fn declared_failure_precedence_holds_for_overlapping_invalidity() {
    let (permit, m, run, suspension, token) = fixture();
    let ckpt = run.export_checkpoint().unwrap();

    // Unsupported version beats tampered binding (gate 1 before gate 3).
    let mut both = token.clone();
    both.schema_version = "9.9.9".into();
    both.plan_digest = OTHER_DIGEST.into();
    let err = permit
        .resume_run(&ckpt, &suspension, &both, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0004".to_string()]);

    // Tampered binding beats expiry (gate 3 before gate 4).
    let mut tampered_and_expired = token.clone();
    tampered_and_expired.plan_digest = OTHER_DIGEST.into();
    tampered_and_expired.expiration = Some("2026-10-12T23:59:59Z".into());
    let err = permit
        .resume_run(
            &ckpt,
            &suspension,
            &tampered_and_expired,
            &m,
            "pd",
            "rev-1",
            NOW,
            None,
        )
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // Expiry beats staleness (gate 4 before gate 5): properly issued but
    // expired token, while the run state moved.
    let mut expired_subject = run;
    let attempt = NodeAttemptRecordV1 {
        attempt: 1,
        started_at: "t9".into(),
        completed_at: Some("t10".into()),
        outcome: OutcomeCategory::Completed,
        result_digest: "d".repeat(64),
    };
    expired_subject
        .apply_node_completion("a", attempt, "t10")
        .unwrap();
    let ckpt2 = expired_subject.export_checkpoint().unwrap();
    let expired = issue_resume_token(&suspension, "tok-x", Some("2026-10-12T23:59:59Z")).unwrap();
    let err = permit
        .resume_run(&ckpt2, &suspension, &expired, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0002".to_string()]);
}

#[test]
fn version_policy_is_uniform_at_create_issue_and_resume() {
    let (permit, m, run, suspension, _token) = fixture();
    // Issuance refuses to mint a token for a drifted suspension.
    let mut drifted = suspension.clone();
    drifted.workflow_version = "2.0.0".into();
    let err = issue_resume_token(&drifted, "tok-d", None).unwrap_err();
    assert!(
        err.to_string().contains("unsupported suspension version"),
        "{err}"
    );
    // And the resume ladder refuses the drifted document itself (gate 1).
    let ckpt = run.export_checkpoint().unwrap();
    let mut drifted_resume = suspension.clone();
    drifted_resume.schema_version = "2.0.0".into();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let err = permit
        .resume_run(&ckpt, &drifted_resume, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0004".to_string()]);
}
