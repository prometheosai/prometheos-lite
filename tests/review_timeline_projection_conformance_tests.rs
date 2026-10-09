//! Slice 3 — review-report + evidence-timeline projection conformance.
//!
//! Every test name maps visibly onto a row of the approved §10 acceptance
//! matrix (rows 1-24), plus the digest-domain, disclosure, audit, and
//! completeness regressions required by the approved contract.

use prometheos_lite::db::repository::work_context_events::ProvenanceState;
use prometheos_lite::harness::review::{
    QualityGrade, ReviewIssue, ReviewIssueType, ReviewQualityScore, ReviewReport, ReviewSeverity,
    ReviewSummary,
};
use prometheos_lite::work::soma_projection::{RunKey, RunKeyKind};
use prometheos_lite::workflow::graph_gates::{HumanDecisionRecordV1, HumanVerdict, ReviewChannel};
use prometheos_lite::workflow::projection::{
    ProjectionPageMeta, REVIEW_SCHEMA_VERSION, ReviewDisclosurePolicy, ReviewFacts,
    ReviewProjectionPayload, ReviewScope, RunKeyView, TIMELINE_SCHEMA_VERSION,
    TimelineDisclosurePolicy, TimelineProjectionSource, VersionedProjectionEnvelope,
    project_canonical_json, render_review_projection, render_timeline_projection,
    verify_canonical_projection_bytes, verify_review_against_source,
    verify_review_projection_bytes, verify_timeline_against_source,
    verify_timeline_projection_bytes,
};
use prometheos_lite::workflow::soma::contracts::{EvidenceReference, WorkflowDefinition};
use prometheos_lite::workflow::soma::event::{WorkEvent, WorkEventBatch};
use prometheos_lite::workflow::soma::types::Hex64;

use std::collections::HashMap;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn base_wf() -> WorkflowDefinition {
    serde_json::from_str(&fixture("fixtures/valid/wf-valid-base.json")).expect("fixture parses")
}

fn nested_wf() -> WorkflowDefinition {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/slice2/wf-nested.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("wf-nested reads"))
        .expect("wf-nested parses")
}

fn hexc(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn synthetic_report() -> ReviewReport {
    ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Security,
                severity: ReviewSeverity::High,
                file: Some("b.rs".to_string()),
                line: Some(7),
                message: "missing bound check".to_string(),
                suggestion: Some("add guard".to_string()),
                rule_id: "SEC-1".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::Critical,
                file: Some("a.rs".to_string()),
                line: Some(3),
                message: "null deref".to_string(),
                suggestion: None,
                rule_id: "BUG-2".to_string(),
            },
        ],
        summary: ReviewSummary {
            total_issues: 2,
            files_with_issues: 2,
            files_reviewed: 3,
            by_type: HashMap::from([
                (ReviewIssueType::Security, 1usize),
                (ReviewIssueType::Bug, 1usize),
            ]),
            by_severity: HashMap::from([
                (ReviewSeverity::High, 1usize),
                (ReviewSeverity::Critical, 1usize),
            ]),
        },
        passed: false,
        critical_count: 1,
        high_count: 1,
        ast_analysis_enabled: true,
        review_performed: true,
        quality_score: ReviewQualityScore {
            overall_score: 0,
            security_score: 0,
            code_quality_score: 0,
            maintainability_score: 0,
            documentation_score: 0,
            performance_score: 0,
            grade: QualityGrade::F,
            confidence: 0,
        },
        quality_metrics: Default::default(),
    }
}

fn gate(node: &str, verdict: HumanVerdict, by: &str, basis: &str) -> HumanDecisionRecordV1 {
    HumanDecisionRecordV1::author(
        node,
        verdict,
        by,
        ReviewChannel::CliInteractive,
        basis,
        "reviewed",
        "2026-10-05T00:00:00Z",
    )
    .expect("author succeeds")
}

fn approved_gate() -> Vec<HumanDecisionRecordV1> {
    vec![gate(
        "intake.review",
        HumanVerdict::Approved,
        "alice",
        &hexc('b'),
    )]
}

/// Evidence references with both event and artifact digests; the artifact
/// digest (`artifactDigest`) is what the gate `basisEvidenceDigest` binds.
fn gate_refs() -> Vec<EvidenceReference> {
    // artifactDigest == gate basis (`hexc('b')`); eventDigest differs (`'c'`).
    vec![evref("r1", &hexc('c'), &hexc('b'), "2026-10-05T00:00:00Z")]
}

fn evref(
    id: &str,
    event_digest: &str,
    artifact_digest: &str,
    produced_at: &str,
) -> EvidenceReference {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "eventDigest": event_digest,
        "artifactDigest": artifact_digest,
        "artifactKind": "review-report",
        "producedBy": "harness",
        "producedAt": produced_at,
    }))
    .expect("evidence reference literal")
}

fn facts<'a>(
    report: Option<&'a ReviewReport>,
    gates: &'a [HumanDecisionRecordV1],
    refs: &'a [EvidenceReference],
) -> ReviewFacts<'a> {
    ReviewFacts {
        report,
        gates,
        evidence_references: refs,
        scope: ReviewScope::default(),
    }
}

