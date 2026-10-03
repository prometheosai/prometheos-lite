//! #132 Slice 1B: the pure fail-closed SOMA `WorkEvent` projection over
//! the verified Slice 1A journal â€” conformance, goldens, and the
//! fail-closed matrix.
//!
//! Layer 1 (this file's first tests): the vendored SPEC 006 event
//! fixtures (`vendored/soma/v1.1/fixtures/`, digest-locked by the
//! bundle manifest) must audit with exactly their manifest-pinned
//! diagnostics under Lite's vendored verifier â€” cross-implementation
//! parity with the published soma-native verifier BEFORE the journal
//! mapping exists.
//!
//! Later sections add the journal-driven mapping tests, the byte-locked
//! goldens, and the fail-closed matrix (legacy/tampered/mixed/unmapped
//! records are refused, never fabricated).

use prometheos_lite::workflow::soma::canonical::try_canonical_digest;
use prometheos_lite::workflow::soma::event::{WorkEvent, WorkEventBatch};
use prometheos_lite::workflow::soma::supported_version;
use serde::Deserialize;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1/fixtures");

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    path: String,
    kind: String,
    #[serde(rename = "expected_codes")]
    expected_codes: Vec<String>,
    artifact: String,
    sha256: String,
}

/// Every SPEC 006 event fixture the v1.1 bundle pins (manifest-driven;
/// a bundle upgrade that adds fixtures automatically extends this test).
fn event_manifest_entries() -> Vec<ManifestEntry> {
    #[derive(Debug, Deserialize)]
    struct ManifestFile {
        fixtures: Vec<ManifestEntry>,
    }
    let manifest: ManifestFile = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURES}/manifest.json"))
            .expect("vendored fixture manifest is present"),
    )
    .expect("vendored fixture manifest shape is pinned by the bundle");
    manifest
        .fixtures
        .into_iter()
        .filter(|e| e.artifact == "WorkEvent" || e.artifact == "WorkEventBatch")
        .collect()
}

fn fixture_bytes(entry: &ManifestEntry) -> Vec<u8> {
    // Manifest paths are "fixtures/<kind>/<name>.json" relative to the
    // bundle root (vendored/soma/v1.1/).
    let rel = entry.path.strip_prefix("fixtures/").unwrap_or(&entry.path);
    std::fs::read(format!("{FIXTURES}/{rel}"))
        .unwrap_or_else(|e| panic!("{}: fixture missing ({e})", entry.path))
}

/// The known event-fixture set: if the bundle gains or loses fixtures,
/// this assert forces an explicit review of the change.
#[test]
fn vendored_event_fixture_set_is_the_pinned_fifteen() {
    let entries = event_manifest_entries();
    let mut names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "fixtures/invalid/wev-auth-cap.json",
            "fixtures/invalid/wev-auth-level.json",
            "fixtures/invalid/wev-auth-mutation.json",
            "fixtures/invalid/wev-auth-network.json",
            "fixtures/invalid/wev-auth-scope.json",
            "fixtures/invalid/wev-cyclic.json",
            "fixtures/invalid/wev-missing-parent.json",
            "fixtures/invalid/wev-no-evidence.json",
            "fixtures/invalid/wev-replay-conflict.json",
            "fixtures/invalid/wev-semantic-loss.json",
            "fixtures/invalid/wev-unsupported-version.json",
            "fixtures/valid/wev-checkpoint-resume.json",
            "fixtures/valid/wev-handoff-native.json",
            "fixtures/valid/wev-valid-chain.json",
            "fixtures/valid/wev-valid-idempotent.json",
        ],
        "the vendored v1.1 event fixture set changed â€” review the bundle upgrade"
    );
}

/// Digest-lock: every event fixture's CANONICAL CONTENT digest must equal
/// the manifest sha256 â€” the same digest the bundle pins (computed by the
/// soma-native verifier over the canonical SOMA render of the parsed
/// fixture, NOT raw file bytes â€” the lock is therefore EOL-independent).
#[test]
fn vendored_event_fixtures_match_their_manifest_digests() {
    for entry in event_manifest_entries() {
        let bytes = fixture_bytes(&entry);
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("{}: not valid JSON: {e}", entry.path));
        let digest = try_canonical_digest(&value)
            .unwrap_or_else(|e| panic!("{}: canonical digest failed: {e}", entry.path));
        assert_eq!(
            digest, entry.sha256,
            "{}: canonical content digest diverges from the manifest digest",
            entry.path
        );
    }
}

