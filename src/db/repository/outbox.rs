//! Outbox operations for idempotency

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;

/// Outbox entry model
#[derive(Debug, Clone)]
pub struct OutboxEntry {
    pub id: String,
    pub run_id: String,
    pub trace_id: String,
    pub node_id: String,
    pub tool_name: String,
    pub input_hash: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub output: Option<String>,
}

/// Outbox operations trait
pub trait OutboxOperations {
    fn create_outbox_entry(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        input_hash: &str,
    ) -> anyhow::Result<OutboxEntry>;

    fn get_outbox_entry_by_hash(
        &self,
        run_id: &str,
        node_id: &str,
        input_hash: &str,
    ) -> anyhow::Result<Option<OutboxEntry>>;

    fn mark_outbox_completed(&self, id: &str, output: &str) -> anyhow::Result<()>;

    fn list_pending_outbox(&self, run_id: &str) -> anyhow::Result<Vec<OutboxEntry>>;

    /// List all pending outbox entries across all runs
    fn list_all_pending_outbox(&self) -> anyhow::Result<Vec<OutboxEntry>>;
}

impl<T: AsDb> OutboxOperations for T {
    fn create_outbox_entry(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        input_hash: &str,
    ) -> anyhow::Result<OutboxEntry> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        conn.execute(
            "INSERT INTO tool_outbox (id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![&id, run_id, trace_id, node_id, tool_name, input_hash, "pending", &now.to_rfc3339()],
        ).context("Failed to insert outbox entry")?;

        Ok(OutboxEntry {
            id,
            run_id: run_id.to_string(),
            trace_id: trace_id.to_string(),
            node_id: node_id.to_string(),
            tool_name: tool_name.to_string(),
            input_hash: input_hash.to_string(),
            status: "pending".to_string(),
            created_at: now,
            completed_at: None,
            output: None,
        })
    }

    fn get_outbox_entry_by_hash(
        &self,
        run_id: &str,
        node_id: &str,
        input_hash: &str,
    ) -> anyhow::Result<Option<OutboxEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at, completed_at, result_json
             FROM tool_outbox
             WHERE run_id = ?1 AND node_id = ?2 AND input_hash = ?3"
        ).context("Failed to prepare outbox query")?;

        let result = stmt.query_row(params![run_id, node_id, input_hash], |row| {
            Ok(OutboxEntry {
                id: row.get(0)?,
                run_id: row.get(1)?,
                trace_id: row.get(2)?,
                node_id: row.get(3)?,
                tool_name: row.get(4)?,
                input_hash: row.get(5)?,
                status: row.get(6)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                completed_at: row.get::<_, Option<String>>(8)?.map(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .unwrap()
                        .with_timezone(&chrono::Utc)
                }),
                output: row.get::<_, Option<String>>(9)?,
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn mark_outbox_completed(&self, id: &str, output: &str) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        let now = Utc::now();

        conn.execute(
            "UPDATE tool_outbox SET status = ?1, completed_at = ?2, result_json = ?3 WHERE id = ?4",
            params!["completed", &now.to_rfc3339(), output, id],
        )
        .context("Failed to update outbox entry")?;

        Ok(())
    }

    fn list_pending_outbox(&self, run_id: &str) -> anyhow::Result<Vec<OutboxEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at, completed_at, result_json
             FROM tool_outbox
             WHERE run_id = ?1 AND status = 'pending'
             ORDER BY created_at"
        ).context("Failed to prepare pending outbox query")?;

        let entries = stmt
            .query_map(params![run_id], |row| {
                Ok(OutboxEntry {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    trace_id: row.get(2)?,
                    node_id: row.get(3)?,
                    tool_name: row.get(4)?,
                    input_hash: row.get(5)?,
                    status: row.get(6)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    completed_at: row.get::<_, Option<String>>(8)?.map(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .unwrap()
                            .with_timezone(&chrono::Utc)
                    }),
                    output: row.get::<_, Option<String>>(9)?,
                })
            })
            .context("Failed to query pending outbox")?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry.context("Failed to parse outbox entry")?);
        }

        Ok(result)
    }

    fn list_all_pending_outbox(&self) -> anyhow::Result<Vec<OutboxEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at, completed_at, result_json
             FROM tool_outbox
             WHERE status = 'pending'
             ORDER BY created_at"
        ).context("Failed to prepare all pending outbox query")?;

        let entries = stmt
            .query_map([], |row| {
                Ok(OutboxEntry {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    trace_id: row.get(2)?,
                    node_id: row.get(3)?,
                    tool_name: row.get(4)?,
                    input_hash: row.get(5)?,
                    status: row.get(6)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    completed_at: row.get::<_, Option<String>>(8)?.map(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .unwrap()
                            .with_timezone(&chrono::Utc)
                    }),
                    output: row.get::<_, Option<String>>(9)?,
                })
            })
            .context("Failed to query all pending outbox")?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry.context("Failed to parse outbox entry")?);
        }

        Ok(result)
    }
}
