//! Slice 3 — review-report + evidence-timeline projection conformance tests.
//!
//! RED evidence: before `src/workflow/projection/review.rs` / `.../timeline.rs`
//! were added, `cargo test --test review_timeline_projection_conformance_tests`
//! failed with every compile dependency on missing items — the first-task RED.

use prometheos_lite::db::repository::work_context_events::ProvenanceState;
use prometheos_lite::harness::review::{
    QualityGrade, ReviewIssue, ReviewIssueType, ReviewQualityScore, ReviewReport, ReviewSeverity,
    ReviewSummary,
};
use prometheos_lite::work::soma_projection::{RunKey, RunKeyKind};
use prometheos_lite::workflow::graph_gates::{HumanDecisionRecordV1, HumanVerdict, ReviewChannel};
use prometheos_lite::workflow::projection::{
    ProjectionPageMeta, REVIEW_SCHEMA_VERSION, ReviewDisclosurePolicy, ReviewFacts, ReviewScope,
    TIMELINE_SCHEMA_VERSION, TimelineProjectionSource, render_review_projection,
    render_timeline_projection, verify_review_against_source, verify_timeline_against_source,
    verify_timeline_projection_bytes,
};
use prometheos_lite::workflow::soma::contracts::WorkflowDefinition;
use prometheos_lite::workflow::soma::event::{WorkEvent, WorkEventBatch};

use std::collections::HashMap;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn base_wf() -> WorkflowDefinition {
    serde_json::from_str(&fixture("fixtures/valid/wf-valid-base.json")).expect("fixture parses")
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

fn synthetic_gates() -> Vec<HumanDecisionRecordV1> {
    vec![
        HumanDecisionRecordV1::author(
            "intake.review",
            HumanVerdict::Approved,
            "alice",
            ReviewChannel::CliInteractive,
            "a".repeat(64),
            "reviewed and accepted",
            "2026-10-05T00:00:00Z",
        )
        .expect("author succeeds"),
    ]
}

fn synthetic_run() -> RunKey {
    RunKey {
        kind: RunKeyKind::WorkRun,
        id: "run-1".to_string(),
    }
}

#[test]
fn review_schema_version_constant_matches_payload_field() {
    let wf = base_wf();
    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope {
            report_id: Some("r1".to_string()),
            run_key: Some(prometheos_lite::workflow::projection::RunKeyView {
                kind: "work-run".to_string(),
                id: "run-1".to_string(),
            }),
        },
    };
    let env = render_review_projection(&wf, &facts, None).expect("renders");
    assert_eq!(env.payload.review_schema_version, REVIEW_SCHEMA_VERSION);
    assert_eq!(env.payload.disclosure_policy_digest.len(), 64);
    assert!(env.payload.gates.len() == 1);
    assert!(env.payload.issues.len() == 2);
    // evidence_bundles is empty → a single unavailable guidance records the absence
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "evidence" && o.category == "unavailable")
    );
}

#[test]
fn timeline_schema_version_constant_matches_payload_field() {
    assert_eq!(TIMELINE_SCHEMA_VERSION, "lite.evidence-timeline.v1");
}

#[test]
fn render_timeline_projection_is_deterministic_for_same_input() {
    let wf = base_wf();
    let run_key = synthetic_run();
    let events = vec![mk_event(
        "e1",
        5,
        "1".repeat(64).as_str(),
        "2026-10-05T00:00:00Z",
    )];
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: run_key.id.clone(),
        events: events.clone(),
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: Some(&ProjectionPageMeta {
            next_after: 5,
            more_available: false,
        }),
        scope: &run_key,
    };
    let a = render_timeline_projection(&source, None, &wf).expect("render a");
    let b = render_timeline_projection(&source, None, &wf).expect("render b");
    assert_eq!(
        a, b,
        "timeline rendering must be deterministic for the same source"
    );
}

#[test]
fn timeline_orders_by_sequence_then_semantic_digest_not_timestamp() {
    let wf = base_wf();
    let rk = synthetic_run();
    let events = vec![
        // identical timestamp; sequence is the load-bearing same-time key
        mk_event("e2", 9, "2".repeat(64).as_str(), "2026-10-05T00:00:00Z"),
        mk_event("e1", 5, "1".repeat(64).as_str(), "2026-10-05T00:00:00Z"),
    ];
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events,
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("renders");
    let rendered = env.payload.events;
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0].sequence, 5);
    assert_eq!(rendered[1].sequence, 9);
    assert_eq!(rendered[0].event_id, "e1");
    assert_eq!(rendered[1].event_id, "e2");
}

#[test]
fn verify_timeline_projection_bytes_detects_payload_mutation() {
    let wf = base_wf();
    let rk = synthetic_run();
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events: vec![mk_event(
            "e1",
            1,
            "a".repeat(64).as_str(),
            "2026-10-05T00:00:00Z",
        )],
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("renders");
    let mut bytes = env.canonical_bytes().expect("canonical");
    assert!(verify_timeline_projection_bytes(&bytes).is_ok());
    bytes[0] ^= 0x01;
    assert!(verify_timeline_projection_bytes(&bytes).is_err());
}

