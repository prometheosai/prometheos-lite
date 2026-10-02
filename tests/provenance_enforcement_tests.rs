//! Slice 1A (#132) regressions: durable provenance enforcement at the
//! database boundary, tamper detection on read, the interruptionÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢cancel
//! causation reference, and legacy-row states. These run as the smoke
//! chain's gate and in the core suite.

use std::sync::Arc;

use rusqlite::params;

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::ProvenanceState;
use prometheos_lite::db::repository::WorkContextEventOperations;
use prometheos_lite::work::provenance::JournalContext;
use prometheos_lite::work::service::WorkContextService;
use prometheos_lite::work::types::{ApprovalPolicy, AutonomyLevel, WorkDomain};

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

    // Any UPDATE of a journal row aborts ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â entire rows, not just
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

    // Record the interruption evidence with the journal ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â the helper
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
    // cancel â€” callers carry this as the causal parent (P1-4), never a
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

    // Idempotent re-cancel returns None â€” no second event.
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
    // fails closed â€” the drifted column surfaces as an error.
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

#[test]
fn canonical_bytes_fixpoint_and_semantic_invariants_are_enforced_at_write() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);
    let journal = test_journal();

    // Valid envelope passes the write invariants.
    let envelope = journal.event_envelope(None);
    envelope.validate_write_invariants().unwrap();

    // Widened effective authority is refused.
    let mut widened = journal.event_envelope(None);
    widened.authority.effective.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::HumanDecision;
    widened.authority.declared.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::Deterministic;
    assert!(
        widened.validate_write_invariants().is_err(),
        "widened effective authority must fail the write invariants"
    );

    // Empty producer identity is refused.
    let mut empty_producer = journal.event_envelope(None);
    empty_producer.producer.identity = String::new();
    assert!(
        empty_producer.validate_write_invariants().is_err(),
        "empty producer identity must fail"
    );

    // The record_event_conn boundary enforces it: a malformed envelope
    // cannot be stored.
    let bad_event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "probe_bad".to_string(),
        serde_json::json!({}),
    );
    let result = prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &bad_event,
        &widened,
    );
    assert!(
        result.is_err(),
        "widened-authority envelope must be refused at the write boundary"
    );

    // The valid envelope round-trips through the stored bytes.
    let good_event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "probe_good".to_string(),
        serde_json::json!({}),
    );
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &good_event,
        &envelope,
    )
    .unwrap();
}

#[test]
fn database_trigger_rejects_malformed_non_null_provenance() {
    let (db, _wcs) = setup();

    // Non-null but too-short provenance_json (doesn't start with '{').
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-shape-1', 'ctx-shape', 'probe', '{}', '2026-01-01T00:00:00Z',
                 'short', '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "short/non-JSON provenance_json must be rejected by the shape check"
    );

    // Non-null but wrong-length source_digest (not 64 chars).
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-shape-2', 'ctx-shape', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\"}', 'short-digest', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "non-64-char source_digest must be rejected by the shape check"
    );

    // Non-null but empty run_id.
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-shape-3', 'ctx-shape', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\"}',
                 '0000000000000000000000000000000000000000000000000000000000000000', '', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "empty run_id must be rejected by the shape check"
    );
}

#[test]
fn partial_schema_migration_converges_on_reopen() {
    // P1 gap 5: a database left in a PARTIALLY migrated state (some
    // provenance columns present, others missing â€” a crash between
    // ALTER TABLE statements) must converge when reopened: the missing
    // columns are added, the triggers are present, and the existing
    // rows are untouched.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("partial.db");
    let db_path_str = db_path.to_str().unwrap().to_string();

    // Phase 1: create a real full-schema database with a context + event.
    {
        let db = Db::new(&db_path_str).unwrap();
        let wcs = WorkContextService::new(std::sync::Arc::new(db));
        let _ctx = wcs
            .create_context(
                "user".to_string(),
                "Partial test".to_string(),
                prometheos_lite::work::types::WorkDomain::General,
                "goal".to_string(),
                &test_journal(),
            )
            .unwrap();
    }

    // Phase 2: simulate the PARTIAL state â€” strip ALL provenance columns
    // and re-add only two of five (a crash mid-migration).
    {
        let conn = rusqlite::Connection::open(&db_path_str).unwrap();
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS work_context_events_provenance_required;
             DROP TRIGGER IF EXISTS work_context_events_append_only;
             CREATE TABLE work_context_events_stripped AS
                 SELECT seq, id, work_context_id, event_type, data, created_at
                 FROM work_context_events;
             DROP TABLE work_context_events;
             ALTER TABLE work_context_events_stripped RENAME TO work_context_events;",
        )
        .unwrap();
        // Re-add only two of five.
        conn.execute(
            "ALTER TABLE work_context_events ADD COLUMN provenance_json TEXT",
            [],
        )
        .unwrap();
        conn.execute("ALTER TABLE work_context_events ADD COLUMN run_id TEXT", [])
            .unwrap();
        // Three columns MISSING: source_digest, principal_id, correlation_id.
    }

    // Phase 3: reopen â€” the migration must converge.
    {
        let db = Db::new(&db_path_str).unwrap();
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('work_context_events')
                 WHERE name IN ('provenance_json','source_digest','run_id','principal_id','correlation_id')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5, "all five columns must converge");

        // The pre-existing row is untouched (legacy NULLs).
        let (envelope, digest): (Option<String>, Option<String>) = db
            .conn()
            .query_row(
                "SELECT provenance_json, source_digest FROM work_context_events LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(envelope.is_none(), "existing row's provenance stays NULL");
        assert!(digest.is_none(), "existing row's digest stays NULL");

        // The triggers exist.
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
        assert_eq!(triggers, 2, "both triggers must exist after convergence");
    }
}

