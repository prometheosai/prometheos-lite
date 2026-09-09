//! WorkContext event repository operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, types::Type};

use super::AsDb;
use crate::work::event::WorkContextEvent;

/// Parse the `data` JSON column fail-closed: corrupt durable rows are a
/// typed conversion error, never a fabricated `null` event payload.
fn parse_event_data(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<serde_json::Value> {
    let raw: String = row.get(col)?;
    serde_json::from_str(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, Type::Text, Box::new(e)))
}

/// Parse the `created_at` RFC 3339 column fail-closed: malformed durable
/// timestamps are a typed conversion error, never a panic.
fn parse_event_created_at(
    row: &rusqlite::Row<'_>,
    col: usize,
) -> rusqlite::Result<chrono::DateTime<Utc>> {
    let raw: String = row.get(col)?;
    chrono::DateTime::parse_from_rfc3339(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, Type::Text, Box::new(e)))
        .map(|dt| dt.with_timezone(&Utc))
}

/// WorkContext event operations trait
pub trait WorkContextEventOperations {
    fn create_event(&self, event: &WorkContextEvent) -> anyhow::Result<WorkContextEvent>;
    fn get_events_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<WorkContextEvent>>;
    /// Cursorable read: return up to `limit` events for a context whose
    /// durable `seq` is strictly greater than `after_seq`, ordered by
    /// seq ASC.
    ///
    /// Each row is returned together with its `seq` — the `seq` IS the
    /// cursor. `seq` is an `INTEGER PRIMARY KEY AUTOINCREMENT` column:
    /// SQLite assigns it from `sqlite_sequence`, so it is strictly
    /// monotonic in insertion order, never reuses deleted maxima, and is
    /// stable across VACUUM and rebuilds. Iterating with
    /// `after_seq = <last returned seq>` therefore yields every event
    /// exactly once, with no gaps and no duplication, even after
    /// maintenance or deletions elsewhere in the table.
    fn get_events_for_context_after(
        &self,
        work_context_id: &str,
        after_seq: i64,
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
                    data: parse_event_data(row, 3)?,
                    created_at: parse_event_created_at(row, 4)?,
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
        after_seq: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, WorkContextEvent)>> {
        let conn = self.as_db().conn();

        let mut stmt = conn
            .prepare(
                "SELECT seq, id, work_context_id, event_type, data, created_at
             FROM work_context_events
             WHERE work_context_id = ?1 AND seq > ?2
             ORDER BY seq ASC
             LIMIT ?3",
            )
            .context("Failed to prepare cursor events query")?;

        let rows = stmt
            .query_map(params![work_context_id, after_seq, limit as i64], |row| {
                let seq: i64 = row.get(0)?;
                let event = WorkContextEvent {
                    id: row.get(1)?,
                    work_context_id: row.get(2)?,
                    event_type: row.get(3)?,
                    data: parse_event_data(row, 4)?,
                    created_at: parse_event_created_at(row, 5)?,
                };
                Ok((seq, event))
            })
            .context("Failed to query cursor events")?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.context("Failed to parse cursor event")?);
        }

        Ok(result)
    }
}