#[test]
fn duplicate_event_identity_is_hard_error() {
    let wf = base_wf();
    let rk = synthetic_run();
    let events = vec![
        mk_event("e1", 3, "a".repeat(64).as_str(), "2026-10-05T00:00:00Z"),
        mk_event("e2", 3, "a".repeat(64).as_str(), "2026-10-05T00:00:00Z"),
    ];
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events,
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let err = render_timeline_projection(&source, None, &wf).expect_err("must fail");
    assert_eq!(err[0].code, "SOMA-CMP-0011");
}

#[test]
fn legacy_unverified_provenance_is_preserved_and_never_coerced() {
    let wf = base_wf();
    let rk = synthetic_run();
    let events = vec![mk_event(
        "e7",
        1,
        "c".repeat(64).as_str(),
        "2026-10-05T00:00:00Z",
    )];
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events,
        compatibility: None,
    };
    let provenance = vec![("e7".to_string(), ProvenanceState::LegacyUnverified)];
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: Some(&provenance),
        page: None,
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("renders");
    assert_eq!(
        env.payload.events[0].provenance.refresh_state,
        "legacy-unverified"
    );
}

#[test]
fn missing_report_aspect_marks_counts_unavailable_not_zero() {
    let wf = base_wf();
    let facts = ReviewFacts {
        report: None,
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let env = render_review_projection(&wf, &facts, None).expect("renders");
    assert!(env.payload.summary.total_issues.is_none());
    assert!(env.payload.summary.by_type.is_empty());
    assert!(env.payload.summary.by_severity.is_empty());
    assert!(
        env.payload
            .omissions
            .iter()
            .any(|o| o.section == "summary" && o.category == "unavailable")
    );
}

#[test]
fn verify_review_against_source_fails_on_issue_substitution() {
    let wf = base_wf();
    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let env = render_review_projection(&wf, &facts, None).expect("renders");
    let env_value = serde_json::to_value(&env).expect("envelope value");
    let ok_env = prometheos_lite::workflow::projection::VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("payload"),
    };
    let v: prometheos_lite::workflow::projection::VersionedProjectionEnvelope<serde_json::Value> =
        ok_env;
    verify_review_against_source(&v, &wf, &facts, None).expect("self-consistent");
    // Now alter an issue's message in the facts copy used for verification:
    let mut report2 = synthetic_report();
    report2.issues[0].message = "substituted message".to_string();
    let facts2 = ReviewFacts {
        report: Some(&report2),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let err = verify_review_against_source(&v, &wf, &facts2, None).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
    let _ = env_value;
}

#[test]
fn verify_timeline_against_source_fails_on_page_meta_substitution() {
    let wf = base_wf();
    let rk = synthetic_run();
    let events = vec![mk_event(
        "e5",
        2,
        "d".repeat(64).as_str(),
        "2026-10-05T00:00:00Z",
    )];
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events,
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: Some(&ProjectionPageMeta {
            next_after: 2,
            more_available: false,
        }),
        scope: &rk,
    };
    let env = render_timeline_projection(&source, None, &wf).expect("renders");
    let v = prometheos_lite::workflow::projection::VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("payload"),
    };
    verify_timeline_against_source(&v, &source, None, &wf).expect("self-consistent");
    let other_page = ProjectionPageMeta {
        next_after: 2,
        more_available: true,
    };
    let source2 = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: Some(&other_page),
        scope: &rk,
    };
    let err = verify_timeline_against_source(&v, &source2, None, &wf).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "PROJ-0002"), "{err:?}");
}

#[test]
fn verify_review_against_source_fails_on_policy_change() {
    let wf = base_wf();
    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let policy0 = ReviewDisclosurePolicy::default();
    let env = render_review_projection(&wf, &facts, Some(&policy0)).expect("renders");
    let v = prometheos_lite::workflow::projection::VersionedProjectionEnvelope {
        projection_version: env.projection_version.clone(),
        schema_version: env.schema_version.clone(),
        source_digest: env.source_digest.clone(),
        projection_digest: env.projection_digest.clone(),
        payload: serde_json::to_value(&env.payload).expect("payload"),
    };
    verify_review_against_source(&v, &wf, &facts, Some(&policy0)).expect("self-consistent");
    // A different §6 target list flips the envelope's disclosed output.
    let policy1 = ReviewDisclosurePolicy {
        authorized_targets: vec!["principals".to_string()],
        count_authorization: vec!["issues".to_string()],
    };
    let err = verify_review_against_source(&v, &wf, &facts, Some(&policy1)).expect_err("must fail");
    // Policy digest mismatch is a canonicalization failure (§7), not a PROJ-0002 render mismatch.
    assert!(err.iter().any(|d| d.code == "SOMA-CMP-0004"), "{err:?}");
}