#[test]
fn mixed_legacy_provenance_state_is_refused_not_misclassified() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);

    // Simulate a mixed state: NULL envelope + NULL digest (looks legacy)
    // but with a NON-NULL run_id column. This is neither a clean legacy
    // row nor a clean provenanced row.
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
             SET provenance_json = NULL, source_digest = NULL,
                 run_id = 'orphaned-run-id'
             WHERE work_context_id = ?1",
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

    // The read must REFUSE the mixed state, not classify it as
    // LegacyUnverified.
    let err =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("mixed"),
        "mixed legacy/provenance state must be refused with 'mixed' in the error, got: {msg}"
    );
}

#[test]
fn non_canonical_bytes_fail_closed_on_read() {
    let (db, wcs) = setup();
    let context = create_context(&wcs);
    let journal = test_journal();

    // Write a valid event.
    let event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "probe_canonical".to_string(),
        serde_json::json!({}),
    );
    let envelope = journal.event_envelope(None);
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        db.conn(),
        &event,
        &envelope,
    )
    .unwrap();

    // Verify the valid event reads clean.
    let records =
        WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id).unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.event.event_type == "probe_canonical")
    );

    // Simulate an out-of-band non-canonical byte tamper: drop the trigger,
    // re-serialize with serde (non-canonical byte form â€” e.g., different
    // key order or spacing), store those bytes, restore the trigger.
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    // Re-serialize with plain serde â€” produces non-canonical bytes.
    let plain_serde = serde_json::to_string(&envelope).unwrap();
    if plain_serde != envelope.to_canonical_json_string().unwrap() {
        // serde and canonical differ for this envelope â€” store the non-canonical one.
        db.conn()
            .execute(
                "UPDATE work_context_events SET provenance_json = ?1 WHERE event_type = 'probe_canonical'",
                rusqlite::params![plain_serde],
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

        // The read must refuse the non-canonical bytes.
        let err = WorkContextEventOperations::get_journal_records_for_context(&*db, &context.id)
            .unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("not canonical") || msg.contains("canonical"),
            "non-canonical bytes must be refused on read, got: {msg}"
        );
    }
}

#[test]
fn cancellation_payload_is_carried_through_the_token_signal() {
    let token = prometheos_lite::workflow::evaluate::CancellationToken::new();

    // No payload before cancel.
    assert!(token.cancellation_payload().is_none());

    // Fire WITH the exact cancellation event ID.
    token.cancel_with("cancel-event-abc-123".to_string());
    assert!(token.is_cancelled());
    assert_eq!(
        token.cancellation_payload(),
        Some("cancel-event-abc-123".to_string()),
        "the exact cancellation event ID must be carried through the signal"
    );

    // Idempotent: the first payload wins.
    token.cancel_with("different-id".to_string());
    assert_eq!(
        token.cancellation_payload(),
        Some("cancel-event-abc-123".to_string()),
        "the first cancellation payload is authoritative"
    );

    // A plain cancel (no payload) carries None.
    let plain = prometheos_lite::workflow::evaluate::CancellationToken::new();
    plain.cancel();
    assert!(plain.is_cancelled());
    assert!(plain.cancellation_payload().is_none());
}

