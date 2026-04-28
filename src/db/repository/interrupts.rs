//! Interrupt operations for guardrail approval workflow

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;

/// Interrupt entry model
#[derive(Debug, Clone)]
pub struct InterruptEntry {
    pub id: String,
    pub run_id: String,
    pub trace_id: String,
    pub node_id: String,
    pub reason: String,
    pub expected_schema: String,
    pub status: String,
    pub decision: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub work_context_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Interrupt operations trait
pub trait InterruptOperations {
    fn create_interrupt(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        reason: &str,
        expected_schema: &str,
        work_context_id: Option<&str>,
    ) -> anyhow::Result<InterruptEntry>;

    fn get_interrupt(&self, id: &str) -> anyhow::Result<Option<InterruptEntry>>;

    fn list_pending_interrupts(&self, run_id: &str) -> anyhow::Result<Vec<InterruptEntry>>;

    fn approve_interrupt(
        &self,
        id: &str,
        decision: &str,
    ) -> anyhow::Result<()>;

    fn deny_interrupt(&self, id: &str) -> anyhow::Result<()>;
}

impl<T: AsDb> InterruptOperations for T {
    fn create_interrupt(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        reason: &str,
        expected_schema: &str,
        work_context_id: Option<&str>,
    ) -> anyhow::Result<InterruptEntry> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        // Default expiration: 1 hour
        let expires_at = now + chrono::Duration::hours(1);

        conn.execute(
            "INSERT INTO interrupts (id, run_id, trace_id, node_id, reason, expected_schema, status, expires_at, work_context_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![&id, run_id, trace_id, node_id, reason, expected_schema, "pending", &expires_at.to_rfc3339(), work_context_id, &now.to_rfc3339()],
        ).context("Failed to insert interrupt")?;

        Ok(InterruptEntry {
            id,
            run_id: run_id.to_string(),
            trace_id: trace_id.to_string(),
            node_id: node_id.to_string(),
            reason: reason.to_string(),
            expected_schema: expected_schema.to_string(),
            status: "pending".to_string(),
            decision: None,
            expires_at: Some(expires_at),
            work_context_id: work_context_id.map(|s| s.to_string()),
            created_at: now,
        })
    }

    fn get_interrupt(&self, id: &str) -> anyhow::Result<Option<InterruptEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, run_id, trace_id, node_id, reason, expected_schema, status, decision, expires_at, work_context_id, created_at
             FROM interrupts
             WHERE id = ?1"
        ).context("Failed to prepare interrupt query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(InterruptEntry {
                id: row.get(0)?,
                run_id: row.get(1)?,
                trace_id: row.get(2)?,
                node_id: row.get(3)?,
                reason: row.get(4)?,
                expected_schema: row.get(5)?,
                status: row.get(6)?,
                decision: row.get(7)?,
                expires_at: row.get::<_, Option<String>>(8)?.map(|s| chrono::DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&chrono::Utc)),
                work_context_id: row.get(9)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?).unwrap().with_timezone(&chrono::Utc),
            })
        }).context("Failed to query interrupt")?;

        match rows.next() {
            Some(result) => Ok(Some(result.context("Failed to parse interrupt")?)),
            None => Ok(None),
        }
    }

    fn list_pending_interrupts(&self, run_id: &str) -> anyhow::Result<Vec<InterruptEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, run_id, trace_id, node_id, reason, expected_schema, status, decision, expires_at, work_context_id, created_at
             FROM interrupts
             WHERE run_id = ?1 AND status = 'pending'
             ORDER BY created_at"
        ).context("Failed to prepare pending interrupts query")?;

        let entries = stmt.query_map(params![run_id], |row| {
            Ok(InterruptEntry {
                id: row.get(0)?,
                run_id: row.get(1)?,
                trace_id: row.get(2)?,
                node_id: row.get(3)?,
                reason: row.get(4)?,
                expected_schema: row.get(5)?,
                status: row.get(6)?,
                decision: row.get(7)?,
                expires_at: row.get::<_, Option<String>>(8)?.map(|s| chrono::DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&chrono::Utc)),
                work_context_id: row.get(9)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?).unwrap().with_timezone(&chrono::Utc),
            })
        }).context("Failed to query pending interrupts")?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry.context("Failed to parse interrupt entry")?);
        }

        Ok(result)
    }

    fn approve_interrupt(
        &self,
        id: &str,
        decision: &str,
    ) -> anyhow::Result<()> {
        let conn = self.as_db().conn();

        conn.execute(
            "UPDATE interrupts SET status = ?1, decision = ?2 WHERE id = ?3",
            params!["approved", decision, id],
        ).context("Failed to update interrupt status")?;

        Ok(())
    }

    fn deny_interrupt(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.as_db().conn();

        conn.execute(
            "UPDATE interrupts SET status = ?1 WHERE id = ?2",
            params!["denied", id],
        ).context("Failed to update interrupt status")?;

        Ok(())
    }
}