fn review_all_policy() -> ReviewDisclosurePolicy {
    ReviewDisclosurePolicy {
        authorized_targets: [
            "findings",
            "files",
            "lines",
            "messages",
            "rules",
            "predicates",
            "principals",
            "evidenceReferences",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        count_authorization: [
            "totalIssues",
            "byType",
            "bySeverity",
            "filesReviewed",
            "filesWithIssues",
            "passed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    }
}

fn timeline_all_policy() -> TimelineDisclosurePolicy {
    TimelineDisclosurePolicy {
        authorized_targets: ["payloadDetails", "actorIdentity", "referenceDigest"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        count_authorization: vec!["events".to_string()],
    }
}

fn mk_event(id: &str, sequence: u64, correlation: &str, timestamp: &str) -> WorkEvent {
    mk_event_parented(id, sequence, correlation, timestamp, vec![])
}

fn mk_event_parented(
    id: &str,
    sequence: u64,
    correlation: &str,
    timestamp: &str,
    parents: Vec<String>,
) -> WorkEvent {
    let mut event: WorkEvent = serde_json::from_value(serde_json::json!({
        "schemaVersion": "1.1.0",
        "version": "1.1.0",
        "id": id,
        "eventType": "status",
        "actor": {"kind": "harness", "identity": "actor-1"},
        "authority": {"executionClass": "deterministic", "mutation": "none"},
        "effectiveAuthority": {"executionClass": "deterministic", "mutation": "none"},
        "sequence": sequence,
        "timestamp": timestamp,
        "idempotencyKey": format!("ik-{id}"),
        "correlationId": correlation,
        "repoRevision": hexc('c'),
        "compatibility": {"schemaVersion": "1.1", "minReaderVersion": "1.0"},
        "semanticDigest": hexc('0'),
        "parents": parents,
        "payload": {},
        "replay": false,
        "conflict": false
    }))
    .expect("event literal");
    let digest = event.computed_semantic_digest().expect("digest computes");
    event.semantic_digest = Hex64::parse(&digest).expect("valid digest");
    event
}

fn batch(run_id: &str, events: Vec<WorkEvent>) -> WorkEventBatch {
    WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: run_id.to_string(),
        events,
        compatibility: None,
    }
}

fn run_key(id: &str) -> RunKey {
    RunKey {
        kind: RunKeyKind::WorkRun,
        id: id.to_string(),
    }
}

fn review_value_env(
    env: &VersionedProjectionEnvelope<ReviewProjectionPayload>,
) -> VersionedProjectionEnvelope<serde_json::Value> {
    VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("payload value"),
    }
}

// ---------------------------------------------------------------------------
// Row 1 / Row 2 — determinism
// ---------------------------------------------------------------------------

#[test]
fn review_render_is_deterministic_bytes() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    let a = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render a");
    let b = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render b");
    assert_eq!(
        a.canonical_bytes().expect("a bytes"),
        b.canonical_bytes().expect("b bytes")
    );
}

#[test]
fn timeline_render_is_deterministic_bytes() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let a = render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("a");
    let b2 = render_timeline_projection(&source, Some(&timeline_all_policy()), &wf)
        .expect("2026-10-05T00:00:00Z");
    assert_eq!(
        a.canonical_bytes().expect("a bytes"),
        b2.canonical_bytes().expect("b bytes")
    );
}

// ---------------------------------------------------------------------------
// Row 3 — input order perturbation does not change bytes
// ---------------------------------------------------------------------------

#[test]
fn canonical_render_ignores_input_key_order() {
    let wf = base_wf();
    let mut report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f1 = facts(Some(&report), &gates, &refs);
    let a = render_review_projection(&wf, &f1, Some(&review_all_policy())).expect("render a");
    report.issues.reverse();
    let f2 = facts(Some(&report), &gates, &refs);
    let b = render_review_projection(&wf, &f2, Some(&review_all_policy())).expect("render b");
    assert_eq!(
        a.canonical_bytes().expect("a bytes"),
        b.canonical_bytes().expect("b bytes")
    );
}

// ---------------------------------------------------------------------------
// Row 4 — timeline ordering
// ---------------------------------------------------------------------------

#[test]
fn timeline_orders_by_sequence_then_digest_not_timestamp() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![
            mk_event("e2", 9, "corr-1", "2026-10-05T00:00:00Z"),
            mk_event("e1", 5, "corr-1", "2026-10-05T00:00:00Z"),
        ],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("renders");
    assert_eq!(env.payload.events.len(), 2);
    assert_eq!(env.payload.events[0].event_id, "e1");
    assert_eq!(env.payload.events[1].event_id, "e2");
    assert_eq!(env.payload.events[0].sequence, 5);
}

// ---------------------------------------------------------------------------
// Row 5 / item 1 — digest domains
// ---------------------------------------------------------------------------

#[test]
fn digest_domain_separation_is_respected() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    let base = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");

    // Policy change alters disclosurePolicyDigest but not reportReferenceDigest.
    let mut narrowed = review_all_policy();
    narrowed.count_authorization.clear();
    let narrowed_env = render_review_projection(&wf, &f, Some(&narrowed)).expect("render narrowed");
    assert_ne!(
        base.payload.disclosure_policy_digest,
        narrowed_env.payload.disclosure_policy_digest
    );
    assert_eq!(
        base.payload.report_reference_digest,
        narrowed_env.payload.report_reference_digest
    );

    // Report change alters the rendered payload projectionDigest.
    let mut other = synthetic_report();
    other.issues.push(ReviewIssue {
        issue_type: ReviewIssueType::Style,
        severity: ReviewSeverity::Low,
        file: Some("c.rs".to_string()),
        line: Some(1),
        message: "style".to_string(),
        suggestion: None,
        rule_id: "STY-1".to_string(),
    });
    let f2 = facts(Some(&other), &gates, &refs);
    let other_env = render_review_projection(&wf, &f2, Some(&review_all_policy())).expect("render");
    assert_ne!(base.projection_digest, other_env.projection_digest);
}

#[test]
fn policy_digest_shape_check() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    for d in [
        &env.payload.disclosure_policy_digest,
        &env.payload.report_reference_digest,
    ] {
        assert_eq!(d.len(), 64);
        assert!(
            d.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }
}