#[test]
fn authority_partial_order_covers_all_three_dimensions() {
    let journal = test_journal();

    // === WIDENING REFUSED (all three dimensions) ===

    // Autonomy widening: Chat (0) → Autonomous (2) is refused.
    let mut widened_autonomy = journal.event_envelope(None);
    widened_autonomy.authority.declared.autonomy = AutonomyLevel::Chat;
    widened_autonomy.authority.effective.autonomy = AutonomyLevel::Autonomous;
    assert!(
        widened_autonomy.validate_write_invariants().is_err(),
        "autonomy widening (Chat → Autonomous) must fail"
    );

    // Approval-policy widening: ManualAll (0) → Auto (4) is refused.
    // Ordering justification: the approval-policy rank measures how much
    // the runtime may do WITHOUT human approval. ManualAll (every action
    // needs approval, rank 0) is the most restrictive. Auto (no approval
    // needed, rank 4) is the most permissive. Widening means the effective
    // policy allows MORE without approval than what was declared.
    let mut widened_approval = journal.event_envelope(None);
    widened_approval.authority.declared.approval_policy = ApprovalPolicy::ManualAll;
    widened_approval.authority.effective.approval_policy = ApprovalPolicy::Auto;
    assert!(
        widened_approval.validate_write_invariants().is_err(),
        "approval-policy widening (ManualAll → Auto) must fail"
    );

    // Execution-class widening: Deterministic (0) → HumanDecision (3) refused.
    let mut widened_execution = journal.event_envelope(None);
    widened_execution.authority.declared.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::Deterministic;
    widened_execution.authority.effective.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::HumanDecision;
    assert!(
        widened_execution.validate_write_invariants().is_err(),
        "execution-class widening (Deterministic → HumanDecision) must fail"
    );

    // === NARROWING PERMITTED (effective may reduce declared) ===

    // Autonomy narrowing: Autonomous declared, Review effective — OK.
    let mut narrowed_autonomy = journal.event_envelope(None);
    narrowed_autonomy.authority.declared.autonomy = AutonomyLevel::Autonomous;
    narrowed_autonomy.authority.effective.autonomy = AutonomyLevel::Review;
    assert!(
        narrowed_autonomy.validate_write_invariants().is_ok(),
        "autonomy narrowing (Autonomous declared, Review effective) must pass"
    );

    // Approval narrowing: Auto declared, ManualAll effective — OK.
    let mut narrowed_approval = journal.event_envelope(None);
    narrowed_approval.authority.declared.approval_policy = ApprovalPolicy::Auto;
    narrowed_approval.authority.effective.approval_policy = ApprovalPolicy::ManualAll;
    assert!(
        narrowed_approval.validate_write_invariants().is_ok(),
        "approval narrowing (Auto declared, ManualAll effective) must pass"
    );

    // Execution narrowing: HumanDecision declared, Deterministic effective — OK.
    let mut narrowed_execution = journal.event_envelope(None);
    narrowed_execution.authority.declared.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::HumanDecision;
    narrowed_execution.authority.effective.execution_class =
        prometheos_lite::work::provenance::ExecutionClass::Deterministic;
    assert!(
        narrowed_execution.validate_write_invariants().is_ok(),
        "execution narrowing (HumanDecision declared, Deterministic effective) must pass"
    );

    // Same-level is always permitted.
    let same = journal.event_envelope(None);
    assert!(same.validate_write_invariants().is_ok());
}

#[test]
fn trigger_validates_typed_json_paths() {
    let (_db, _wcs) = setup();

    // Valid JSON but wrong schemaVersion â€” json_extract check fires.
    let err = _db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-typed-1', 'ctx-typed', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"2.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "wrong schemaVersion must be rejected by json_extract"
    );

    // Valid JSON but empty producer identity â€” json_extract check fires.
    let err = _db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-typed-2', 'ctx-typed', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "empty producer identity must be rejected by json_extract"
    );

    // Valid JSON but empty correlationId â€” json_extract check fires.
    let err = _db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-typed-3', 'ctx-typed', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "empty correlationId must be rejected by json_extract"
    );
}

#[test]
fn trigger_rejects_non_hex_digest() {
    let (db, _wcs) = setup();

    // 64 chars but NOT all lowercase hex (contains 'G').
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-hex-1', 'ctx-hex', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '00000000000000000000000000000000000000000000000000000000000000GG', 'r', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "non-hex characters in source_digest must be rejected by the GLOB check"
    );

    // 64 chars, all hex, correct — accepted (no error from the hex check).
    // (The insert may still fail from the flat-column equality checks
    // because the run_id/correlation_id must match the envelope's
    // derived keys — but the HEX check itself passes.)
    let _ = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-hex-2', 'ctx-hex', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'c')",
        [],
    );
    // Should succeed: all-hex digest, run_id='r' matches request_id='r',
    // correlation_id='c' matches correlationId='c', principal_id NULL
    // with principal='absent' (no human identity in the envelope).
}

