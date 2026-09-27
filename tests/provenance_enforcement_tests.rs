//! Slice 1A (#132) regressions: durable provenance enforcement at the
//! database boundary, tamper detection on read, the interruptionÃ¢â€ â€™cancel
//! causation reference, and legacy-row states. These run as the smoke
//! chain's gate and in the core suite.

use std::sync::Arc;

use rusqlite::params;

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::ProvenanceState;
use prometheos_lite::db::repository::WorkContextEventOperations;
use prometheos_lite::work::provenance::JournalContext;
use prometheos_lite::work::service::WorkContextService;
use prometheos_lite::work::types::{AutonomyLevel, WorkDomain};

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
        "Provenance test".to_string(),
        WorkDomain::General,
        "goal".to_string(),
        &journal,
    )
    .unwrap()
}

#[test]
fn database_refuses_new_inserts_without_complete_provenance() {
    let (db, _wcs) = setup();
    let raw = r#"{"schemaVersion":"1.0.0"}"#;

    // Missing provenance_json entirely.
    let err = db.conn().execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at, source_digest, run_id, correlation_id)
         VALUES ('ev-x', 'ctx-x', 'probe', '{}', '2026-01-01T00:00:00Z', 'd', 'r', 'c')",
        [],
    );
    assert!(err.is_err(), "insert without provenance_json must abort");

    // Missing source_digest.
    let err = db.conn().execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at, provenance_json, run_id, correlation_id)
         VALUES ('ev-x', 'ctx-x', 'probe', '{}', '2026-01-01T00:00:00Z', ?, 'r', 'c')",
        params![raw],
    );
    assert!(err.is_err(), "insert without source_digest must abort");

    // Missing run_id.
    let err = db.conn().execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at, provenance_json, source_digest, correlation_id)
         VALUES ('ev-x', 'ctx-x', 'probe', '{}', '2026-01-01T00:00:00Z', ?, 'd', 'c')",
        params![raw],
    );
    assert!(err.is_err(), "insert without run_id must abort");

    // Missing correlation_id.
    let err = db.conn().execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at, provenance_json, source_digest, run_id)
         VALUES ('ev-x', 'ctx-x', 'probe', '{}', '2026-01-01T00:00:00Z', ?, 'd', 'r')",
        params![raw],
    );
    assert!(err.is_err(), "insert without correlation_id must abort");
}

#[test]
fn journal_rows_are_append_only_at_the_database_boundary() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Any UPDATE of a journal row aborts Ã¢â‚¬â€ entire rows, not just
    // provenance.
    let err = db.conn().execute(
        "UPDATE work_context_events SET data = '{}' WHERE work_context_id = ?1",
        params![context.id],
    );
    assert!(err.is_err(), "any journal-row update must abort");
    let msg = format!("{:?}", err.unwrap_err());
    assert!(
        msg.contains("append-only"),
        "the refusal must name the append-only contract, got: {msg}"
    );
}

#[test]
fn tamper_detection_on_read_fails_closed() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Simulate an out-of-band corruption: drop the enforcement trigger,
    // tamper with the event data, restore the trigger.
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    db.conn()
        .execute(
            "UPDATE work_context_events SET data = '{\"tampered\": true}' WHERE work_context_id = ?1",
            params![context.id],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN
                 SELECT RAISE(ABORT, 'journal rows are append-only');
             END",
            [],
        )
        .unwrap();

    // The read re-verifies the source digest and fails closed.
    let err =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("tamper") || msg.contains("digest"),
        "the read must surface the tamper detection, got: {msg}"
    );
}

#[test]
fn legacy_rows_surface_as_native_legacy_unverified() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Simulate a legacy row: strip its provenance via an out-of-band
    // write (drop the triggers, null the columns, restore).
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    db.conn()
        .execute(
            "DROP TRIGGER IF EXISTS work_context_events_provenance_required",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE work_context_events
             SET provenance_json = NULL, source_digest = NULL, run_id = NULL,
                 principal_id = NULL, correlation_id = NULL
             WHERE work_context_id = ?1",
            params![context.id],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN
                 SELECT RAISE(ABORT, 'journal rows are append-only');
             END",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER IF NOT EXISTS work_context_events_provenance_required
             BEFORE INSERT ON work_context_events
             WHEN NEW.provenance_json IS NULL
               OR NEW.source_digest IS NULL
               OR NEW.run_id IS NULL
               OR NEW.correlation_id IS NULL
             BEGIN
                 SELECT RAISE(ABORT, 'journal insert requires complete provenance');
             END",
            [],
        )
        .unwrap();

    let records =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap();
    assert!(!records.is_empty());
    assert!(
        records
            .iter()
            .all(|r| matches!(r.provenance, ProvenanceState::LegacyUnverified)),
        "every stripped row must surface as the native LegacyUnverified state"
    );
}