/// Valid SPEC 006 fixtures must parse into their pinned artifact type
/// and audit CLEAN under the vendored verifier.
#[test]
fn vendored_valid_event_fixtures_audit_clean() {
    let supported = supported_version();
    for entry in event_manifest_entries()
        .into_iter()
        .filter(|e| e.kind == "valid")
    {
        let bytes = fixture_bytes(&entry);
        match entry.artifact.as_str() {
            "WorkEvent" => {
                let event: WorkEvent = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                assert!(
                    event.audit(&supported).is_empty(),
                    "{}: valid fixture audits dirty: {:?}",
                    entry.path,
                    event.audit(&supported)
                );
            }
            "WorkEventBatch" => {
                let batch: WorkEventBatch = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                assert!(
                    batch.audit(&supported).is_empty(),
                    "{}: valid fixture audits dirty: {:?}",
                    entry.path,
                    batch.audit(&supported)
                );
            }
            other => panic!("{}: unknown artifact {other}", entry.path),
        }
    }
}

/// Invalid SPEC 006 fixtures must audit with EXACTLY their
/// manifest-pinned `expected_codes` (set equality â€” the same ground-truth
/// rule the published soma-native verifier enforces).
#[test]
fn vendored_invalid_event_fixtures_produce_manifest_pinned_codes() {
    let supported = supported_version();
    for entry in event_manifest_entries()
        .into_iter()
        .filter(|e| e.kind == "invalid")
    {
        let bytes = fixture_bytes(&entry);
        let codes: Vec<String> = match entry.artifact.as_str() {
            "WorkEvent" => {
                let event: WorkEvent = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                event
                    .audit(&supported)
                    .into_iter()
                    .map(|d| d.code)
                    .collect()
            }
            "WorkEventBatch" => {
                let batch: WorkEventBatch = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                batch
                    .audit(&supported)
                    .into_iter()
                    .map(|d| d.code)
                    .collect()
            }
            other => panic!("{}: unknown artifact {other}", entry.path),
        };
        let mut want = entry.expected_codes.clone();
        want.sort_unstable();
        let mut got = codes;
        got.sort_unstable();
        got.dedup();
        assert_eq!(
            got, want,
            "{}: audit codes diverge from the manifest-pinned expected_codes",
            entry.path
        );
    }
}

// ---------------------------------------------------------------------------
// Slice 1B journal projection: stream pages (correction 1 - a page is
// NOT a WorkEventBatch), complete-record source digest (correction 2),
// cursor semantics, and determinism.
// ---------------------------------------------------------------------------

use prometheos_lite::db::Db;
use prometheos_lite::work::JournalContext;
use prometheos_lite::work::WorkContextService;
use prometheos_lite::work::soma_projection::{ProjectionPage, WorkEventStreamPage, project_page};
use prometheos_lite::work::types::{AutonomyLevel, WorkDomain};
use prometheos_lite::workflow::projection::VersionedProjectionEnvelope;
use std::sync::Arc;

fn test_journal() -> JournalContext {
    JournalContext::internal_system(
        format!("test-{}", uuid::Uuid::new_v4()),
        JournalContext::work_authority(AutonomyLevel::Review, Default::default()),
    )
}

fn setup() -> (Arc<Db>, Arc<WorkContextService>) {
    let db = Arc::new(Db::in_memory().unwrap());
    let wcs = Arc::new(WorkContextService::new(db.clone()));
    (db, wcs)
}

fn create_context(wcs: &WorkContextService) -> prometheos_lite::work::types::WorkContext {
    let journal = test_journal();
    wcs.create_context(
        "user-1".to_string(),
        "Projection test".to_string(),
        WorkDomain::General,
        "goal".to_string(),
        &journal,
    )
    .unwrap()
}

/// Drive a fixed, fully-mappable event set through the REAL writers.
fn drive_events(wcs: &WorkContextService) -> prometheos_lite::work::types::WorkContext {
    let journal = test_journal();
    let mut context = create_context(wcs); // context_created
    wcs.update_status(
        &mut context,
        prometheos_lite::work::types::WorkStatus::InProgress,
        &journal,
    )
    .unwrap(); // status_changed
    wcs.update_phase(
        &mut context,
        prometheos_lite::work::types::WorkPhase::Planning,
        &journal,
    )
    .unwrap(); // phase_transition
    context
}