#[test]
fn event_stream_digest_shape_check() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    assert_eq!(env.payload.event_stream_digest.len(), 64);
}

// ---------------------------------------------------------------------------
// Row 6 / Row 15 — structural verifiers
// ---------------------------------------------------------------------------

#[test]
fn verify_review_projection_bytes_rejects_tampering() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    let bytes = env.canonical_bytes().expect("bytes");
    assert!(verify_review_projection_bytes(&bytes).is_ok());
    let mut tampered = bytes.clone();
    let idx = tampered.len() / 2;
    tampered[idx] ^= 0x01;
    assert!(verify_review_projection_bytes(&tampered).is_err());
}

#[test]
fn verify_timeline_projection_bytes_rejects_tampering() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    let bytes = env.canonical_bytes().expect("bytes");
    assert!(verify_timeline_projection_bytes(&bytes).is_ok());
    let mut tampered = bytes.clone();
    let idx = tampered.len() / 2;
    tampered[idx] ^= 0x01;
    assert!(verify_timeline_projection_bytes(&tampered).is_err());
}

// ---------------------------------------------------------------------------
// Row 7 — unsupported versions
// ---------------------------------------------------------------------------

fn mutate_envelope_field(bytes: &[u8], key: &str, value: serde_json::Value) -> Vec<u8> {
    let mut v: serde_json::Value = serde_json::from_slice(bytes).expect("env parses");
    v[key] = value;
    serde_json::to_vec(&v).expect("re-serialize")
}

#[test]
fn unsupported_projection_version_fails() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    let bytes = env.canonical_bytes().expect("bytes");
    let forged = mutate_envelope_field(&bytes, "projectionVersion", "projection.v999".into());
    assert!(verify_review_projection_bytes(&forged).is_err());
}

#[test]
fn unsupported_envelope_schema_version_fails() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    let bytes = env.canonical_bytes().expect("bytes");
    let forged = mutate_envelope_field(&bytes, "schemaVersion", "9.9.9".into());
    assert!(verify_review_projection_bytes(&forged).is_err());
}

#[test]
fn unsupported_review_schema_version_fails() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    let mut v: serde_json::Value =
        serde_json::from_slice(&env.canonical_bytes().expect("bytes")).expect("parses");
    v["payload"]["reviewSchemaVersion"] = "lite.review-report.v2".into();
    assert!(
        verify_review_projection_bytes(&serde_json::to_vec(&v).expect("ser")).is_err(),
        "an out-of-allow-list reviewSchemaVersion must fail closed"
    );
}

#[test]
fn unsupported_timeline_schema_version_fails() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("render");
    let mut v: serde_json::Value =
        serde_json::from_slice(&env.canonical_bytes().expect("bytes")).expect("parses");
    v["payload"]["timelineSchemaVersion"] = "lite.evidence-timeline.v9".into();
    assert!(verify_timeline_projection_bytes(&serde_json::to_vec(&v).expect("ser")).is_err());
}

// ---------------------------------------------------------------------------
// Row 8 — unknown fields / duplicate keys fail closed
// ---------------------------------------------------------------------------

#[test]
fn parse_envelope_bytes_rejects_duplicate_and_unknown_keys() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    let bytes = env.canonical_bytes().expect("bytes");

    // Unknown top-level envelope key.
    let unknown = mutate_envelope_field(&bytes, "bogus", serde_json::json!(1));
    assert!(verify_review_projection_bytes(&unknown).is_err());

    // Duplicate key at the byte level.
    let text = String::from_utf8(bytes).expect("utf8");
    let dup = text.replacen(
        "\"projectionVersion\"",
        "\"projectionVersion\":\"projection.v1\",\"projectionVersion\"",
        1,
    );
    assert!(verify_review_projection_bytes(dup.as_bytes()).is_err());
}

// ---------------------------------------------------------------------------
// Row 9 / Row 22 — authority non-expansion, no invented verdicts
// ---------------------------------------------------------------------------

#[test]
fn projection_cannot_widen_authority_or_invent_verdicts() {
    let wf = base_wf();
    let report = synthetic_report();
    let empty_gates: [HumanDecisionRecordV1; 0] = [];
    let refs = gate_refs();
    let f = facts(Some(&report), &empty_gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    // No gate record => unavailable disposition, never an implied approval.
    assert_eq!(env.payload.disposition.status, "unavailable");
    assert!(!env.payload.disposition.supported_by_evidence);
}

#[test]
fn unavailable_verdict_is_distinguishable_from_approved() {
    let wf = base_wf();
    let report = synthetic_report();
    let refs = gate_refs();

    let none: [HumanDecisionRecordV1; 0] = [];
    let f_none = facts(Some(&report), &none, &refs);
    let unavail = render_review_projection(&wf, &f_none, None).expect("render");
    assert_eq!(unavail.payload.disposition.status, "unavailable");

    let gates = approved_gate();
    let f_appr = facts(Some(&report), &gates, &refs);
    let approved = render_review_projection(&wf, &f_appr, None).expect("render");
    assert_eq!(approved.payload.disposition.status, "approve");
    assert_ne!(
        unavailable_status(&unavail),
        approved.payload.disposition.status
    );
}

fn unavailable_status(env: &VersionedProjectionEnvelope<ReviewProjectionPayload>) -> String {
    env.payload.disposition.status.clone()
}

// ---------------------------------------------------------------------------
// Row 10 / Row 11 — disclosure: no leak; withheld vs unavailable vs outOfScope
// ---------------------------------------------------------------------------

#[test]
fn policy_drawn_report_carries_no_withheld_issue_content() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let policy = ReviewDisclosurePolicy {
        authorized_targets: vec!["findings".to_string()],
        count_authorization: vec![],
    };
    let env = render_review_projection(&wf, &f, Some(&policy)).expect("render");
    assert_eq!(env.payload.issues.len(), 2);
    for issue in &env.payload.issues {
        assert!(issue.file.is_none(), "file must be withheld");
        assert!(issue.line.is_none(), "line must be withheld");
        assert!(issue.message.is_empty(), "message must be withheld");
        assert!(issue.rule_id.is_empty(), "ruleId must be withheld");
    }
    let bytes = String::from_utf8(env.canonical_bytes().expect("bytes")).expect("utf8");
    assert!(!bytes.contains("a.rs"));
    assert!(!bytes.contains("null deref"));
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "files" && o.category == "withheld")
    );
}

