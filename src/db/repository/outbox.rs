//! Outbox operations for idempotency

use anyhow::Context;
use chrono::Utc;
use rusqlite::{Connection, Row, params};

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

fn parse_timestamp(
    value: String,
    column: usize,
) -> rusqlite::Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(&value)
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<OutboxEntry> {
    Ok(OutboxEntry {
        id: row.get(0)?,
        run_id: row.get(1)?,
        trace_id: row.get(2)?,
        node_id: row.get(3)?,
        tool_name: row.get(4)?,
        input_hash: row.get(5)?,
        status: row.get(6)?,
        created_at: parse_timestamp(row.get::<_, String>(7)?, 7)?,
        completed_at: row
            .get::<_, Option<String>>(8)?
            .map(|value| parse_timestamp(value, 8))
            .transpose()?,
        output: row.get::<_, Option<String>>(9)?,
    })
}

/// All claims for one operation, using fail-closed timestamp parsing.
fn entries_for(
    conn: &Connection,
    run_id: &str,
    node_id: &str,
    input_hash: &str,
) -> anyhow::Result<Vec<OutboxEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at, completed_at, result_json
         FROM tool_outbox
         WHERE run_id = ?1 AND node_id = ?2 AND input_hash = ?3"
    ).context("Failed to prepare outbox query")?;
    let rows = stmt
        .query_map(params![run_id, node_id, input_hash], entry_from_row)
        .context("Failed to query outbox claims")?;
    let mut entries = Vec::new();
    for entry in rows {
        entries.push(entry.context("Failed to decode outbox claim")?);
    }
    Ok(entries)
}

/// Result of one atomic admission decision.
#[derive(Debug, Clone)]
pub enum OutboxAdmission {
    /// An already completed receipt was found; no new effect is admitted.
    Completed(OutboxEntry),
    /// A single pending canonical claim was created under this transaction.
    Admitted(OutboxEntry),
}

/// Outbox operations trait
pub trait OutboxOperations {
    /// Legacy append-only helper retained for repository tests and explicitly
    /// non-governed tooling. Governed effect paths must use
    /// [`OutboxOperations::admit_outbox_operation`]; this method deliberately
    /// does not check for competing or duplicate claims.
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

    /// Every stored claim for one operation, not just the first matching row.
    fn list_outbox_entries_by_hash(
        &self,
        run_id: &str,
        node_id: &str,
        input_hash: &str,
    ) -> anyhow::Result<Vec<OutboxEntry>>;

    /// Inspect both key versions and create the canonical pending claim in one
    /// SQLite IMMEDIATE transaction. Completed receipts are reused, uncertain
    /// pending claims refuse admission, and competing writers cannot duplicate.
    fn admit_outbox_operation(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        canonical_hash: &str,
        legacy_hash: &str,
    ) -> anyhow::Result<OutboxAdmission>;

    /// Atomically create the single pending claim for an operation. Refuses if
    /// any row already exists, so competing writers cannot duplicate admission.
    fn claim_outbox_entry(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        input_hash: &str,
    ) -> anyhow::Result<OutboxEntry>;

    fn mark_outbox_completed(&self, id: &str, output: &str) -> anyhow::Result<()>;