#[test]
fn all_nine_event_types_carry_complete_provenance() {
    let (db, wcs) = setup();
    let mut context = create_context(&wcs);
    let journal = test_journal();

    let expected = [
        "context_created",
        "artifact_added",
        "decision_added",
        "status_changed",
        "phase_transition",
        "context_blocked",
        "context_unblocked",
        "context_cancelled",
        "execution_interrupted",
    ];

    // Drive one writer per event type.
    use prometheos_lite::work::artifact::{Artifact, ArtifactKind};
    let artifact = Artifact::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        ArtifactKind::Plan,
        "plan".to_string(),
        serde_json::json!({}),
        "test".to_string(),
    );
    wcs.add_artifact(&mut context, artifact, &journal).unwrap();

    use prometheos_lite::work::decision::DecisionRecord;
    let decision = DecisionRecord::new(
        "d1".to_string(),
        "decision".to_string(),
        "reason".to_string(),
        vec![],
    );
    wcs.add_decision(&mut context, decision, &journal).unwrap();

    wcs.update_status(
        &mut context,
        prometheos_lite::work::types::WorkStatus::InProgress,
        &journal,
    )
    .unwrap();

    use prometheos_lite::work::types::WorkPhase;
    wcs.update_phase(&mut context, WorkPhase::Planning, &journal)
        .unwrap();

    wcs.set_blocked_reason(&mut context, "blocked".to_string(), &journal)
        .unwrap();
    wcs.clear_blocked_reason(&mut context, &journal).unwrap();

    wcs.cancel_context(&mut context, "test", &journal).unwrap();

    // The execution_interrupted event: written through the same evidence
    // path the orchestrator uses (its parent is the cancellation event).
    let cancel_id =
        prometheos_lite::db::repository::work_context_events::cancellation_event_id_conn(
            db.conn(),
            &context.id,
        )
        .unwrap()
        .expect("cancellation event must exist after cancel_context");
    let interrupted = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "execution_interrupted".to_string(),
        serde_json::json!({"reason": "cancelled"}),
    );
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &interrupted,
        &journal.event_envelope(Some(cancel_id)),
    )
    .unwrap();

    let records =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap();
    for event_type in expected {
        let record = records
            .iter()
            .find(|r| r.event.event_type == event_type)
            .unwrap_or_else(|| panic!("event {event_type} must exist"));
        match &record.provenance {
            ProvenanceState::Verified(envelope) => {
                assert!(!envelope.run.request_id.is_empty());
                assert!(
                    !envelope.run_query_key().is_empty(),
                    "{event_type}: run identity must be present"
                );
            }
            ProvenanceState::LegacyUnverified => {
                panic!("{event_type} must carry verified provenance, not legacy")
            }
        }
    }
}

#[test]
fn execution_interrupted_references_the_durable_cancellation_event_id() {
    let (db, wcs) = setup();
    let mut context = create_context(&wcs);
    let journal = test_journal();

    // Cancel durably first (writes the context_cancelled event).
    wcs.cancel_context(&mut context, "test", &journal).unwrap();

    // Find the cancellation event id.
    let cancel_id =
        prometheos_lite::db::repository::work_context_events::cancellation_event_id_conn(
            db.conn(),
            &context.id,
        )
        .unwrap()
        .expect("the durable cancellation event must exist");

    // Record the interruption evidence with the journal Ã¢â‚¬â€ the helper
    // that mirrors the orchestrator's evidence write.
    let event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "execution_interrupted".to_string(),
        serde_json::json!({"reason": "cancelled"}),
    );
    let envelope = journal.event_envelope(Some(cancel_id.clone()));
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &event,
        &envelope,
    )
    .unwrap();

    // The recorded interruption must reference the cancellation event.
    let records =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap();
    let interrupted = records
        .iter()
        .find(|r| r.event.event_type == "execution_interrupted")
        .expect("interruption evidence must exist");
    match &interrupted.provenance {
        ProvenanceState::Verified(envelope) => {
            assert_eq!(
                envelope.causation.parent_event_id,
                Some(cancel_id),
                "the interruption must reference the durable cancellation event id"
            );
        }
        ProvenanceState::LegacyUnverified => panic!("must be verified"),
    }
}