#[test]
fn timeline_payload_no_leak_of_withheld_payload_details() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-secret", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("render");
    let view = &env.payload.events[0];
    assert!(view.actor_identity.is_empty());
    assert!(view.event_type.is_empty());
    assert!(view.correlation_id.is_empty());
    let bytes = String::from_utf8(env.canonical_bytes().expect("bytes")).expect("utf8");
    assert!(!bytes.contains("corr-secret"));
    assert!(!bytes.contains("actor-1"));
}

#[test]
fn omissions_distinguish_withheld_unavailable_outofscope() {
    let wf = base_wf();
    let gates = approved_gate();
    let refs = gate_refs();
    // No report => unavailable (not withheld), for the issues section.
    let f = facts(None, &gates, &refs);
    let env = render_review_projection(&wf, &f, None).expect("render");
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "issues" && o.category == "unavailable")
    );

    // Mixed correlations in a timeline => outOfScope scope omission.
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![
            mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z"),
            mk_event("e2", 2, "corr-2", "2026-10-05T00:00:01Z"),
        ],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let t = render_timeline_projection(&source, None, &wf).expect("render");
    assert!(
        t.payload
            .omissions
            .iter()
            .any(|o| o.category == "outOfScope")
    );
}

/// §6.2 / item 4 — timeline event count independently gated.
#[test]
fn timeline_event_count_is_authorized() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "c", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let policy = TimelineDisclosurePolicy {
        authorized_targets: ["payloadDetails", "actorIdentity", "referenceDigest"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        count_authorization: vec!["events".to_string()],
    };
    let env = render_timeline_projection(&source, Some(&policy), &wf).expect("render");
    assert_eq!(env.payload.event_count, Some(1));
    assert!(
        !env.payload
            .omissions
            .iter()
            .any(|o| o.section == "events" && o.category == "withheld")
    );
}