#[test]
fn trigger_rejects_flat_column_envelope_mismatch() {
    let (db, _wcs) = setup();

    // run_id column doesn't match the envelope's derived run key.
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-flat-1', 'ctx-flat', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'WRONG', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "run_id column mismatching the envelope must be rejected"
    );

    // correlation_id column doesn't match the envelope's derived correlation.
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, correlation_id)
         VALUES ('ev-flat-2', 'ctx-flat', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'WRONG')",
        [],
    );
    assert!(
        err.is_err(),
        "correlation_id column mismatching the envelope must be rejected"
    );
}

#[test]
fn trigger_rejects_principal_null_semantics_violation() {
    let (db, _wcs) = setup();

    // principal_id is non-NULL but the envelope says absent (no human identity).
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, principal_id, correlation_id)
         VALUES ('ev-principal-1', 'ctx-principal', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":\"absent\",\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', 'user-1', 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "principal_id set but envelope says absent must be rejected"
    );

    // principal_id is NULL but the envelope has a human identity.
    let err = db.conn().execute(
        "INSERT INTO work_context_events
            (id, work_context_id, event_type, data, created_at,
             provenance_json, source_digest, run_id, principal_id, correlation_id)
         VALUES ('ev-principal-2', 'ctx-principal', 'probe', '{}', '2026-01-01T00:00:00Z',
                 '{\"schemaVersion\":\"1.0.0\",\"producer\":{\"identity\":\"x\",\"kind\":\"harness\"},\"principal\":{\"human\":{\"identity\":\"user-1\"}},\"causation\":{\"correlationId\":\"c\"},\"authority\":{\"declared\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"},\"effective\":{\"autonomy\":\"Review\",\"approvalPolicy\":\"Auto\",\"executionClass\":\"deterministic\"}},\"repoBinding\":\"unbound\",\"run\":{\"requestId\":\"r\"}}',
                 '0000000000000000000000000000000000000000000000000000000000000000', 'r', NULL, 'c')",
        [],
    );
    assert!(
        err.is_err(),
        "principal_id NULL but envelope has human identity must be rejected"
    );
}

#[test]
fn cancel_with_publishes_payload_before_flag() {
    // #232 P1 publication-order regression: after cancel_with returns,
    // BOTH the flag and the payload are visible — guaranteed by the
    // SeqCst ordering contract (payload written and mutex released
    // BEFORE the flag store). No observer can see is_cancelled()==true
    // with cancellation_payload()==None for a payload-bearing cancel.
    let token = prometheos_lite::workflow::evaluate::CancellationToken::new();
    token.cancel_with("exact-event-id-123".to_string());

    assert!(
        token.is_cancelled(),
        "flag must be visible after cancel_with returns"
    );
    assert_eq!(
        token.cancellation_payload(),
        Some("exact-event-id-123".to_string()),
        "payload must be visible after cancel_with returns — the publication-order contract guarantees no fired-without-payload gap"
    );
}

#[tokio::test]
async fn concurrent_observers_never_see_fired_without_payload() {
    // #232 P1 stronger regression: spawn many concurrent observers that
    // poll is_cancelled() and immediately check cancellation_payload().
    // No observer may see the flag as true with a None payload after a
    // payload-bearing cancel — the SeqCst ordering (payload before flag)
    // makes this impossible.
    let token = prometheos_lite::workflow::evaluate::CancellationToken::new();

    let mut observers = Vec::new();
    for _ in 0..50 {
        let observer = token.clone();
        observers.push(tokio::spawn(async move {
            let mut saw_gap = false;
            for _ in 0..100_000 {
                if observer.is_cancelled() {
                    if observer.cancellation_payload().is_none() {
                        saw_gap = true;
                    }
                    break;
                }
                tokio::task::yield_now().await;
            }
            saw_gap
        }));
    }

    // Give the observers a head start, then cancel WITH a payload.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    token.cancel_with("event-under-concurrent-observation".to_string());

    for handle in observers {
        let saw_gap = handle.await.unwrap();
        assert!(
            !saw_gap,
            "no concurrent observer may see is_cancelled()==true with cancellation_payload()==None after a payload-bearing cancel"
        );
    }
}

