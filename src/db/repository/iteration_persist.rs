//! Atomic iteration persistence for WorkContext runs (#222).
//!
//! One cancellation-conditional SQLite transaction covers EVERY durable
//! write of a single run-loop iteration: the full-row context update,
//! the iteration's artifact rows, the `artifact_added` /
//! `phase_transition` / `status_changed` events, and the flow
//! performance record. Outcomes:
//!
//! - **Cancel-writer-wins**: the durable cancel commits before (or at)
//!   this transaction — the guarded full-row UPDATE (single source:
//!   [`crate::db::repository::update_work_context_on_conn`], whose
//!   `WHERE status <> Cancelled` predicate is the condition) affects zero
//!   rows, the transaction rolls back having written nothing, and the
//!   caller observes `Ok(false)`. No partial rows can exist because no
//!   other write precedes the guard inside the transaction, and SQLite
//!   serializes writers from the first write statement onward.
//! - **Iteration-writer-wins**: this transaction commits first; a
//!   concurrent cancel then lands on top through its own conditional
//!   transaction (a non-terminal status is cancellable), so the
//!   iteration's rows are either fully present or fully absent — never
//!   partial.
//! - **Any persistence failure**: the error propagates and the
//!   transaction (dropped without commit) rolls back in full — the
//!   previous per-write partial-commit hazard (artifact rows orphaned by
//!   a later failure, or a status flip racing between writes) cannot
//!   occur.
//!
//! Event-stream compatibility: the transaction emits the same events, in
//! the same order, as the previous non-atomic sequence
//! (`artifact_added` per artifact, `phase_transition`,
//! `status_changed` for the intermediate and final statuses).

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::work::artifact::Artifact;
use crate::work::types::FlowPerformanceRecord;
use crate::work::types::{WorkPhase, WorkStatus};

use super::update_work_context_on_conn;

/// The durable write set of ONE run-loop iteration, built in memory by
/// `WorkExecutionService` after the flow completes and the post-flow
/// cancellation checkpoint passes. Contains no DB handles: the service
/// layer builds it, this module commits it atomically.
#[derive(Debug, Clone)]
pub struct IterationDraft {
    /// The iteration's primary artifact (returned to the caller).
    pub primary_artifact: Artifact,
    /// Every artifact produced by this iteration: one `work_artifacts`
    /// row plus one `artifact_added` event each.
    pub new_artifacts: Vec<Artifact>,
    /// `(from, to)` phase transition to persist + `phase_transition`
    /// event, if the iteration advances the phase.
    pub phase_transition: Option<(WorkPhase, WorkPhase)>,
    /// `(from, to)` intermediate status stop (Review autonomy pauses at
    /// AwaitingApproval) persisted as a `status_changed` event before
    /// the final status.
    pub intermediate_status: Option<(WorkStatus, WorkStatus)>,
    /// `(from, to)` final status transition of the iteration; the
    /// committed context row carries `to`. `None` for the standalone
    /// `execute_flow_in_context` entry point, which never applied a
    /// final status.
    pub final_status: Option<(WorkStatus, WorkStatus)>,
}

