//! E4/X05 (#162) Slice 1 — typed suspension + resume-token conformance.
//!
//! Proves the vendored-map-shaped `SuspensionPointV1` / `ResumeTokenV1`
//! contracts, the journaled suspension event, byte-deterministic
//! suspension/token digests for identical inputs, and the fail-closed
//! resume ladder: expired / stale / tampered / unsupported / mismatched
//! all refuse, and a valid token still passes through the existing #123
//! replay gate (a token never bypasses the run law).

use prometheos_lite::workflow::graph_resume::ReplayAuthorization;
use prometheos_lite::workflow::graph_state::{
    EdgeKind, GraphEdgeV1, GraphManifestV1, GraphNodeV1, GraphRunStateV1, NodeAttemptRecordV1,
    OutcomeCategory, RouteDecisionV1,
};
use prometheos_lite::workflow::graph_suspend::{
    ResumeCheckParams, ResumeWithTokenError, SuspendParams, issue_resume_token, resume_with_token,
    suspend_run,
};
use prometheos_lite::workflow::soma::canonical::try_canonical_digest;

const NOW: &str = "2026-10-12T00:00:00Z";
const OPS_DIGEST: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const PLAN_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const AUTH_DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";
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

fn suspend_params<'a>() -> SuspendParams<'a> {
    SuspendParams {
        id: "susp-1",
        workflow_version: "1.0.0",
        trigger: "human-review",
        revision: None,
        expiration: None,
        operation_id: "a",
        inputs_digest: OPS_DIGEST,
        plan_digest: PLAN_DIGEST,
        authority_snapshot_digest: AUTH_DIGEST,
        recorded_at: "t1",
    }
}

fn check_params<'a>(m: &'a GraphManifestV1) -> ResumeCheckParams<'a> {
    ResumeCheckParams {
        manifest: m,
        portable_digest: "pd",
        repo_revision_now: "rev-1",
        plan_digest_now: PLAN_DIGEST,
        authority_snapshot_digest_now: AUTH_DIGEST,
        now: NOW,
        replay: None,
    }
}

fn codes(err: &ResumeWithTokenError) -> Vec<String> {
    match err {
        ResumeWithTokenError::Refusal(diags) => diags.iter().map(|d| d.code.clone()).collect(),
        ResumeWithTokenError::Underlying(e) => vec![e.to_string()],
    }
}

#[test]
fn suspension_point_round_trips_with_vendored_shape() {
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_run(&mut run, suspend_params()).expect("suspend");
    assert_eq!(suspension.schema_version, "1.0.0");
    assert_eq!(suspension.id, "susp-1");
    assert_eq!(suspension.plan_digest, PLAN_DIGEST);
    assert_eq!(suspension.authority_snapshot_digest, AUTH_DIGEST);
    assert_eq!(suspension.resumability.operation_id, "a");
    assert_eq!(suspension.resumability.inputs_digest, OPS_DIGEST);
    assert_eq!(suspension.state_digest.len(), 64);
    // Vendored-shape wire check: camelCase, no extras.
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
    let round: prometheos_lite::workflow::graph_suspend::SuspensionPointV1 =
        serde_json::from_value(serde_json::to_value(&suspension).unwrap()).unwrap();
    assert_eq!(round, suspension);
    // Unknown members refuse (deny_unknown_fields).
    let mut v = serde_json::to_value(&suspension).unwrap();
    v.as_object_mut()
        .unwrap()
        .insert("mystery".into(), serde_json::Value::Null);
    let dj: Result<prometheos_lite::workflow::graph_suspend::SuspensionPointV1, serde_json::Error> =
        serde_json::from_value(v);
    assert!(dj.is_err());
}