#[test]
fn timeline_event_count_is_withheld_when_unauthorized() {
    let wf = base_wf();
    let rk = run_key("run-2");
    let b = batch(
        "run-2",
        vec![
            mk_event("e1", 1, "c", "2026-10-05T00:00:00Z"),
            mk_event("e2", 2, "c", "2026-10-05T00:00:01Z"),
        ],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let policy = TimelineDisclosurePolicy {
        authorized_targets: ["payloadDetails", "actorIdentity", "referenceDigest"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        count_authorization: vec![],
    };
    let env = render_timeline_projection(&source, Some(&policy), &wf).expect("render");
    assert!(env.payload.event_count.is_none());
    assert!(env.payload.omissions.iter().any(|o| o.section == "events"
        && o.category == "withheld"
        && o.reason == "disclosurePolicy"));
}

#[test]
fn timeline_unknown_count_target_fails_closed() {
    let wf = base_wf();
    let rk = run_key("run-3");
    let b = batch(
        "run-3",
        vec![mk_event("e1", 1, "c", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let policy = TimelineDisclosurePolicy {
        authorized_targets: ["payloadDetails", "actorIdentity", "referenceDigest"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        count_authorization: vec!["eventCount".to_string()],
    };
    let err = render_timeline_projection(&source, Some(&policy), &wf).expect_err("unknown target");
    assert!(err.iter().any(|d| d.code == "PROJ-0003"), "{err:?}");
}

/// §6.2 / item 3 — withheld `referenceDigest` must be explicit null, never a
/// zero digest that could be mistaken for a real value.
#[test]
fn withheld_reference_digest_is_null_not_zero_digest() {
    let wf = base_wf();
    let rk = run_key("run-4");
    let b = batch(
        "run-4",
        vec![mk_event("e1", 1, "c", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let policy = TimelineDisclosurePolicy {
        authorized_targets: ["payloadDetails", "actorIdentity"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        count_authorization: vec!["events".to_string()],
    };
    let env = render_timeline_projection(&source, Some(&policy), &wf).expect("render");
    assert!(env.payload.event_count == Some(1));
    let event = &env.payload.events[0];
    assert!(
        event.semantic_digest.is_none(),
        "semanticDigest must be null when withheld"
    );
    let bytes = String::from_utf8(env.canonical_bytes().expect("bytes")).expect("utf8");
    assert!(
        bytes.contains("\"semanticDigest\":null"),
        "must contain explicit null"
    );
    assert!(
        !bytes.contains("0000000000000000000000000000000000000000000000000000000000000000"),
        "must not contain zero digest"
    );
    // Structural byte verifier still accepts the envelope.
    assert!(verify_timeline_projection_bytes(&env.canonical_bytes().expect("bytes")).is_ok());
}

#[test]
fn withheld_counts_are_not_exposed() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let policy = ReviewDisclosurePolicy {
        authorized_targets: vec!["findings".to_string()],
        count_authorization: vec![],
    };
    let env = render_review_projection(&wf, &f, Some(&policy)).expect("render");
    assert!(env.payload.summary.total_issues.is_none());
    assert!(env.payload.summary.by_type.is_empty());
    assert!(env.payload.summary.passed.is_none());
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "summary" && o.category == "withheld")
    );
}

// ---------------------------------------------------------------------------
// Row 12 — private/composite boundaries
// ---------------------------------------------------------------------------

/// Row 12 (revised) — filenames are never composite/node identities; without
/// an authoritative issue-to-boundary binding, findings render verbatim when
/// `files` is authorized, and no composite parent is claimed.
#[test]
fn filenames_cannot_masquerade_as_composite_identities() {
    let wf = nested_wf(); // composite id = "flow"
    let mut report = synthetic_report();
    // A path that looks nested under a composite, plus one exactly named like it.
    report.issues[0].file = Some("flow/src/main.rs".to_string());
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    // files + findings authorized (no boundary claim expected); the file renders verbatim.
    let policy = ReviewDisclosurePolicy {
        authorized_targets: vec!["findings".to_string(), "files".to_string()],
        count_authorization: vec![],
    };
    let env = render_review_projection(&wf, &f, Some(&policy)).expect("render");
    assert!(
        env.payload
            .issues
            .iter()
            .any(|i| i.file.as_deref() == Some("flow/src/main.rs"))
    );
    // No fabricated parent-composite substitution (old `generalize_file` removed).
    assert!(
        !env.payload
            .issues
            .iter()
            .any(|i| i.file.as_deref().is_some_and(|f| f.contains(":flow")))
    );
    // No `outOfScope/scopeExcluded` omission fabricated from path matching.
    assert!(!env.payload.omissions.iter().any(|o| o.section == "files"
        && o.category == "outOfScope"
        && o.reason == "scopeExcluded"));
}

// ---------------------------------------------------------------------------
// Row 13 — evidence references deterministic
// ---------------------------------------------------------------------------

#[test]
fn review_findings_evidence_references_sorted_and_stable() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let unsorted = [
        evref("r2", &hexc('d'), &hexc('d'), "2026-10-05T00:00:02Z"),
        evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z"),
    ];
    let f = facts(Some(&report), &gates, &unsorted);
    let a = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render a");
    let b = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render b");
    assert_eq!(a.payload.evidence_references, b.payload.evidence_references);
    let produced: Vec<&str> = a
        .payload
        .evidence_references
        .iter()
        .map(|r| r.produced_at.as_deref().unwrap_or(""))
        .collect();
    let mut sorted = produced.clone();
    sorted.sort();
    assert_eq!(produced, sorted);
}

// ---------------------------------------------------------------------------
// Row 14 — dangling evidence reference fails closed
// ---------------------------------------------------------------------------

#[test]
fn review_against_source_fails_on_dangling_evidence_reference() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = vec![gate(
        "intake.review",
        HumanVerdict::Approved,
        "alice",
        &hexc('e'),
    )];
    let refs = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    let err = render_review_projection(&wf, &f, Some(&review_all_policy()))
        .expect_err("dangling gate basis must fail closed");
    assert!(err.iter().any(|d| d.code == "SOMA-CMP-0004"), "{err:?}");
}

/// Row 14 — gate basis binds to ARTIFACT digest, not event digest.
#[test]
fn gate_basis_binds_to_artifact_digest() {
    let wf = base_wf();
    let gates = approved_gate();
    // artifactDigest == gate basis (`b`); eventDigest differs (`c`).
    let refs = [evref("r1", &hexc('c'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let report = synthetic_report();
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render ok");
    assert!(env.payload.disposition.supported_by_evidence);
}

/// A reference whose eventDigest matches the gate basis but whose
/// artifactDigest does NOT must still fail closed (SOMA-CMP-0004).
#[test]
fn gate_basis_matching_event_digest_only_fails_closed() {
    let wf = base_wf();
    let gates = vec![gate(
        "intake.review",
        HumanVerdict::Approved,
        "alice",
        &hexc('b'),
    )];
    // eventDigest == `b` (would match if resolved wrongly); artifactDigest == `a` (does not match).
    let refs = [evref(
        "r_bad",
        &hexc('b'),
        &hexc('a'),
        "2026-10-05T00:00:00Z",
    )];
    let report = synthetic_report();
    let f = facts(Some(&report), &gates, &refs);
    let err = render_review_projection(&wf, &f, Some(&review_all_policy())).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "SOMA-CMP-0004"), "{err:?}");
}

// ---------------------------------------------------------------------------
// Row 16 — against-source recomputation pinned
// ---------------------------------------------------------------------------

#[test]
fn verify_review_against_source_accepts_self_consistent() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    let v = review_value_env(&env);
    verify_review_against_source(&v, &wf, &f, Some(&review_all_policy())).expect("self-consistent");
}

#[test]
fn verify_timeline_against_source_accepts_self_consistent() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    let v = VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("value"),
    };
    verify_timeline_against_source(&v, &source, Some(&timeline_all_policy()), &wf)
        .expect("self-consistent");
}

// ---------------------------------------------------------------------------
// Row 17 — no edit/mutation surface in the projection API
// ---------------------------------------------------------------------------

#[test]
fn projection_public_api_has_no_edit_surface() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/workflow/projection");
    let mut offenders: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).expect("projection dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("pub fn ") || trimmed.starts_with("pub async fn ") {
                let lower = trimmed.to_ascii_lowercase();
                if ["mutate", "save", "persist", "apply_to_ast", "write_ast"]
                    .iter()
                    .any(|bad| lower.contains(bad))
                {
                    offenders.push(trimmed.to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "projection API exposes edit-like functions: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// Row 18 — existing Slice-1/Slice-2 projection paths remain green
// ---------------------------------------------------------------------------

#[test]
fn existing_slice1_projection_path_remains_green() {
    let wf = base_wf();
    let env = project_canonical_json(&wf).expect("canonical projects");
    let bytes = env.canonical_bytes().expect("bytes");
    verify_canonical_projection_bytes(&bytes).expect("canonical verifies");
}

// ---------------------------------------------------------------------------
// Row 19 — counts without source are unavailable, not zero
// ---------------------------------------------------------------------------

#[test]
fn summary_counts_are_absent_not_zero_when_source_absent() {
    let wf = base_wf();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(None, &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    assert!(env.payload.summary.total_issues.is_none());
    assert!(env.payload.summary.by_type.is_empty());
    assert!(env.payload.summary.passed.is_none());
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "summary" && o.category == "unavailable")
    );
}

// ---------------------------------------------------------------------------
// Row 20 — duplicate event identity fails closed
// ---------------------------------------------------------------------------

#[test]
fn duplicate_event_identity_is_hard_error() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![
            mk_event("e1", 3, "corr-1", "2026-10-05T00:00:00Z"),
            mk_event("e2", 3, "corr-1", "2026-10-05T00:00:00Z"),
        ],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let err = render_timeline_projection(&source, None, &wf).expect_err("must fail");
    assert_eq!(err[0].code, "SOMA-CMP-0011");
}

// ---------------------------------------------------------------------------
// Row 21 — LegacyUnverified preserved
// ---------------------------------------------------------------------------

#[test]
fn legacy_provenance_state_is_preserved() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e7", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let provenance = vec![("e7".to_string(), ProvenanceState::LegacyUnverified)];
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: Some(&provenance),
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    assert_eq!(
        env.payload.events[0].provenance.refresh_state,
        "legacy-unverified"
    );
}

// ---------------------------------------------------------------------------
// Row 23 — against-source catches substituted ReviewFacts
// ---------------------------------------------------------------------------

fn self_consistent_review() -> (
    WorkflowDefinition,
    ReviewReport,
    Vec<HumanDecisionRecordV1>,
    Vec<EvidenceReference>,
    VersionedProjectionEnvelope<serde_json::Value>,
) {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = vec![evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    let v = review_value_env(&env);
    (wf, report, gates, refs, v)
}

#[test]
fn verify_review_against_source_fails_on_issue_substitution() {
    let (wf, mut report, gates, refs, v) = self_consistent_review();
    report.issues[0].message = "substituted".to_string();
    let f = facts(Some(&report), &gates, &refs);
    let err = verify_review_against_source(&v, &wf, &f, Some(&review_all_policy()))
        .expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

#[test]
fn verify_review_against_source_fails_on_gate_verdict_substitution() {
    let (wf, report, _gates, refs, v) = self_consistent_review();
    let swapped = vec![gate(
        "intake.review",
        HumanVerdict::Rejected,
        "alice",
        &hexc('b'),
    )];
    let f = facts(Some(&report), &swapped, &refs);
    let err = verify_review_against_source(&v, &wf, &f, Some(&review_all_policy()))
        .expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

#[test]
fn verify_review_against_source_fails_on_evidence_ref_substitution() {
    let (wf, report, gates, _refs, v) = self_consistent_review();
    let swapped = vec![evref("r9", &hexc('f'), &hexc('f'), "2026-10-05T00:00:09Z")];
    let f = facts(Some(&report), &gates, &swapped);
    let err = verify_review_against_source(&v, &wf, &f, Some(&review_all_policy()))
        .expect_err("must fail");
    assert!(
        err.is_empty()
            || err
                .iter()
                .any(|d| d.code == "SOMA-CMP-0004" || d.code == "PROJ-0002")
    );
}

#[test]
fn verify_review_against_source_fails_on_disposition_substitution() {
    let (wf, report, _gates, refs, v) = self_consistent_review();
    let changes = vec![gate(
        "intake.review",
        HumanVerdict::ChangesRequested,
        "alice",
        &hexc('b'),
    )];
    let f = facts(Some(&report), &changes, &refs);
    let err = verify_review_against_source(&v, &wf, &f, Some(&review_all_policy()))
        .expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

/// §5 / item 5 — mutation regression: each authoritative review fact mutation
/// flips the fresh render or fails closed.
#[test]
fn every_accepted_review_fact_changes_render_or_fails_closed() {
    let wf = base_wf();
    let (_wf_ref, base_report, base_gates, base_refs, v) = self_consistent_review();
    // Issue message mutation.
    {
        let mut r = base_report.clone();
        r.issues[0].message = "mutated".to_string();
        let mutated = facts(Some(&r), &base_gates, &base_refs);
        let fresh =
            render_review_projection(&wf, &mutated, Some(&review_all_policy())).expect("render ok");
        assert_ne!(
            serde_json::to_string(&v.payload).expect("str"),
            serde_json::to_string(&fresh.payload).expect("str"),
            "mutation must alter the projection payload"
        );
    }
    // Gate verdict substitution (changes disposition).
    {
        let changed_gates = vec![gate(
            "intake.review",
            HumanVerdict::Rejected,
            "alice",
            &hexc('b'),
        )];
        let changed = facts(Some(&base_report), &changed_gates, &base_refs);
        let fresh =
            render_review_projection(&wf, &changed, Some(&review_all_policy())).expect("render ok");
        assert_eq!(fresh.payload.disposition.status, "reject");
    }
    // Evidence reference identity mutation (changes evidence-reference list order/content ⇒ facts digest differs).
    {
        let refs_with_extra = {
            let mut r = base_refs.clone();
            r.push(evref(
                "r_extra",
                &hexc('c'),
                &hexc('e'),
                "2026-10-05T00:00:00Z",
            ));
            r
        };
        let mutated = facts(Some(&base_report), &base_gates, &refs_with_extra);
        let fresh =
            render_review_projection(&wf, &mutated, Some(&review_all_policy())).expect("render ok");
        assert_ne!(
            serde_json::to_string(&v.payload).expect("str"),
            serde_json::to_string(&fresh.payload).expect("str"),
            "mutation must alter the projection payload"
        );
    }
    // Scope identity mutation (flips projection digest / source identity).
    {
        let mut mutated = facts(Some(&base_report), &base_gates, &base_refs);
        mutated.scope = ReviewScope {
            report_id: Some("mutated-report-id".to_string()),
            run_key: Some(RunKeyView {
                kind: "other".to_string(),
                id: "other-run".to_string(),
            }),
        };
        let fresh =
            render_review_projection(&wf, &mutated, Some(&review_all_policy())).expect("render ok");
        assert_ne!(
            serde_json::to_string(&v.payload).expect("str"),
            serde_json::to_string(&fresh.payload).expect("str"),
            "mutation must alter the projection payload"
        );
    }
}

// ---------------------------------------------------------------------------
// Row 24 — against-source catches substituted TimelineProjectionSource
// ---------------------------------------------------------------------------

#[test]
fn verify_timeline_against_source_fails_on_page_meta_substitution() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let page = ProjectionPageMeta {
        next_after: 1,
        more_available: false,
    };
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: Some(&page),
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    let v = VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("value"),
    };
    verify_timeline_against_source(&v, &source, Some(&timeline_all_policy()), &wf)
        .expect("self-consistent");
    let other = ProjectionPageMeta {
        next_after: 1,
        more_available: true,
    };
    let source2 = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: Some(&other),
        scope: &rk,
    };
    let err = verify_timeline_against_source(&v, &source2, Some(&timeline_all_policy()), &wf)
        .expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

#[test]
fn verify_timeline_against_source_fails_on_provenance_substitution() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let prov_a = vec![("e1".to_string(), ProvenanceState::LegacyUnverified)];
    let prov_b = vec![("e-name".to_string(), ProvenanceState::LegacyUnverified)];
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: Some(&prov_a),
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    let v = VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("value"),
    };
    let source2 = TimelineProjectionSource {
        batch: &b,
        provenance: Some(&prov_b),
        page: None,
        scope: &rk,
    };
    let err = verify_timeline_against_source(&v, &source2, Some(&timeline_all_policy()), &wf)
        .expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

#[test]
fn verify_timeline_against_source_fails_on_batch_substitution() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env =
        render_timeline_projection(&source, Some(&timeline_all_policy()), &wf).expect("render");
    let v = VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("value"),
    };
    let b2 = batch(
        "run-1",
        vec![mk_event("e9", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source2 = TimelineProjectionSource {
        batch: &b2,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let err = verify_timeline_against_source(&v, &source2, Some(&timeline_all_policy()), &wf)
        .expect_err("must fail");
    assert!(!err.is_empty());
}

// ---------------------------------------------------------------------------
// Contract regressions (items 1-6)
// ---------------------------------------------------------------------------

#[test]
fn unknown_review_disclosure_target_fails_closed() {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = gate_refs();
    let f = facts(Some(&report), &gates, &refs);
    let policy = ReviewDisclosurePolicy {
        authorized_targets: vec!["nonsense".to_string()],
        count_authorization: vec![],
    };
    let err = render_review_projection(&wf, &f, Some(&policy)).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0003"), "{err:?}");
}

#[test]
fn unknown_timeline_disclosure_target_fails_closed() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let policy = TimelineDisclosurePolicy {
        authorized_targets: vec!["bogus".to_string()],
        count_authorization: vec![],
    };
    let err = render_timeline_projection(&source, Some(&policy), &wf).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0003"), "{err:?}");
}

#[test]
fn timeline_audit_rejects_invalid_batch() {
    let wf = base_wf();
    let rk = run_key("run-1");
    // `e2` references a missing causal parent => SOMA-EVT-0002 at audit.
    let b = batch(
        "run-1",
        vec![mk_event_parented(
            "e2",
            2,
            "corr-1",
            "2026-10-05T00:00:00Z",
            vec!["ghost".to_string()],
        )],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let err = render_timeline_projection(&source, None, &wf).expect_err("must fail closed");
    assert!(err.iter().any(|d| d.code == "SOMA-EVT-0002"), "{err:?}");
}

#[test]
fn timeline_scope_must_match_batch_run_id() {
    let wf = base_wf();
    let rk = run_key("other-run");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let err = render_timeline_projection(&source, None, &wf).expect_err("must fail closed");
    assert!(err.iter().any(|d| d.code == "SOMA-CMP-0002"), "{err:?}");
}

#[test]
fn completeness_is_unknown_when_page_state_absent() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("render");
    assert!(!env.payload.completeness.available);
    assert!(env.payload.completeness.more_available.is_none());
    assert!(env.payload.completeness.next_after.is_none());
}

#[test]
fn completeness_is_copied_when_page_state_present() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let page = ProjectionPageMeta {
        next_after: 7,
        more_available: true,
    };
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: Some(&page),
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("render");
    assert!(env.payload.completeness.available);
    assert_eq!(env.payload.completeness.more_available, Some(true));
    assert_eq!(env.payload.completeness.next_after, Some(7));
}

#[test]
fn sequence_gap_is_surfaced_as_source_gap() {
    let wf = base_wf();
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![
            mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z"),
            mk_event("e3", 3, "corr-1", "2026-10-05T00:00:02Z"),
        ],
    );
    let source = TimelineProjectionSource {
        batch: &b,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("render");
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.reason == "sourceGap" && o.category == "unavailable")
    );
}

#[test]
fn supported_by_evidence_is_bound_to_gate_basis() {
    let wf = base_wf();
    let report = synthetic_report();
    // Gate with a non-empty basis matching an authoritative reference.
    let gates = vec![gate(
        "intake.review",
        HumanVerdict::Approved,
        "alice",
        &hexc('b'),
    )];
    let matching = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &matching);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    assert!(env.payload.disposition.supported_by_evidence);
    // No gate records => unavailable disposition and no evidence support.
    let none: [HumanDecisionRecordV1; 0] = [];
    let f2 = facts(Some(&report), &none, &matching);
    let env2 = render_review_projection(&wf, &f2, Some(&review_all_policy())).expect("render");
    assert_eq!(env2.payload.disposition.status, "unavailable");
    assert!(!env2.payload.disposition.supported_by_evidence);
}

