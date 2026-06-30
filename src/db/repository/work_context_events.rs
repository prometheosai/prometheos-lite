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
}
