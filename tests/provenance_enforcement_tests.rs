//! Slice 1A (#132) regressions: durable provenance enforcement at the
//! database boundary, tamper detection on read, the interruptionâ†’cancel
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

    // Any UPDATE of a journal row aborts â€” entire rows, not just
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
        prometheos_lite::db::repository::work_context_events::latest_cancellation_event_id_conn(
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
        prometheos_lite::db::repository::work_context_events::latest_cancellation_event_id_conn(
            db.conn(),
            &context.id,
        )
        .unwrap()
        .expect("the durable cancellation event must exist");

    // Record the interruption evidence with the journal â€” the helper
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