/// Persist one iteration atomically. Returns `Ok(true)` when committed,
/// `Ok(false)` when cancellation was observed (full rollback, nothing
/// written). Any persistence error propagates as `Err` with the full
/// transaction rolled back.
///
/// `context` is the FINAL in-memory state of the iteration (phase,
/// status, artifacts, execution metadata already applied in memory); the
/// guarded full-row UPDATE writes it once. `performance` is the
/// iteration's flow performance record (the run loop always provides
/// one; the standalone `execute_flow_in_context` entry point preserves
/// its historical no-record behavior with `None`).
pub fn persist_iteration_conn(
    conn: &rusqlite::Connection,
    context: &crate::work::types::WorkContext,
    draft: &IterationDraft,
    performance: Option<&FlowPerformanceRecord>,
) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;

    // Guarded full-row UPDATE is the FIRST statement: it both takes the
    // write lock (SQLite serializes all subsequent writers behind this
    // transaction) and applies the cancellation condition. A deferred
    // BEGIN leaves a gap before this statement — a cancel committing in
    // that gap simply makes this UPDATE affect zero rows below.
    let affected = update_work_context_on_conn(&tx, context)?;
    if affected == 0 {
        // Distinguish refused-by-cancellation from not-found. Either way
        // the transaction has written nothing.
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM work_contexts WHERE id = ?1",
                params![context.id],
                |row| row.get(0),
            )
            .optional()?;
        match status.as_deref() {
            Some(stored) if stored.contains("Cancelled") => {
                tx.rollback()?;
                return Ok(false);
            }
            Some(stored) => {
                // Should be unreachable (the guard refuses only Cancelled),
                // but fail closed rather than guess.
                anyhow::bail!(
                    "iteration persist guard refused a non-cancelled status for {}: {}",
                    context.id,
                    stored
                );
            }
            None => anyhow::bail!("work context not found: {}", context.id),
        }
    }

    // Artifact rows + events. The artifact_count in each event mirrors
    // the historical running count as artifacts were appended.
    let base_count = context
        .artifacts
        .len()
        .saturating_sub(draft.new_artifacts.len());
    for (index, artifact) in draft.new_artifacts.iter().enumerate() {
        let storage_type = match &artifact.storage {
            crate::work::artifact::ArtifactStorage::Inline => "inline".to_string(),
            crate::work::artifact::ArtifactStorage::FilePath(path) => {
                format!("file:{}", path)
            }
        };
        let file_path = match &artifact.storage {
            crate::work::artifact::ArtifactStorage::FilePath(path) => Some(path.as_str()),
            _ => None,
        };
        tx.execute(
            "INSERT INTO work_artifacts (id, work_context_id, kind, name, content, created_by, storage_type, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &artifact.id,
                &artifact.work_context_id,
                serde_json::to_string(&artifact.kind)?,
                &artifact.name,
                serde_json::to_string(&artifact.content)?,
                &artifact.created_by,
                storage_type,
                file_path,
                &artifact.created_at.to_rfc3339(),
            ],
        )
        .context("Failed to insert artifact")?;
        insert_event(
            &tx,
            &context.id,
            "artifact_added",
            serde_json::json!({ "artifact_count": base_count + index + 1 }),
        )?;
    }

    if let Some((from, to)) = draft.phase_transition {
        insert_event(
            &tx,
            &context.id,
            "phase_transition",
            serde_json::json!({ "from": from, "to": to }),
        )?;
    }

    if let Some((from, to)) = draft.intermediate_status {
        insert_event(
            &tx,
            &context.id,
            "status_changed",
            serde_json::json!({ "from": from, "to": to }),
        )?;
    }

    if let Some((from, to)) = draft.final_status {
        insert_event(
            &tx,
            &context.id,
            "status_changed",
            serde_json::json!({ "from": from, "to": to }),
        )?;
    }

    if let Some(record) = performance {
        tx.execute(
            "INSERT INTO flow_performance_records (
                id, flow_id, work_context_id, success_score, duration_ms,
                token_cost, revision_count, executed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.id,
                record.flow_id,
                record.work_context_id,
                record.success_score,
                record.duration_ms as i64,
                record.token_cost,
                record.revision_count as i64,
                record.executed_at.to_rfc3339(),
            ],
        )
        .context("Failed to create flow performance record")?;
    }

    tx.commit()?;
    Ok(true)
}