#[test]
fn review_authority_represents_heterogeneous_channels() {
    use prometheos_lite::workflow::graph_gates::ReviewChannel as Ch;
    let wf = base_wf();
    let report = synthetic_report();
    let gates = vec![
        HumanDecisionRecordV1::author(
            "g1",
            HumanVerdict::Approved,
            "alice",
            Ch::CliInteractive,
            hexc('b'),
            "r",
            "2026-10-05T00:00:00Z",
        )
        .expect("author"),
        HumanDecisionRecordV1::author(
            "g2",
            HumanVerdict::Approved,
            "bob",
            Ch::ExternalSystem {
                system_id: "gerrit".to_string(),
            },
            hexc('c'),
            "r",
            "2026-10-05T00:00:01Z",
        )
        .expect("author"),
    ];
    let refs = [
        evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z"),
        evref("r2", &hexc('c'), &hexc('c'), "2026-10-05T00:00:01Z"),
    ];
    let f = facts(Some(&report), &gates, &refs);
    let env = render_review_projection(&wf, &f, Some(&review_all_policy())).expect("render");
    assert_eq!(env.payload.authority.review_channels.len(), 2);
    assert!(env.payload.authority.review_channel.is_none());
    assert!(env.payload.authority.executor_class.is_none());
}

// ---------------------------------------------------------------------------
// Golden locks — read-only verification; writer is an ignored authoring helper
// ---------------------------------------------------------------------------