#[test]
fn page_payload_is_not_a_batch_on_the_wire() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs);
    let page = project_page(&db, &context.id, 0, 500).unwrap();
    let event = page.envelope.payload.events[0].clone();

    // A page serializes with ONLY `events`; deserializing it as a
    // WorkEventBatch must fail (missing required batch fields).
    let page_bytes = serde_json::to_vec(&WorkEventStreamPage {
        events: vec![event.clone()],
    })
    .unwrap();
    assert!(
        serde_json::from_slice::<prometheos_lite::workflow::soma::event::WorkEventBatch>(
            &page_bytes
        )
        .is_err(),
        "a stream page must not be consumable as a WorkEventBatch"
    );

    // A batch must not deserialize as a page (deny_unknown_fields).
    let batch = prometheos_lite::workflow::soma::event::WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: "run-1".to_string(),
        events: vec![event],
        compatibility: None,
    };
    let batch_bytes = serde_json::to_vec(&batch).unwrap();
    assert!(
        serde_json::from_slice::<WorkEventStreamPage>(&batch_bytes).is_err(),
        "a WorkEventBatch must not be consumable as a stream page"
    );
}

#[test]
fn pages_cover_the_journal_without_gaps_or_duplication() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs); // exactly 3 events

    // Page through with a small limit; the union must be every event,
    // seq-ordered, exactly once.
    let mut seen: Vec<(u64, String)> = Vec::new();
    let mut after = 0i64;
    let mut pages = 0;
    loop {
        let page: ProjectionPage = project_page(&db, &context.id, after, 2).unwrap();
        for event in &page.envelope.payload.events {
            seen.push((event.sequence, event.id.clone()));
        }
        pages += 1;
        match page.next_after {
            Some(next) => after = next,
            None => break,
        }
        assert!(pages < 10, "paging must terminate");
    }
    assert_eq!(pages, 2, "3 events with limit 2 -> exactly 2 pages");
    assert_eq!(seen.len(), 3);
    let mut seqs: Vec<u64> = seen.iter().map(|(s, _)| *s).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "pages preserve durable seq order");
    seqs.dedup();
    assert_eq!(seqs.len(), 3, "no duplicates across pages");

    // Resuming from the last DELIVERED seq (the exhausted cursor) yields
    // an empty page: no gaps, no duplication on reconnect.
    let last_delivered = seen.last().unwrap().0 as i64;
    let done = project_page(&db, &context.id, last_delivered, 2).unwrap();
    assert!(done.envelope.payload.events.is_empty());
    assert_eq!(done.next_after, None);
}

#[test]
fn page_envelope_carries_verified_digests_and_is_deterministic() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs);

    let first = project_page(&db, &context.id, 0, 500).unwrap();
    let second = project_page(&db, &context.id, 0, 500).unwrap();

    // Pure projection: identical input -> byte-identical canonical output.
    assert_eq!(
        first.envelope.canonical_bytes().unwrap(),
        second.envelope.canonical_bytes().unwrap(),
        "the projection is deterministic"
    );

    let env: &VersionedProjectionEnvelope<WorkEventStreamPage> = &first.envelope;
    assert_eq!(env.projection_version, "projection.v1");
    assert_eq!(env.schema_version, "1.1.0");
    assert_ne!(
        env.source_digest, env.projection_digest,
        "journal binding and payload digest are distinct roles"
    );
    for digest in [&env.source_digest, &env.projection_digest] {
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}

#[test]
fn empty_context_projects_an_empty_page() {
    let (db, _wcs) = setup();
    // A context id with no journal at all: an honest empty stream page.
    let page = project_page(&db, &uuid::Uuid::new_v4().to_string(), 0, 10).unwrap();
    assert!(page.envelope.payload.events.is_empty());
    assert_eq!(page.next_after, None);
}

// ---------------------------------------------------------------------------
// Per-run batches (correction 4): real recorded run identities, typed
// keys that never merge equal strings (binding requirement 1),
// cross-run causal-ancestor closure, and the full audit gate.
// ---------------------------------------------------------------------------

use prometheos_lite::db::repository::work_context::WorkContextOperations;
use prometheos_lite::db::repository::work_context_events::record_event_conn;
use prometheos_lite::work::event::WorkContextEvent;
use prometheos_lite::work::soma_projection::{
    ProjectionError, RunKey, RunKeyKind, project_run_work_event_batch, recorded_run_keys,
};