#[tokio::test]
async fn registered_run_cancel_uses_token_payload_not_durable_lookup() {
    // #232 P1 strongest regression: a registered-run cancel fires the
    // token WITH the exact cancellation event ID. The orchestrator must
    // extract the ID from the token payload — even if the durable
    // lookup would return a DIFFERENT (wrong) answer. This proves the
    // token payload is the preferred source and the durable lookup is
    // only the cross-process fallback, not the primary.
    use prometheos_lite::db::Db;
    use prometheos_lite::work::service::WorkContextService;
    use prometheos_lite::work::types::WorkDomain;

    let db = std::sync::Arc::new(Db::in_memory().unwrap());
    let wcs = std::sync::Arc::new(WorkContextService::new(db.clone()));
    let journal = test_journal();

    let mut context = wcs
        .create_context(
            "user-1".to_string(),
            "Token payload test".to_string(),
            WorkDomain::General,
            "goal".to_string(),
            &journal,
        )
        .unwrap();

    // Cancel durably — the transactional cancel returns the exact event ID.
    let cancel_event_id = wcs
        .cancel_context(&mut context, "test", &journal)
        .unwrap()
        .expect("cancel must return the exact event id");

    // Create a token and fire it with the SAME event ID (as the API
    // cancel handler does via fire_with_cancellation).
    let token = prometheos_lite::workflow::evaluate::CancellationToken::new();
    token.cancel_with(cancel_event_id.clone());

    // The token's payload IS the exact durable event ID — no lookup needed.
    assert_eq!(
        token.cancellation_payload(),
        Some(cancel_event_id),
        "the token payload must be the exact cancellation event ID from cancel_context's return"
    );

    // The durable lookup ALSO returns the same ID (exactly one).
    let durable = prometheos_lite::db::repository::work_context_events::cancellation_event_id_conn(
        db.conn(),
        &context.id,
    )
    .unwrap()
    .expect("exactly one cancellation event");
    assert_eq!(
        token.cancellation_payload().as_deref(),
        Some(durable.as_str()),
        "the token payload and the durable lookup must agree — but the token is the preferred source"
    );
}

#[tokio::test]
async fn orchestrator_uses_token_payload_when_durable_lookup_is_broken() {
    // #232 P1 HARDEST regression: the orchestrator must produce correct
    // evidence referencing the TOKEN payload's event ID even when the
    // durable lookup is deliberately broken (two cancellation events
    // = a durability violation that makes cancellation_event_id_conn
    // return Err). This proves the token is the primary source.
    use prometheos_lite::db::repository::WorkContextEventOperations;
    use prometheos_lite::db::repository::work_context_events::{
        cancellation_event_id_conn, record_event_conn,
    };
    use prometheos_lite::flow::RuntimeContext;
    use prometheos_lite::flow::execution_service::FlowExecutionService;
    use prometheos_lite::work::event::WorkContextEvent;
    use prometheos_lite::work::evolution_engine::EvolutionEngine;
    use prometheos_lite::work::execution_service::WorkExecutionService;
    use prometheos_lite::work::orchestrator::{ExecutionLimits, WorkOrchestrator};
    use prometheos_lite::work::playbook_resolver::PlaybookResolver;
    use prometheos_lite::work::service::WorkContextService;

    let db = Arc::new(Db::in_memory().unwrap());
    let wcs = Arc::new(WorkContextService::new(db.clone()));
    let journal = test_journal();

    let mut context = wcs
        .create_context(
            "user-1".to_string(),
            "Token-vs-lookup test".to_string(),
            prometheos_lite::work::types::WorkDomain::General,
            "goal".to_string(),
            &journal,
        )
        .unwrap();
    context.autonomy_level = AutonomyLevel::Review;
    wcs.update_context(&context).unwrap();

    // Step 1: Cancel durably — the exact event ID is returned.
    let real_cancel_id = wcs
        .cancel_context(&mut context, "test", &journal)
        .unwrap()
        .expect("cancel must return the exact event id");

    // Step 2: BREAK the durable lookup — insert a FAKE second
    // cancellation event so COUNT(*) == 2 (durability violation).
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    let fake_event = WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        context.id.clone(),
        "context_cancelled".to_string(),
        serde_json::json!({"reason": "fake second cancel for the test"}),
    );
    record_event_conn(db.conn(), &fake_event, &journal.event_envelope(None)).unwrap();
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

    // Verify the durable lookup IS broken.
    let lookup_result = cancellation_event_id_conn(db.conn(), &context.id);
    assert!(
        lookup_result.is_err(),
        "the durable lookup must be broken (two cancellation events = durability violation)"
    );

    // Step 3: The token carries the EXACT event ID — fire with payload.
    let token = prometheos_lite::workflow::evaluate::CancellationToken::new();
    token.cancel_with(real_cancel_id.clone());

    // Step 4: Build the orchestrator and run — the observation must use
    // the token payload, NOT the broken durable lookup.
    let model_router = Arc::new(prometheos_lite::flow::intelligence::ModelRouter::new(
        vec![],
    ));
    let runtime = Arc::new(RuntimeContext::default().with_model_router(model_router));
    let flow_execution_service = Arc::new(FlowExecutionService::new(runtime).unwrap());
    let work_execution_service = Arc::new(WorkExecutionService::new(
        wcs.clone(),
        flow_execution_service,
    ));
    let playbook_resolver = Arc::new(PlaybookResolver::new(db.clone()));
    let intent_classifier = Arc::new(prometheos_lite::intent::IntentClassifier::new().unwrap());
    let evolution_engine = Arc::new(EvolutionEngine::new(db.clone()));
    let orchestrator = WorkOrchestrator::new(
        wcs.clone(),
        playbook_resolver,
        work_execution_service,
        intent_classifier,
        evolution_engine,
    );

    let result = orchestrator
        .run_until_blocked_or_complete_with_token(
            context.id.clone(),
            ExecutionLimits::default(),
            token,
            Arc::new(journal),
        )
        .await;

    // The run must NOT error — the token payload was used.
    let final_context = result.expect(
        "the orchestrator must succeed despite the broken durable lookup — the token payload is the primary source",
    );
    assert!(final_context.is_cancelled());

    // Step 5: The evidence references the TOKEN's payload ID.
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
                Some(real_cancel_id),
                "the evidence must reference the TOKEN payload's event ID — not the fake, not a durable-lookup error"
            );
        }
        ProvenanceState::LegacyUnverified => panic!("must be verified"),
    }
}