fn insert_event(
    tx: &rusqlite::Connection,
    context_id: &str,
    event_type: &str,
    data: serde_json::Value,
) -> Result<()> {
    tx.execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            context_id,
            event_type,
            serde_json::to_string(&data)?,
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .with_context(|| format!("failed to record {event_type} event"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::work::artifact::{Artifact, ArtifactKind};
    use crate::work::types::{WorkContext, WorkDomain, WorkPhase, WorkStatus};
    use std::sync::Arc;

    fn artifact_for(context_id: &str) -> Artifact {
        Artifact::new(
            uuid::Uuid::new_v4().to_string(),
            context_id.to_string(),
            ArtifactKind::Plan,
            "plan".to_string(),
            serde_json::json!({"steps": []}),
            "flow".to_string(),
        )
    }

    fn draft(context_id: &str) -> IterationDraft {
        let artifacts = vec![artifact_for(context_id)];
        IterationDraft {
            primary_artifact: artifacts[0].clone(),
            new_artifacts: artifacts,
            phase_transition: Some((WorkPhase::Intake, WorkPhase::Planning)),
            intermediate_status: Some((WorkStatus::InProgress, WorkStatus::AwaitingApproval)),
            final_status: Some((WorkStatus::AwaitingApproval, WorkStatus::InProgress)),
        }
    }

    fn persist(
        db: &Arc<Db>,
        context: &WorkContext,
        draft: &IterationDraft,
        performance: Option<&FlowPerformanceRecord>,
    ) -> Result<bool> {
        persist_iteration_conn(db.conn(), context, draft, performance)
    }

    fn count(db: &Arc<Db>, table: &str) -> i64 {
        db.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn make_context(wcs: &crate::work::service::WorkContextService) -> WorkContext {
        wcs.create_context(
            "user-1".to_string(),
            "T".to_string(),
            WorkDomain::General,
            "goal".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn commit_writes_everything_or_nothing() {
        let db = Arc::new(Db::in_memory().unwrap());
        let wcs = Arc::new(crate::work::service::WorkContextService::new(db.clone()));
        let mut context = make_context(&wcs);
        let draft = draft(&context.id);

        // Apply the iteration's in-memory mutations, as the service does.
        context.current_phase = WorkPhase::Planning;
        context.status = WorkStatus::InProgress;
        for artifact in &draft.new_artifacts {
            context.artifacts.push(artifact.clone());
        }
        let performance = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "planning.flow.yaml".to_string(),
            work_context_id: context.id.clone(),
            success_score: 0.5,
            duration_ms: 10,
            token_cost: 0.0,
            revision_count: 0,
            executed_at: chrono::Utc::now(),
        };

        let committed = persist(&db, &context, &draft, Some(&performance)).unwrap();
        assert!(committed);
        assert_eq!(count(&db, "work_artifacts"), 1);
        assert_eq!(count(&db, "flow_performance_records"), 1);

        let events = crate::db::repository::WorkContextEventOperations::get_events_for_context(
            &*db,
            &context.id,
        )
        .unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"artifact_added"));
        assert!(types.contains(&"phase_transition"));
        assert_eq!(types.iter().filter(|t| **t == "status_changed").count(), 2);
    }

    #[test]
    fn committed_iteration_survives_subsequent_cancel() {
        let db = Arc::new(Db::in_memory().unwrap());
        let wcs = Arc::new(crate::work::service::WorkContextService::new(db.clone()));
        let mut context = make_context(&wcs);
        let draft = draft(&context.id);

        context.current_phase = WorkPhase::Planning;
        context.status = WorkStatus::InProgress;
        for artifact in &draft.new_artifacts {
            context.artifacts.push(artifact.clone());
        }

        // Writer ordering 2: the iteration commits FIRST.
        let committed = persist(&db, &context, &draft, None).unwrap();
        assert!(committed);

        // Then the durable cancel lands on top through its own conditional
        // transaction (InProgress is cancellable).
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "after commit").unwrap();

        // Both writers' effects are fully present: the committed
        // iteration's rows AND the terminal cancel.
        let stored = wcs.get_context(&context.id).unwrap().unwrap();
        assert!(stored.is_cancelled());
        assert_eq!(count(&db, "work_artifacts"), 1);
        let events = crate::db::repository::WorkContextEventOperations::get_events_for_context(
            &*db,
            &context.id,
        )
        .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == "context_cancelled")
                .count(),
            1
        );
        assert!(events.iter().any(|e| e.event_type == "artifact_added"));
    }

    #[test]
    fn cancelled_before_transaction_writes_nothing() {
        let db = Arc::new(Db::in_memory().unwrap());
        let wcs = Arc::new(crate::work::service::WorkContextService::new(db.clone()));
        let context = make_context(&wcs);
        let draft = draft(&context.id);

        // The cancel wins before the transaction.
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "race").unwrap();

        let committed = persist(&db, &context, &draft, None).unwrap();
        assert!(!committed, "must observe cancellation");
        assert_eq!(count(&db, "work_artifacts"), 0);
        assert_eq!(count(&db, "flow_performance_records"), 0);
        let events = crate::db::repository::WorkContextEventOperations::get_events_for_context(
            &*db,
            &context.id,
        )
        .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == "status_changed")
                .count(),
            0
        );
    }

    #[test]
    fn mid_transaction_failure_rolls_back_everything() {
        let db = Arc::new(Db::in_memory().unwrap());
        let wcs = Arc::new(crate::work::service::WorkContextService::new(db.clone()));
        let mut context = make_context(&wcs);
        let draft = draft(&context.id);
        context.current_phase = WorkPhase::Planning;
        context.status = WorkStatus::InProgress;

        // The performance insert (late in the transaction) will fail.
        db.conn()
            .execute("DROP TABLE flow_performance_records", [])
            .unwrap();

        let performance = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "planning.flow.yaml".to_string(),
            work_context_id: context.id.clone(),
            success_score: 0.5,
            duration_ms: 10,
            token_cost: 0.0,
            revision_count: 0,
            executed_at: chrono::Utc::now(),
        };

        let err = persist(&db, &context, &draft, Some(&performance)).unwrap_err();
        assert!(
            err.to_string().contains("flow performance")
                || err.to_string().contains("no such table")
        );

        // Full rollback: the guarded context UPDATE and the artifact rows
        // that preceded the failure are NOT committed.
        assert_eq!(count(&db, "work_artifacts"), 0);
        let stored = wcs.get_context(&context.id).unwrap().unwrap();
        assert_eq!(
            stored.current_phase,
            WorkPhase::Intake,
            "context row unchanged"
        );
    }
}