/// The `work_context_events` table has a real foreign key to
/// `work_contexts` (PRAGMA foreign_keys = ON at init) â€” every scenario
/// context row must actually exist before journal events reference it.
fn persist_context(db: &Db, context_id: &str) {
    let context = prometheos_lite::work::types::WorkContext::new(
        context_id.to_string(),
        "user-1".to_string(),
        format!("Scenario {context_id}"),
        prometheos_lite::work::types::WorkDomain::General,
        "goal".to_string(),
    );
    WorkContextOperations::create_work_context(db, &context).unwrap();
}

/// A fixed, deterministic journal scenario: a work run "wr-1" whose
/// execution was interrupted, with the interruption's causal parent â€”
/// the cancellation â€” written under a DIFFERENT run identity (the
/// cancel request). Timestamps/ids are fixed: this exact scenario is
/// byte-locked as the golden below.
fn golden_scenario(db: &Db) -> String {
    let work_context_id = "ctx-golden".to_string();
    persist_context(db, &work_context_id);
    let authority = JournalContext::work_authority(
        prometheos_lite::work::types::AutonomyLevel::Review,
        Default::default(),
    );

    // 1. The cancellation, written by the cancel REQUEST (a human
    //    acting directly) â€” a different run identity from the work run.
    let cancel_journal =
        JournalContext::for_request("user-1", "req-cancel".to_string(), authority.clone());
    let cancel_event = WorkContextEvent {
        id: "ev-cancel".to_string(),
        work_context_id: work_context_id.clone(),
        event_type: "context_cancelled".to_string(),
        data: serde_json::json!({
            "from": "InProgress", "to": "Cancelled", "reason": "operator request"
        }),
        created_at: chrono::DateTime::parse_from_rfc3339("2026-10-02T12:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc),
    };
    record_event_conn(
        db.conn(),
        &cancel_event,
        &cancel_journal.event_envelope(None),
    )
    .unwrap();

    // 2-3. The work run's own events.
    let run_journal = JournalContext::for_work_run(
        "user-1",
        "req-run".to_string(),
        "wr-1".to_string(),
        authority,
    );
    for (id, event_type, data, parent, created_at) in [
        (
            "ev-run-start",
            "status_changed",
            serde_json::json!({ "from": "Pending", "to": "InProgress" }),
            None,
            "2026-10-02T12:01:00+00:00",
        ),
        (
            "ev-interrupted",
            "execution_interrupted",
            serde_json::json!({
                "reason": "cancelled", "iterations": 3,
                "phase": "Planning", "checkpoint_ref": null
            }),
            // The REAL recorded causal parent: the exact cancellation
            // event id â€” never a "latest" inference.
            Some("ev-cancel"),
            "2026-10-02T12:02:00+00:00",
        ),
    ] {
        let event = WorkContextEvent {
            id: id.to_string(),
            work_context_id: work_context_id.clone(),
            event_type: event_type.to_string(),
            data,
            created_at: chrono::DateTime::parse_from_rfc3339(created_at)
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        record_event_conn(
            db.conn(),
            &event,
            &run_journal.event_envelope(parent.map(str::to_string)),
        )
        .unwrap();
    }
    work_context_id
}

#[test]
fn run_batch_carries_the_real_run_identity_not_the_container() {
    let (db, _wcs) = setup();
    let context_id = golden_scenario(&db);
    let batch = project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::WorkRun,
            id: "wr-1".to_string(),
        },
    )
    .unwrap();
    assert_eq!(batch.payload.run_id, "wr-1");
    assert_ne!(batch.payload.run_id, context_id);
    assert_eq!(batch.payload.schema_version, "1.1.0");
    assert_eq!(batch.payload.version, "1.1.0");
}

#[test]
fn run_batch_includes_the_cross_run_causal_ancestor() {
    let (db, _wcs) = setup();
    let context_id = golden_scenario(&db);
    let env = project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::WorkRun,
            id: "wr-1".to_string(),
        },
    )
    .unwrap();

    // The work run's two own events PLUS the cancellation ancestor â€”
    // in durable seq order.
    let ids: Vec<&str> = env
        .payload
        .events
        .iter()
        .map(|event| event.id.as_str())
        .collect();
    assert_eq!(ids, vec!["ev-cancel", "ev-run-start", "ev-interrupted"]);

    // The ancestor is included VERBATIM: its own correlation (the
    // cancel request) and actor (the human) are intact.
    let ancestor = &env.payload.events[0];
    assert_eq!(ancestor.id, "ev-cancel");
    assert_eq!(ancestor.correlation_id, "req-cancel");
    assert_eq!(
        ancestor.actor.kind,
        prometheos_lite::workflow::soma::event::ActorKind::Human
    );

    // The interruption's parent resolves WITHIN the batch, and the full
    // vendored audit gate passed (the projection returned, not Audit).
    let interrupted = &env.payload.events[2];
    assert_eq!(interrupted.parents, vec!["ev-cancel".to_string()]);

    // The cancel request is ALSO its own run â€” projecting it yields
    // just its own event (a root with no parents outside itself).
    let cancel_run = project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::Request,
            id: "req-cancel".to_string(),
        },
    )
    .unwrap();
    assert_eq!(cancel_run.payload.events.len(), 1);
    assert_eq!(cancel_run.payload.events[0].id, "ev-cancel");
}