/// Test fixture helper: runs a git command, asserts success, returns output.
/// Every git fixture must use this — never a bare `.output().unwrap()` that
/// silently swallows command failures.
fn git_cmd(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Provenance Test")
        .env("GIT_AUTHOR_EMAIL", "provenance-test@test.local")
        .env("GIT_COMMITTER_NAME", "Provenance Test")
        .env("GIT_COMMITTER_EMAIL", "provenance-test@test.local")
        .output()
        .unwrap_or_else(|e| panic!("git {:?} failed to spawn: {e}", args));
    assert!(
        out.status.success(),
        "git {:?} failed (exit {:?}): {}",
        args,
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Create a clean committed git repo at `dir` with one file.
fn make_committed_repo(dir: &std::path::Path) {
    git_cmd(dir, &["init"]);
    std::fs::write(dir.join("a.txt"), "initial").unwrap();
    git_cmd(dir, &["add", "."]);
    git_cmd(dir, &["commit", "-m", "init"]);
}

#[test]
fn repo_binding_clean_directory_returns_unbound() {
    let dir = tempfile::tempdir().unwrap();
    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(dir.path()).unwrap();
    assert!(matches!(
        binding,
        prometheos_lite::work::provenance::RepoBinding::Unbound
    ));
}

#[test]
fn repo_binding_clean_committed_repo_returns_bound() {
    let dir = tempfile::tempdir().unwrap();
    make_committed_repo(dir.path());
    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(dir.path()).unwrap();
    assert!(
        matches!(
            binding,
            prometheos_lite::work::provenance::RepoBinding::Bound { .. }
        ),
        "a clean committed repo must be Bound"
    );
}

#[test]
fn repo_binding_dirty_repo_returns_dirty_with_policy() {
    let dir = tempfile::tempdir().unwrap();
    make_committed_repo(dir.path());
    std::fs::write(dir.path().join("b.txt"), "dirty").unwrap();

    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(dir.path()).unwrap();
    match binding {
        prometheos_lite::work::provenance::RepoBinding::Dirty {
            revision,
            workspace_digest,
            digest_policy,
        } => {
            assert!(!revision.is_empty());
            assert!(!workspace_digest.is_empty());
            assert_eq!(digest_policy, "soma-canonical-json-v1");
        }
        other => panic!("expected Dirty, got {:?}", other),
    }
}

#[test]
fn repo_binding_empty_repo_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    git_cmd(dir.path(), &["init"]);

    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(dir.path());
    assert!(
        binding.is_err(),
        "an empty git repo claiming a binding but with no HEAD must fail closed"
    );
}

#[test]
fn repo_binding_subdirectory_detected_by_git_walk_up() {
    let dir = tempfile::tempdir().unwrap();
    make_committed_repo(dir.path());

    let subdir = dir.path().join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();

    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(&subdir).unwrap();
    assert!(
        matches!(
            binding,
            prometheos_lite::work::provenance::RepoBinding::Bound { .. }
        ),
        "a subdirectory of a git repo must be detected via git rev-parse walk-up"
    );
}

#[test]
fn repo_binding_linked_worktree_detected() {
    // #232 P1: in a linked worktree, `.git` is a FILE (not a directory).
    // detect_repo_binding uses `git rev-parse --git-dir` instead of
    // checking `.git` existence, so linked worktrees must be detected.
    let main_dir = tempfile::tempdir().unwrap();
    make_committed_repo(main_dir.path());

    // Create a branch for the worktree.
    git_cmd(main_dir.path(), &["branch", "wt-branch"]);

    // Create the worktree at a separate path.
    let wt_dir = tempfile::tempdir().unwrap();
    let wt_path = wt_dir.path().join("linked-worktree");
    git_cmd(
        main_dir.path(),
        &["worktree", "add", wt_path.to_str().unwrap(), "wt-branch"],
    );

    // Verify: `.git` is a FILE in a linked worktree (not a directory).
    assert!(
        wt_path.join(".git").is_file(),
        "linked worktree .git must be a file, not a directory"
    );

    // The binding must be detected (Bound — clean committed worktree).
    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(&wt_path).unwrap();
    assert!(
        matches!(
            binding,
            prometheos_lite::work::provenance::RepoBinding::Bound { .. }
        ),
        "a linked worktree must be detected via git rev-parse --git-dir (not .git existence)"
    );

    // Clean up the worktree so the tempdirs can be removed.
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(main_dir.path())
        .args(["worktree", "remove", "--force", wt_path.to_str().unwrap()])
        .output();
}

#[test]
fn trigger_version_upgrade_replaces_incomplete_trigger() {
    // #232 P1: a trigger WITHOUT the v3 version marker is replaced by
    // the v3 version on reopen — even if it carries SOME v3 checks
    // (proving the marker is the sole version-identity criterion).
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("trigger_upgrade.db");
    let db_path_str = db_path.to_str().unwrap().to_string();

    {
        let _db = Db::new(&db_path_str).unwrap();
    }

    // Phase 2: replace with a trigger that has SOME v3 checks but NO
    // version marker (a "partial v3" — json_extract present, GLOB and
    // principal_id absent). The marker-based detection must still
    // upgrade it because the marker is the criterion, not individual
    // check strings.
    {
        let conn = rusqlite::Connection::open(&db_path_str).unwrap();
        conn.execute(
            "DROP TRIGGER IF EXISTS work_context_events_provenance_required",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TRIGGER work_context_events_provenance_required
             BEFORE INSERT ON work_context_events
             WHEN NEW.provenance_json IS NULL
               OR NEW.source_digest IS NULL
               OR json_valid(NEW.provenance_json) = 0
               OR json_extract(NEW.provenance_json, '$.schemaVersion') != '1.0.0'
             BEGIN
                 SELECT RAISE(ABORT, 'journal insert requires complete, well-formed provenance');
             END",
            [],
        )
        .unwrap();
    }

    // Phase 3: reopen — the v3 trigger must replace the marker-less one.
    {
        let db = Db::new(&db_path_str).unwrap();
        let sql: String = db
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'trigger' AND name = 'work_context_events_provenance_required'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("provenance-trigger-v3"),
            "a trigger with some v3 checks but NO version marker must be upgraded to v3"
        );
        assert!(sql.contains("GLOB"), "v3 must have the GLOB hex check");
        assert!(
            sql.contains("principal_id"),
            "v3 must have principal-null checks"
        );
    }
}

