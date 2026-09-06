//! WorkContext event repository operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;
use crate::work::event::WorkContextEvent;

/// WorkContext event operations trait
pub trait WorkContextEventOperations {
    fn create_event(&self, event: &WorkContextEvent) -> anyhow::Result<WorkContextEvent>;
    fn get_events_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<WorkContextEvent>>;
    /// Cursorable read: return up to `limit` events for a context whose
    /// rowid is strictly greater than `after_rowid`, ordered by rowid ASC.
    ///
    /// Each row is returned together with its rowid — the rowid IS the
    /// cursor. rowid is strictly monotonically increasing in insertion
    /// order for this table (a regular rowid table, not WITHOUT ROWID),
    /// so iterating with `after_rowid = <last returned rowid>` yields
    /// every event exactly once, with no gaps and no duplication.
    ///
    /// Invariant note: this cursor contract relies on the table never
    /// being VACUUMed or rebuilt (VACUUM may reassign rowids of tables
    /// without an INTEGER PRIMARY KEY). There is no VACUUM call anywhere
    /// in the codebase; do not add one without replacing this cursor
    /// scheme with a durable sequence column.
    fn get_events_for_context_after(
        &self,
        work_context_id: &str,
        after_rowid: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, WorkContextEvent)>>;
}

impl<T: AsDb> WorkContextEventOperations for T {
    fn create_event(&self, event: &WorkContextEvent) -> anyhow::Result<WorkContextEvent> {
        let conn = self.as_db().conn();

        conn.execute(
            "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &event.id,
                &event.work_context_id,
                &event.event_type,
                &serde_json::to_string(&event.data)?,
                &event.created_at.to_rfc3339(),
            ],
        )
        .context("Failed to insert work context event")?;

        Ok(event.clone())
    }

    fn get_events_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<WorkContextEvent>> {
        let conn = self.as_db().conn();

        let mut stmt = conn
            .prepare(
                "SELECT id, work_context_id, event_type, data, created_at
             FROM work_context_events
             WHERE work_context_id = ?1
             ORDER BY created_at ASC",
            )
            .context("Failed to prepare events query")?;

        let events = stmt
            .query_map(params![work_context_id], |row| {
                Ok(WorkContextEvent {
                    id: row.get(0)?,
                    work_context_id: row.get(1)?,
                    event_type: row.get(2)?,
                    data: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })
            .context("Failed to query events")?;

        let mut result = Vec::new();
        for event in events {
            result.push(event.context("Failed to parse event")?);
        }

        Ok(result)
    }

    fn get_events_for_context_after(
        &self,
        work_context_id: &str,
        after_rowid: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, WorkContextEvent)>> {
        let conn = self.as_db().conn();

        let mut stmt = conn
            .prepare(
                "SELECT rowid, id, work_context_id, event_type, data, created_at
             FROM work_context_events
             WHERE work_context_id = ?1 AND rowid > ?2
             ORDER BY rowid ASC
             LIMIT ?3",
            )
            .context("Failed to prepare cursor events query")?;

        let rows = stmt
            .query_map(params![work_context_id, after_rowid, limit as i64], |row| {
                let rowid: i64 = row.get(0)?;
                let event = WorkContextEvent {
                    id: row.get(1)?,
                    work_context_id: row.get(2)?,
                    event_type: row.get(3)?,
                    data: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&Utc),
                };
                Ok((rowid, event))
            })
            .context("Failed to query cursor events")?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.context("Failed to parse cursor event")?);
        }

        Ok(result)
    }
}