#[test]
fn suspend_journals_evidence_and_reseals_state() {
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_run(&mut run, suspend_params()).expect("suspend");
    assert_eq!(run.evidence_refs.len(), 1);
    let ev = &run.evidence_refs[0];
    assert_eq!(ev.id, "suspension:susp-1");
    assert_eq!(ev.artifact_kind, "suspension-point");
    // The journaled event digest is the digest of the suspension document,
    // and its artifact digest is the pre-journal sealed state digest the
    // suspension binds (the resume gate reconstructs it by removing this
    // one journal entry).
    assert_eq!(ev.event_digest, suspension.digest().expect("digest"));
    assert_eq!(ev.artifact_digest, suspension.state_digest);
    // A checkpoint exported after suspension imports cleanly (digest chain intact).
    let ckpt = run.export_checkpoint().expect("export");
    let resumed = resume_run_mockless(&ckpt, &m);
    assert_eq!(resumed.evidence_refs.len(), 1);
}

fn resume_run_mockless(ckpt: &str, m: &GraphManifestV1) -> GraphRunStateV1 {
    prometheos_lite::workflow::graph_resume::resume_run(ckpt, m, "pd", "rev-1", None)
        .expect("resume after suspend")
        .state
}

#[test]
fn suspension_and_token_digests_are_byte_deterministic() {
    let m = manifest();
    let mut run_a = open_run(&m);
    let mut run_b = open_run(&m);
    let s_a = suspend_run(&mut run_a, suspend_params()).unwrap();
    let s_b = suspend_run(&mut run_b, suspend_params()).unwrap();
    assert_eq!(s_a.digest().unwrap(), s_b.digest().unwrap());
    assert_eq!(
        serde_json::to_string(&s_a).unwrap(),
        serde_json::to_string(&s_b).unwrap()
    );
    let t_a = issue_resume_token(&s_a, "tok-1", None).expect("issue");
    let t_b = issue_resume_token(&s_b, "tok-1", None).expect("issue");
    assert_eq!(t_a.binding_digest, t_b.binding_digest);
    assert_eq!(t_a.suspension_digest, t_b.suspension_digest);
}

#[test]
fn suspend_refuses_terminated_run() {
    let m = manifest();
    let mut run = open_run(&m);
    prometheos_lite::workflow::graph_resume::cancel_run(&mut run, "stop", "t2").unwrap();
    let err = suspend_run(&mut run, suspend_params()).unwrap_err();
    assert!(err.to_string().contains("terminated"), "{err}");
}

type SuspensionPoint = prometheos_lite::workflow::graph_suspend::SuspensionPointV1;
type ResumeToken = prometheos_lite::workflow::graph_suspend::ResumeTokenV1;

fn suspended_fixture() -> (
    GraphManifestV1,
    GraphRunStateV1,
    SuspensionPoint,
    ResumeToken,
) {
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    (m, run, suspension, token)
}

#[test]
fn resume_with_valid_token_succeeds_and_preserves_frontier() {
    let (m, run, suspension, token) = suspended_fixture();
    let ckpt = run.export_checkpoint().unwrap();
    let resumed = resume_with_token(&ckpt, &suspension, &token, &check_params(&m))
        .expect("valid token resumes");
    assert_eq!(resumed.state.frontier, vec!["a".to_string()]);
}