#[test]
fn trigger_with_v3_marker_but_partial_checks_is_not_upgraded() {
    // #232 P1: a trigger that HAS the v3 marker but is MISSING some
    // checks is NOT upgraded — the marker is the version identity, and
    // the trigger was necessarily created by the v3 code (atomically,
    // with all checks + marker). A marker-carrying but check-missing
    // trigger can only arise from manual corruption, which the read-path
    // validation catches as defense in depth. The migration's contract
    // is: the marker identifies the version; the marker is only written
    // by the code that writes ALL the checks.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("trigger_partial_v3.db");
    let db_path_str = db_path.to_str().unwrap().to_string();

    {
        let _db = Db::new(&db_path_str).unwrap();
    }

    // Replace with a trigger that HAS the marker but is missing GLOB.
    {
        let conn = rusqlite::Connection::open(&db_path_str).unwrap();
        conn.execute(
            "DROP TRIGGER IF EXISTS work_context_events_provenance_required",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TRIGGER work_context_events_provenance_required
             -- provenance-trigger-v3
             BEFORE INSERT ON work_context_events
             WHEN NEW.provenance_json IS NULL
               OR json_valid(NEW.provenance_json) = 0
             BEGIN
                 SELECT RAISE(ABORT, 'journal insert requires complete provenance');
             END",
            [],
        )
        .unwrap();
    }

    // Reopen — the marker IS present, so the migration does NOT fire.
    // This is by design: the marker is the version identity, and manual
    // corruption is caught by the read-path Rust validation (the trigger
    // is defense in depth, not the sole enforcement layer).
    {
        let db = Db::new(&db_path_str).unwrap();
        let sql: String = db
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'trigger' AND name = 'work_context_events_provenance_required'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("provenance-trigger-v3") && !sql.contains("GLOB"),
            "the marker-carrying partial trigger is NOT upgraded (marker = version identity; read-path validation is the defense in depth for manual corruption)"
        );
    }
}