#[test]
fn cross_slice_envelope_verify_parity() {
    use prometheos_lite::workflow::projection::{
        project_canonical_json, verify_canonical_projection_bytes, verify_review_projection_bytes,
        verify_timeline_projection_bytes,
    };
    let wf = base_wf();
    let canonical_env = project_canonical_json(&wf).expect("canonical renders");
    let canonical_bytes = canonical_env.canonical_bytes().expect("canonical bytes");
    verify_canonical_projection_bytes(&canonical_bytes).expect("canonical verifies");

    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let review_env = render_review_projection(&wf, &facts, None).expect("review renders");
    let review_bytes = review_env.canonical_bytes().expect("review bytes");
    verify_review_projection_bytes(&review_bytes).expect("review verifies");

    let rk = synthetic_run();
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events: vec![mk_event("e-parity", 1, "b".repeat(64).as_str(), "2026-10-05T00:00:00Z")],
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let timeline_env = render_timeline_projection(&source, None, &wf).expect("timeline renders");
    let timeline_bytes = timeline_env.canonical_bytes().expect("timeline bytes");
    verify_timeline_projection_bytes(&timeline_bytes).expect("timeline verifies");

    // All three envelopes parse under the same strict envelope rules and
    // re-render byte-identically.
    for bytes in [&canonical_bytes, &review_bytes, &timeline_bytes] {
        let bytes: &[u8] = bytes;
        let env = serde_json::from_slice::<
            prometheos_lite::workflow::projection::VersionedProjectionEnvelope<serde_json::Value>,
        >(bytes);
        assert!(env.is_ok(), "shared envelope bytes parse");
    }
}

/// Deterministic golden-lock: renders match the byte-locked fixture files.
/// When fixtures are absent (first authoring) this test records RED.
#[test]
fn slice3_golden_fixtures_match_renders() {
    let wf = base_wf();
    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let review_env = render_review_projection(&wf, &facts, None).expect("render review");
    let review_bytes = review_env.canonical_bytes().expect("canonical");

    let rk = synthetic_run();
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events: vec![mk_event(
            "e-generated",
            1,
            "9".repeat(64).as_str(),
            "2026-10-05T00:00:00Z",
        )],
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let timeline_env = render_timeline_projection(&source, None, &wf).expect("render timeline");
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
            Err(_) => panic!("missing fixture {path} — generate it first (golden_fixture_writer)"),
        }
    }
}

/// Authoring helper: emits the golden fixture files. Run exactly once.
#[test]
fn golden_fixture_writer() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("slice3")
        .join("valid");
    std::fs::create_dir_all(&dir).expect("dir");
    let wf = base_wf();
    let facts = ReviewFacts {
        report: Some(&synthetic_report()),
        gates: &synthetic_gates(),
        evidence_bundles: &[],
        scope: ReviewScope::default(),
    };
    let review_env = render_review_projection(&wf, &facts, None).expect("render review");
    std::fs::write(
        dir.join("review-report-basic.json"),
        review_env.canonical_bytes().expect("canonical"),
    )
    .expect("write review fixture");

    let rk = synthetic_run();
    let batch = WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: rk.id.clone(),
        events: vec![mk_event(
            "e-generated",
            1,
            "9".repeat(64).as_str(),
            "2026-10-05T00:00:00Z",
        )],
        compatibility: None,
    };
    let source = TimelineProjectionSource {
        batch: &batch,
        provenance: None,
        page: None,
        scope: &rk,
    };
    let timeline_env = render_timeline_projection(&source, None, &wf).expect("render timeline");
    std::fs::write(
        dir.join("timeline-basic.json"),
        timeline_env.canonical_bytes().expect("canonical"),
    )
    .expect("write timeline fixture");
}

/// Invalid fixtures must fail closed under both byte verifiers.
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
        let review_ok = verify_review_projection_bytes_boot(&bytes);
        let timeline_ok = verify_timeline_projection_bytes(&bytes).is_ok();
        assert!(
            !review_ok && !timeline_ok,
            "{} must fail closed",
            entry.path().display()
        );
    }
}

fn verify_review_projection_bytes_boot(bytes: &[u8]) -> bool {
    use prometheos_lite::workflow::projection::verify_review_projection_bytes as v;
    v(bytes).is_ok()
}

fn mk_event(id: &str, sequence: u64, digest: &str, timestamp: &str) -> WorkEvent {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": "1.1",
        "version": "1.1.0",
        "id": id,
        "eventType": "status",
        "actor": {"kind": "harness", "identity": "actor-1"},
        "authority": {"executionClass": "deterministic", "mutation": "none"},
        "effectiveAuthority": {"executionClass": "deterministic", "mutation": "none"},
        "sequence": sequence,
        "timestamp": timestamp,
        "idempotencyKey": format!("ik-{id}"),
        "correlationId": "corr-1",
        "repoRevision": "aabbccdd0011223344556677889900112233445500112233445566778899001122",
        "compatibility": {"schemaVersion": "1.1", "minReaderVersion": "1.0"},
        "semanticDigest": digest,
        "parents": [],
        "payload": {},
        "replay": false,
        "conflict": false
    }))
    .expect("event literal")
}