    /// Mark completion and verify that exactly one row was updated.
    fn mark_outbox_completed_checked(&self, id: &str, output: &str) -> anyhow::Result<()>;

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
        Ok(entries_for(conn, run_id, node_id, input_hash)?
            .into_iter()
            .next())
    }

    fn list_outbox_entries_by_hash(
        &self,
        run_id: &str,
        node_id: &str,
        input_hash: &str,
    ) -> anyhow::Result<Vec<OutboxEntry>> {
        let conn = self.as_db().conn();
        entries_for(conn, run_id, node_id, input_hash)
    }

    fn admit_outbox_operation(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        canonical_hash: &str,
        legacy_hash: &str,
    ) -> anyhow::Result<OutboxAdmission> {
        let conn = self.as_db().conn();
        // One IMMEDIATE transaction covers inspection of both key versions and
        // the canonical admission insert. Competing writers serialize here;
        // no nested transaction is started by entries_for.
        conn.execute_batch("BEGIN IMMEDIATE")
            .context("Failed to begin atomic outbox admission")?;
        let decision = (|| -> anyhow::Result<OutboxAdmission> {
            let mut completed = Vec::new();
            let mut pending = 0usize;
            for hash in [canonical_hash, legacy_hash] {
                for entry in entries_for(conn, run_id, node_id, hash)? {
                    match entry.status.as_str() {
                        "completed" => completed.push(entry),
                        "pending" => pending += 1,
                        other => anyhow::bail!(
                            "unknown outbox status {other} for run {run_id} node {node_id}"
                        ),
                    }
                }
            }
            if completed.len() > 1 {
                let first = completed[0].output.as_deref();
                if completed
                    .iter()
                    .skip(1)
                    .any(|e| e.output.as_deref() != first)
                {
                    anyhow::bail!(
                        "conflicting completed outbox receipts for run {run_id} node {node_id}"
                    );
                }
            }
            if let Some(entry) = completed.into_iter().next() {
                return Ok(OutboxAdmission::Completed(entry));
            }
            if pending > 0 {
                anyhow::bail!(
                    "unresolved prior outbox claim ({pending}) for run {run_id} node {node_id}"
                );
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = Utc::now();
            conn.execute(
                "INSERT INTO tool_outbox (id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![&id, run_id, trace_id, node_id, tool_name, canonical_hash, "pending", &now.to_rfc3339()],
            ).context("Failed to insert atomic outbox admission")?;
            Ok(OutboxAdmission::Admitted(OutboxEntry {
                id,
                run_id: run_id.to_string(),
                trace_id: trace_id.to_string(),
                node_id: node_id.to_string(),
                tool_name: tool_name.to_string(),
                input_hash: canonical_hash.to_string(),
                status: "pending".to_string(),
                created_at: now,
                completed_at: None,
                output: None,
            }))
        })();
        match decision {
            Ok(value) => {
                conn.execute_batch("COMMIT")
                    .context("Failed to commit atomic outbox admission")?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn claim_outbox_entry(
        &self,
        run_id: &str,
        trace_id: &str,
        node_id: &str,
        tool_name: &str,
        input_hash: &str,
    ) -> anyhow::Result<OutboxEntry> {
        let conn = self.as_db().conn();
        conn.execute_batch("BEGIN IMMEDIATE")
            .context("Failed to begin exclusive outbox claim")?;
        let result = (|| -> anyhow::Result<OutboxEntry> {
            let existing: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tool_outbox WHERE run_id = ?1 AND node_id = ?2 AND input_hash = ?3",
                    params![run_id, node_id, input_hash],
                    |row| row.get(0),
                )
                .context("Failed to inspect existing outbox claims")?;
            if existing != 0 {
                anyhow::bail!(
                    "competing or duplicate outbox claim for run {run_id} node {node_id}"
                );
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = Utc::now();
            conn.execute(
                "INSERT INTO tool_outbox (id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![&id, run_id, trace_id, node_id, tool_name, input_hash, "pending", &now.to_rfc3339()],
            ).context("Failed to insert atomic outbox claim")?;
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
        })();
        match result {
            Ok(entry) => {
                conn.execute_batch("COMMIT")
                    .context("Failed to commit outbox claim")?;
                Ok(entry)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
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

    fn mark_outbox_completed_checked(&self, id: &str, output: &str) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        let now = Utc::now();
        // Compare-and-swap: only the exact pending claim we admitted may
        // transition to completed. An already completed receipt can never be
        // overwritten by a later caller.
        let changed = conn
            .execute(
                "UPDATE tool_outbox SET status = ?1, completed_at = ?2, result_json = ?3 WHERE id = ?4 AND status = 'pending'",
                params!["completed", &now.to_rfc3339(), output, id],
            )
            .context("Failed to update outbox entry")?;
        if changed != 1 {
            anyhow::bail!(
                "completion CAS affected {changed} rows for outbox entry {id}; expected one pending claim"
            );
        }
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