#[test]
fn differently_typed_run_keys_with_equal_strings_never_merge() {
    let (db, _wcs) = setup();
    let context_id = "ctx-typed".to_string();
    persist_context(&db, &context_id);
    let authority = JournalContext::work_authority(
        prometheos_lite::work::types::AutonomyLevel::Review,
        Default::default(),
    );

    // A work run "shared-1" and a GRAPH run "shared-1": equal strings,
    // different identity types. They must NEVER merge into one run.
    let work_journal = JournalContext::for_work_run(
        "user-1",
        "req-a".to_string(),
        "shared-1".to_string(),
        authority.clone(),
    );
    let graph_base = JournalContext::for_request("user-1", "req-b".to_string(), authority);
    let graph_envelope = graph_base.graph_run_envelope("shared-1".to_string(), None);

    for (id, envelope) in [
        ("ev-work", work_journal.event_envelope(None)),
        ("ev-graph", graph_envelope),
    ] {
        let event = WorkContextEvent {
            id: id.to_string(),
            work_context_id: context_id.clone(),
            event_type: "status_changed".to_string(),
            data: serde_json::json!({ "from": "Pending", "to": "InProgress" }),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-10-02T12:00:00+00:00")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        record_event_conn(db.conn(), &event, &envelope).unwrap();
    }

    let keys = recorded_run_keys(&db, &context_id).unwrap();
    assert_eq!(keys.len(), 2, "the equal strings must not merge: {keys:?}");
    assert!(keys.contains(&RunKey {
        kind: RunKeyKind::WorkRun,
        id: "shared-1".to_string()
    }));
    assert!(keys.contains(&RunKey {
        kind: RunKeyKind::GraphRun,
        id: "shared-1".to_string()
    }));

    // Each run projects separately, one event each.
    for (kind, expected_id) in [
        (RunKeyKind::WorkRun, "ev-work"),
        (RunKeyKind::GraphRun, "ev-graph"),
    ] {
        let env = project_run_work_event_batch(
            &db,
            &context_id,
            &RunKey {
                kind,
                id: "shared-1".to_string(),
            },
        )
        .unwrap();
        assert_eq!(env.payload.events.len(), 1);
        assert_eq!(env.payload.events[0].id, expected_id);
    }
}

#[test]
fn run_batch_fails_closed_on_unresolvable_parent() {
    let (db, _wcs) = setup();
    let context_id = "ctx-dangling".to_string();
    persist_context(&db, &context_id);
    let journal = JournalContext::internal_system(
        "req-dangling".to_string(),
        JournalContext::work_authority(
            prometheos_lite::work::types::AutonomyLevel::Review,
            Default::default(),
        ),
    );
    let event = WorkContextEvent {
        id: "ev-dangling".to_string(),
        work_context_id: context_id.clone(),
        event_type: "status_changed".to_string(),
        data: serde_json::json!({}),
        created_at: chrono::DateTime::parse_from_rfc3339("2026-10-02T12:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc),
    };
    // Write invariants pass (parent reality is the caller's contract);
    // the PROJECTION must fail closed on the dangling reference.
    record_event_conn(
        db.conn(),
        &event,
        &journal.event_envelope(Some("ev-nowhere".to_string())),
    )
    .unwrap();

    match project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::Request,
            id: "req-dangling".to_string(),
        },
    ) {
        Err(ProjectionError::Unsupported { reason, .. }) => {
            assert!(reason.contains("ev-nowhere"), "{reason}")
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn run_batch_fails_closed_on_ancestor_cycle() {
    let (db, _wcs) = setup();
    let context_id = "ctx-cycle".to_string();
    persist_context(&db, &context_id);
    let journal = JournalContext::internal_system(
        "req-cycle".to_string(),
        JournalContext::work_authority(
            prometheos_lite::work::types::AutonomyLevel::Review,
            Default::default(),
        ),
    );
    for (id, parent) in [("ev-a", Some("ev-b")), ("ev-b", Some("ev-a"))] {
        let event = WorkContextEvent {
            id: id.to_string(),
            work_context_id: context_id.clone(),
            event_type: "status_changed".to_string(),
            data: serde_json::json!({}),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-10-02T12:00:00+00:00")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        record_event_conn(
            db.conn(),
            &event,
            &journal.event_envelope(parent.map(str::to_string)),
        )
        .unwrap();
    }

    match project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::Request,
            id: "req-cycle".to_string(),
        },
    ) {
        // The vendored audit is the cycle authority: the batch is
        // refused, never emitted dirty.
        Err(ProjectionError::Audit(diags)) => {
            assert!(
                diags.iter().any(|d| d.code == "SOMA-EVT-0002"),
                "expected the cyclic-parents diagnostic: {diags:?}"
            );
        }
        other => panic!("expected Audit, got {other:?}"),
    }
}

#[test]
fn unknown_run_key_fails_closed() {
    let (db, _wcs) = setup();
    let context_id = golden_scenario(&db);
    match project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::WorkRun,
            id: "wr-nope".to_string(),
        },
    ) {
        Err(ProjectionError::Unsupported { reason, .. }) => {
            assert!(reason.contains("wr-nope"), "{reason}")
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn read_model_rebuild_equals_the_per_run_union() {
    let (db, _wcs) = setup();
    let context_id = golden_scenario(&db);

    // The rebuild: enumerate the recorded run keys, project each batch.
    let keys = recorded_run_keys(&db, &context_id).unwrap();
    assert_eq!(keys.len(), 2, "the work run and the cancel request");
    let mut rebuilt: Vec<String> = Vec::new();
    for key in &keys {
        let env = project_run_work_event_batch(&db, &context_id, key).unwrap();
        for event in &env.payload.events {
            rebuilt.push(event.id.clone());
        }
    }

    // The DEDUPLICATED union of run batches == the context's full
    // verified journal — no gaps. The shared causal ancestor
    // (`ev-cancel`) legitimately appears in BOTH batches: it is the
    // work run's recorded causal history AND the cancel request's own
    // event; consumers dedup by event id (the stable identity).
    let mut sorted = rebuilt.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        3,
        "the union (deduplicated by event id) is the complete journal: {sorted:?}"
    );
    assert_eq!(
        rebuilt.len(),
        4,
        "the cancel ancestor appears in both run batches by design"
    );
    let mut expected: Vec<String> = vec![
        "ev-cancel".to_string(),
        "ev-run-start".to_string(),
        "ev-interrupted".to_string(),
    ];
    expected.sort_unstable();
    assert_eq!(sorted, expected);
}

/// Byte-lock the deterministic cancelled-work-run scenario: the batch
/// envelope's canonical bytes are pinned under
/// `tests/fixtures/soma-event-projection/` (+ .sha256) so any drift in
/// the mapping, canonical renderer, or bundle is detected. The scenario
/// is fully deterministic (fixed ids, fixed RFC 3339 timestamps, fixed
/// envelopes, fresh in-memory seq 1..3).
#[test]
fn cancelled_work_run_batch_matches_the_locked_golden_bytes() {
    let (db, _wcs) = setup();
    let context_id = golden_scenario(&db);
    let env = project_run_work_event_batch(
        &db,
        &context_id,
        &RunKey {
            kind: RunKeyKind::WorkRun,
            id: "wr-1".to_string(),
        },
    )
    .unwrap();
    let actual = env.canonical_bytes().expect("the envelope canonicalizes");

    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/soma-event-projection"
    );
    let golden_path = format!("{dir}/cancelled-work-run.batch.canonical.json");
    let digest_path = format!("{dir}/cancelled-work-run.batch.sha256");
    let golden = std::fs::read(&golden_path)
        .unwrap_or_else(|e| panic!("golden missing ({e}) — regenerate deterministically"));
    assert_eq!(
        actual, golden,
        "the projected batch diverges from the locked golden bytes"
    );
    let locked_digest = std::fs::read_to_string(&digest_path)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        prometheos_lite::workflow::soma::canonical::sha256_hex(&actual),
        locked_digest,
        "the golden digest lock diverges"
    );
}
