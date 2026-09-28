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

    // Autonomy widening is refused.
    let mut widened_autonomy = journal.event_envelope(None);
    widened_autonomy.authority.declared.autonomy = AutonomyLevel::Chat;
    widened_autonomy.authority.effective.autonomy = AutonomyLevel::Autonomous;
    assert!(
        widened_autonomy.validate_write_invariants().is_err(),
        "autonomy widening (Chat â†’ Autonomous) must fail"
    );

    // Approval-policy widening is refused.
    let mut widened_approval = journal.event_envelope(None);
    widened_approval.authority.declared.approval_policy = ApprovalPolicy::ManualAll;
    widened_approval.authority.effective.approval_policy = ApprovalPolicy::Auto;
    assert!(
        widened_approval.validate_write_invariants().is_err(),
        "approval-policy widening (ManualAll â†’ Auto) must fail"
    );

    // Same-level narrowing is allowed (effective == declared).
    let narrowed = journal.event_envelope(None);
    assert!(narrowed.validate_write_invariants().is_ok());
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