fn golden_review_env() -> VersionedProjectionEnvelope<ReviewProjectionPayload> {
    let wf = base_wf();
    let report = synthetic_report();
    let gates = approved_gate();
    let refs = [evref("r1", &hexc('b'), &hexc('b'), "2026-10-05T00:00:00Z")];
    let f = facts(Some(&report), &gates, &refs);
    render_review_projection(&wf, &f, Some(&review_all_policy())).expect("golden review render")
}

fn golden_timeline_env(
    wf: &WorkflowDefinition,
    rk: &RunKey,
    b: &WorkEventBatch,
) -> VersionedProjectionEnvelope<prometheos_lite::workflow::projection::TimelineProjectionPayload> {
    let source = TimelineProjectionSource {
        batch: b,
        provenance: None,
        page: None,
        scope: rk,
    };
    render_timeline_projection(&source, Some(&timeline_all_policy()), wf).expect("golden timeline")
}

#[test]
fn slice3_golden_fixtures_match_renders() {
    let wf = base_wf();
    let review_env = golden_review_env();
    let review_bytes = review_env.canonical_bytes().expect("canonical");
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let timeline_env = golden_timeline_env(&wf, &rk, &b);
    let timeline_bytes = timeline_env.canonical_bytes().expect("canonical");

    let review_path = format!(
        "{}/tests/fixtures/slice3/valid/review-report-basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let timeline_path = format!(
        "{}/tests/fixtures/slice3/valid/timeline-basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    for (path, bytes) in [(review_path, review_bytes), (timeline_path, timeline_bytes)] {
        match std::fs::read(&path) {
            Ok(disk) => assert_eq!(disk, bytes, "{path} drifted from the authoritative render"),
            Err(_) => panic!("missing fixture {path} — run `golden_fixture_writer` (ignored) once"),
        }
    }
}