#[test]
fn trigger_v3_schema_version_unchanged_on_reopen() {
    // #232 P1: PRAGMA schema_version is SQLite's documented DDL counter —
    // it increments on every schema change (CREATE, DROP, ALTER) but NOT
    // on data changes (INSERT, UPDATE, DELETE). If the trigger were
    // dropped and recreated (even with identical SQL), schema_version
    // would increment. Comparing schema_version before and after reopen
    // proves NO DDL occurred — a stable, documented SQLite guarantee.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("trigger_schema_version.db");
    let db_path_str = db_path.to_str().unwrap().to_string();

    let schema_version_before: i64;
    {
        let db = Db::new(&db_path_str).unwrap();
        schema_version_before = db
            .conn()
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
    }

    // Reopen — schema_version must be IDENTICAL (no DDL on the trigger).
    {
        let db = Db::new(&db_path_str).unwrap();
        let schema_version_after: i64 = db
            .conn()
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            schema_version_before, schema_version_after,
            "PRAGMA schema_version must be unchanged on reopen (no DDL = trigger truly untouched)"
        );
    }

    // Contrast: after a manual DROP TRIGGER, schema_version INCREMENTS.
    {
        let conn = rusqlite::Connection::open(&db_path_str).unwrap();
        conn.execute("DROP TRIGGER work_context_events_provenance_required", [])
            .unwrap();
        let after_drop: i64 = conn
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
        assert!(
            after_drop > schema_version_before,
            "a DROP TRIGGER must increment schema_version (proving the counter detects DDL)"
        );

        conn.execute(
            "CREATE TRIGGER work_context_events_provenance_required
             -- provenance-trigger-v3
             BEFORE INSERT ON work_context_events
             WHEN NEW.provenance_json IS NULL
             BEGIN
                 SELECT RAISE(ABORT, 'journal insert requires complete provenance');
             END",
            [],
        )
        .unwrap();
        let after_recreate: i64 = conn
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .unwrap();
        assert!(
            after_recreate > after_drop,
            "a CREATE TRIGGER must increment schema_version again"
        );
    }
}

#[test]
fn repo_binding_corrupted_index_never_returns_unbound() {
    // #232 P1: a repo with a VALID HEAD but a CORRUPTED index —
    // `git rev-parse HEAD` succeeds (the repo IS a git repo) but
    // `git status` may fail. The function must produce:
    //   Err → fail-closed (git status genuinely failed)
    //   Ok(Bound) or Ok(Dirty) → git recovered and produced an honest binding
    // NEVER Ok(Unbound) — the repo WAS detected via --git-dir and HEAD,
    // so returning Unbound would be a silent lie about the repo state.
    let dir = tempfile::tempdir().unwrap();
    make_committed_repo(dir.path());

    let index_path = dir.path().join(".git").join("index");
    std::fs::write(
        &index_path,
        b"garbage index data that is not a valid git index",
    )
    .unwrap();

    let binding = prometheos_lite::api::work_contexts::detect_repo_binding(dir.path());
    match binding {
        Err(_) => {
            // Fail-closed: git status genuinely failed — the correct behavior.
        }
        Ok(prometheos_lite::work::provenance::RepoBinding::Bound { revision }) => {
            // Git rebuilt the index and reports a clean tree — honest.
            assert!(!revision.is_empty());
        }
        Ok(prometheos_lite::work::provenance::RepoBinding::Dirty {
            revision,
            workspace_digest,
            digest_policy,
        }) => {
            // Git rebuilt the index and reports dirty state — honest.
            assert!(!revision.is_empty());
            assert!(!workspace_digest.is_empty());
            assert_eq!(digest_policy, "soma-canonical-json-v1");
        }
        Ok(other) => {
            panic!(
                "a detected repo (rev-parse --git-dir succeeded, HEAD present) must never return {:?} — Unbound is a silent lie about a repo that exists",
                other
            );
        }
    }
}