#[test]
fn migration_is_idempotent_across_repeated_opens() {
    // Opening a database twice must converge: columns exist once,
    // triggers are singletons, no error on the second open.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("idempotent.db");
    let db_path_str = db_path.to_str().unwrap().to_string();

    {
        let _db = Db::new(&db_path_str).unwrap();
    }
    {
        let db = Db::new(&db_path_str).unwrap();
        // Verify the provenance columns exist.
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('work_context_events')
                 WHERE name IN ('provenance_json','source_digest','run_id','principal_id','correlation_id')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5, "all five provenance columns must exist");

        // Exactly one of each trigger.
        let triggers: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'trigger' AND tbl_name = 'work_context_events'
                   AND name IN ('work_context_events_provenance_required','work_context_events_append_only')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(triggers, 2, "exactly one of each trigger");
    }
    // Third open: still converges.
    {
        let _db = Db::new(&db_path_str).unwrap();
    }
}

#[test]
fn cancel_context_returns_the_exact_cancellation_event_id() {
    let (_db, wcs) = setup();
    let mut context = create_context(&wcs);
    let journal = test_journal();

    // The exact cancellation event id returned by the transactional
    // cancel — callers carry this as the causal parent (P1-4), never a
    // "latest" inference.
    let returned_id = wcs
        .cancel_context(&mut context, "test", &journal)
        .unwrap()
        .expect("first cancel must return the exact event id");
    assert!(!returned_id.is_empty());

    // The deterministic lookup agrees (exactly one, not "latest").
    let looked_up =
        prometheos_lite::db::repository::work_context_events::cancellation_event_id_conn(
            _db.conn(),
            &context.id,
        )
        .unwrap()
        .expect("exactly one cancellation event");
    assert_eq!(
        returned_id, looked_up,
        "the carried ID must be the exact durable event"
    );

    // Idempotent re-cancel returns None — no second event.
    let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
    let re_cancel = wcs
        .cancel_context(&mut snapshot, "again", &journal)
        .unwrap();
    assert!(
        re_cancel.is_none(),
        "idempotent re-cancel returns no event id"
    );
}

#[test]
fn column_drift_between_envelope_and_flat_columns_fails_closed_on_read() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Simulate an out-of-band column tamper: drop the append-only
    // trigger, change the run_id column without changing the envelope,
    // restore the trigger.
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    db.conn()
        .execute(
            "UPDATE work_context_events SET run_id = 'drifted-run' WHERE work_context_id = ?1",
            rusqlite::params![context.id],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN
                 SELECT RAISE(ABORT, 'journal rows are append-only');
             END",
            [],
        )
        .unwrap();

    // The read revalidates the flat columns against the envelope and
    // fails closed — the drifted column surfaces as an error.
    let err =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("column drift") || msg.contains("drift"),
        "the read must surface the column drift, got: {msg}"
    );
}

#[test]
fn malformed_provenance_envelope_fails_closed_on_read() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Simulate an out-of-band envelope corruption: drop the trigger,
    // write garbage into provenance_json, restore.
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    db.conn()
        .execute(
            "UPDATE work_context_events SET provenance_json = '{not-json' WHERE work_context_id = ?1",
            rusqlite::params![context.id],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN
                 SELECT RAISE(ABORT, 'journal rows are append-only');
             END",
            [],
        )
        .unwrap();

    let err =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap_err();
    let msg = format!("{:?}", err);
    // The read refuses the malformed envelope (P1-3: non-null garbage is
    // not accepted, even though the trigger only checks for NULL).
    assert!(
        msg.contains("envelope") || msg.contains("parse") || msg.contains("provenance"),
        "the read must surface the malformed-envelope refusal, got: {msg}"
    );
}

#[test]
fn graph_decision_event_carries_complete_provenance() {
    let (db, wcs) = setup();
    let journal = test_journal();
    let context = create_context(&wcs);

    // Simulate the decide-path journal write: the graph_decision event
    // carries the graph-run identity via graph_run_envelope (P1-1: the
    // event inventory explicitly includes graph_decision).
    let graph_run_id = "graph-run-test-1";
    let event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "graph_decision".to_string(),
        serde_json::json!({"runId": graph_run_id, "newDigest": "abc123"}),
    );
    let envelope = journal.graph_run_envelope(graph_run_id.to_string(), None);
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &event,
        &envelope,
    )
    .unwrap();

    let records =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap();
    let decision = records
        .iter()
        .find(|r| r.event.event_type == "graph_decision")
        .expect("graph_decision event must exist");
    match &decision.provenance {
        ProvenanceState::Verified(envelope) => {
            assert_eq!(
                envelope.run.graph_run_id,
                Some(graph_run_id.to_string()),
                "the graph-run identity must be preserved in the envelope"
            );
        }
        ProvenanceState::LegacyUnverified => panic!("must be verified"),
    }
}