/// Authoring helper — NOT part of the normal suite (item 8: no repository
/// mutation during ordinary runs). Run explicitly with:
/// `cargo test --test review_timeline_projection_conformance_tests -- --ignored`
#[test]
#[ignore = "authoring helper: writes golden fixtures; run explicitly with --ignored"]
fn golden_fixture_writer() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("slice3")
        .join("valid");
    std::fs::create_dir_all(&dir).expect("dir");
    let wf = base_wf();
    let review_env = golden_review_env();
    std::fs::write(
        dir.join("review-report-basic.json"),
        review_env.canonical_bytes().expect("canonical"),
    )
    .expect("write review fixture");
    let rk = run_key("run-1");
    let b = batch(
        "run-1",
        vec![mk_event("e1", 1, "corr-1", "2026-10-05T00:00:00Z")],
    );
    let timeline_env = golden_timeline_env(&wf, &rk, &b);
    std::fs::write(
        dir.join("timeline-basic.json"),
        timeline_env.canonical_bytes().expect("canonical"),
    )
    .expect("write timeline fixture");
}

#[test]
fn invalid_fixtures_fail_under_byte_verifiers() {
    let invalid_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("slice3")
        .join("invalid");
    for entry in std::fs::read_dir(&invalid_dir).expect("invalid fixture dir exists") {
        let entry = entry.expect("entry");
        let bytes = std::fs::read(entry.path()).expect("read");
        assert!(
            verify_review_projection_bytes(&bytes).is_err()
                && verify_timeline_projection_bytes(&bytes).is_err(),
            "{} must fail closed",
            entry.path().display()
        );
    }
}

#[test]
fn schema_version_constants_are_pinned() {
    assert_eq!(REVIEW_SCHEMA_VERSION, "lite.review-report.v1");
    assert_eq!(TIMELINE_SCHEMA_VERSION, "lite.evidence-timeline.v1");
}