#[test]
fn resume_refuses_tampered_token_binding() {
    let (m, run, suspension, token) = suspended_fixture();
    let mut forged = token;
    forged.plan_digest = OTHER_DIGEST.into(); // mutate without re-binding
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &forged, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn resume_refuses_token_pointing_at_other_suspension() {
    let (m, run, suspension, _token) = suspended_fixture();
    // Mint a real token over a *different* suspension.
    let mut other_run = open_run(&m);
    let other_susp = suspend_run(
        &mut other_run,
        SuspendParams {
            id: "susp-other",
            operation_id: "b",
            ..suspend_params()
        },
    )
    .unwrap();
    let foreign = issue_resume_token(&other_susp, "tok-2", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &foreign, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);
}

#[test]
fn resume_refuses_expired_token() {
    let (m, run, suspension, _token) = suspended_fixture();
    let expired = issue_resume_token(&suspension, "tok-x", Some("2026-10-11T23:59:59Z")).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &expired, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0002".to_string()]);
    // Same token before its expiration is accepted.
    let ok = resume_with_token(
        &ckpt,
        &suspension,
        &expired,
        &ResumeCheckParams {
            now: "2026-10-11T00:00:00Z",
            ..check_params(&m)
        },
    );
    assert!(
        ok.is_ok(),
        "{}",
        ok.err().map(|e| codes(&e).join(",")).unwrap_or_default()
    );
}

#[test]
fn resume_refuses_unsupported_versions() {
    let (m, run, suspension, token) = suspended_fixture();
    // A forged schemaVersion refuses as unsupported before the binding
    // recompute (the version gate runs first in the ladder).
    let mut bad_token = token;
    bad_token.schema_version = "9.9.9".into();
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &bad_token, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0004".to_string()]);
}

#[test]
fn resume_refuses_stale_plan_digest() {
    let (m, run, suspension, token) = suspended_fixture();
    let ckpt = run.export_checkpoint().unwrap();
    // Reviewed plan identity at resume time no longer matches the suspension.
    let err = resume_with_token(
        &ckpt,
        &suspension,
        &token,
        &ResumeCheckParams {
            plan_digest_now: OTHER_DIGEST,
            ..check_params(&m)
        },
    )
    .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_mismatched_authority_snapshot() {
    let (m, run, suspension, token) = suspended_fixture();
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(
        &ckpt,
        &suspension,
        &token,
        &ResumeCheckParams {
            authority_snapshot_digest_now: OTHER_DIGEST,
            ..check_params(&m)
        },
    )
    .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_checkpoint_that_moved_since_suspension() {
    let (m, run, suspension, token) = suspended_fixture();
    // Mutate the run AFTER the suspension was bound: the checkpoint digest
    // no longer matches the suspension's stateDigest.
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
    let err = resume_with_token(&ckpt, &suspension, &token, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn valid_token_still_requires_replay_authorization() {
    // Manifest variant with a cycle edge b -> a (same shape as graph_resume tests).
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
            // Cycle edge: a completed node re-entering the frontier for the
            // replay gate.
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

    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();

    // Valid token but no replay authorization -> underlying replay gate refuses.
    let err = resume_with_token(&ckpt, &suspension, &token, &check_params(&m)).unwrap_err();
    match err {
        ResumeWithTokenError::Underlying(e) => {
            assert!(e.to_string().contains("replay"), "{e}")
        }
        ResumeWithTokenError::Refusal(d) => {
            panic!("token was valid; expected replay gate, got {d:?}")
        }
    }
    // Valid token + authorization -> resumes.
    let ok = resume_with_token(
        &ckpt,
        &suspension,
        &token,
        &ResumeCheckParams {
            replay: Some(ReplayAuthorization {
                reason: "replay a".into(),
                authorized_by: "op".into(),
            }),
            ..check_params(&m)
        },
    )
    .expect("authorized replay resumes");
    assert!(ok.replay_authorized);
}

#[test]
fn suspension_digest_is_the_canonical_digest_of_the_document() {
    let m = manifest();
    let mut run = open_run(&m);
    let s = suspend_run(&mut run, suspend_params()).unwrap();
    let value = serde_json::to_value(&s).unwrap();
    let digest = try_canonical_digest(&value).unwrap();
    assert_eq!(s.digest().unwrap(), digest);
}

// --- Review-repair negative controls (P1 x4) ---

#[test]
fn expiration_is_bound_into_the_token_binding() {
    let (m, run, suspension, _token) = suspended_fixture();
    let issued = issue_resume_token(&suspension, "tok-e", Some("2026-10-13T00:00:00Z")).unwrap();
    let ckpt = run.export_checkpoint().unwrap();

    // Removing expiration must invalidate the binding (absence is explicit).
    let mut removed = issued.clone();
    removed.expiration = None;
    let err = resume_with_token(&ckpt, &suspension, &removed, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // Extending expiration must invalidate the binding.
    let mut extended = issued;
    extended.expiration = Some("2027-01-01T00:00:00Z".into());
    let err = resume_with_token(&ckpt, &suspension, &extended, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // The intact token still passes at an in-window clock reading.
    let issued = issue_resume_token(&suspension, "tok-e", Some("2026-10-13T00:00:00Z")).unwrap();
    assert!(resume_with_token(&ckpt, &suspension, &issued, &check_params(&m)).is_ok(),);
}

#[test]
fn resume_refuses_pre_suspension_checkpoint() {
    let m = manifest();
    let mut run = open_run(&m);
    // Checkpoint exported BEFORE the suspension exists.
    let pre_ckpt = run.export_checkpoint().unwrap();
    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let err = resume_with_token(&pre_ckpt, &suspension, &token, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_tampered_suspension_evidence_entry() {
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    // Corrupt the journaled event digest and reseal the state honestly.
    run.evidence_refs[0].event_digest = OTHER_DIGEST.into();
    run.content_digest = Some(run.compute_digest());
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &token, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn resume_refuses_duplicate_suspension_evidence_entries() {
    let m = manifest();
    let mut run = open_run(&m);
    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    // Duplicate the journal entry and reseal honestly.
    let dup = run.evidence_refs[0].clone();
    run.evidence_refs.push(dup);
    run.content_digest = Some(run.compute_digest());
    let ckpt = run.export_checkpoint().unwrap();
    let err = resume_with_token(&ckpt, &suspension, &token, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
}

#[test]
fn suspend_refuses_malformed_digest_fields() {
    let m = manifest();
    let mut run = open_run(&m);
    for (plan, auth, inputs, label) in [
        ("xyz", AUTH_DIGEST, OPS_DIGEST, "plan"),
        (PLAN_DIGEST, "not-hex", OPS_DIGEST, "authority"),
        (PLAN_DIGEST, AUTH_DIGEST, "abc", "inputs"),
        (&"F".repeat(64), AUTH_DIGEST, OPS_DIGEST, "uppercase plan"),
        (&PLAN_DIGEST[..63], AUTH_DIGEST, OPS_DIGEST, "short plan"),
    ] {
        let err = suspend_run(
            &mut run,
            SuspendParams {
                plan_digest: plan,
                authority_snapshot_digest: auth,
                inputs_digest: inputs,
                ..suspend_params()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("64-hex"), "{label}: {err}");
    }
}

#[test]
fn create_then_resume_version_policy_is_consistent() {
    let m = manifest();
    let mut run = open_run(&m);
    // A version the resume ladder cannot support must refuse at CREATION.
    let err = suspend_run(
        &mut run,
        SuspendParams {
            workflow_version: "2.0.0",
            ..suspend_params()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("workflow_version"), "{err}");
    // The supported version creates and resumes end to end.
    let suspension = suspend_run(&mut run, suspend_params()).unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    assert!(resume_with_token(&ckpt, &suspension, &token, &check_params(&m)).is_ok());
}

#[test]
fn declared_failure_precedence_holds_for_overlapping_invalidity() {
    let (m, run, suspension, token) = suspended_fixture();
    let ckpt = run.export_checkpoint().unwrap();

    // Unsupported version beats tampered binding (gate 1 before gate 3).
    let mut both = token.clone();
    both.schema_version = "9.9.9".into();
    both.plan_digest = OTHER_DIGEST.into();
    let err = resume_with_token(&ckpt, &suspension, &both, &check_params(&m)).unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0004".to_string()]);

    // Tampered binding beats expiry (gate 3 before gate 4).
    let mut tampered_and_expired = token.clone();
    tampered_and_expired.plan_digest = OTHER_DIGEST.into();
    tampered_and_expired.expiration = Some("2026-10-11T23:59:59Z".into());
    let err = resume_with_token(&ckpt, &suspension, &tampered_and_expired, &check_params(&m))
        .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0003".to_string()]);

    // Expiry beats staleness (gate 4 before gate 5): a properly issued but
    // expired token against a stale authority snapshot.
    let expired = issue_resume_token(&suspension, "tok-x", Some("2026-10-11T23:59:59Z")).unwrap();
    let err = resume_with_token(
        &ckpt,
        &suspension,
        &expired,
        &ResumeCheckParams {
            authority_snapshot_digest_now: OTHER_DIGEST,
            ..check_params(&m)
        },
    )
    .unwrap_err();
    assert_eq!(codes(&err), vec!["SOMA-RES-0002".to_string()]);
}
