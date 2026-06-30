//! Trace storage in SQLite for persistent observability
//!
//! This module provides SQLite-based storage for execution traces,
//! node runs, tool calls, and LLM metrics.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing;

use crate::flow::tracing::{HierarchicalTrace, LlmCall, NodeRun, ToolCall};

/// Trace storage manager
pub struct TraceStorage {
    conn: Arc<Mutex<Connection>>,
}

impl TraceStorage {
    /// Create or open a trace database at the given path
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open trace database at: {}", db_path.display()))?;

        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Create an in-memory trace database for testing
    pub fn in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("Failed to create in-memory trace database")?;

        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        // Schema version table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            )",
            [],
        )
        .context("Failed to create schema_version table")?;

        // Get current schema version
        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Apply migrations if needed
        // V1.5.2: Enhanced migration framework for trace storage versioning
        let target_version = 2; // Current schema version

        if current_version < 1 {
            tracing::info!("Migrating trace storage to v1...");
            self.migrate_to_v1(&conn)?;
        }

        if current_version < 2 {
            tracing::info!("Migrating trace storage to v2...");
            self.migrate_to_v2(&conn)?;
        }

        // Record final schema version
        if current_version < target_version {
            tracing::info!(
                "Trace storage schema migrated from v{} to v{}",
                current_version,
                target_version
            );
        }

        Ok(())
    }

    /// Migrate to schema version 1 - Initial trace storage schema
    fn migrate_to_v1(&self, conn: &Connection) -> Result<()> {
        // Execution traces table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS execution_traces (
                trace_id TEXT PRIMARY KEY,
                work_context_id TEXT,
                flow_run_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                node_runs_count INTEGER DEFAULT 0,
                tool_calls_count INTEGER DEFAULT 0,
                llm_calls_count INTEGER DEFAULT 0
            )",
            [],
        )
        .context("Failed to create execution_traces table")?;

        // Node runs table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS node_runs (
                id TEXT PRIMARY KEY,
                trace_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                input_summary TEXT,
                output_summary TEXT,
                status TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                error TEXT,
                started_at TEXT NOT NULL,
                completed_at TEXT NOT NULL,
                FOREIGN KEY (trace_id) REFERENCES execution_traces(trace_id) ON DELETE CASCADE
            )",
            [],
        )
        .context("Failed to create node_runs table")?;

        // Tool calls table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_calls (
                id TEXT PRIMARY KEY,
                trace_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_hash TEXT NOT NULL,
                result_hash TEXT NOT NULL,
                success BOOLEAN NOT NULL,
                duration_ms INTEGER NOT NULL,
                called_at TEXT NOT NULL,
                FOREIGN KEY (trace_id) REFERENCES execution_traces(trace_id) ON DELETE CASCADE
            )",
            [],
        )
        .context("Failed to create tool_calls table")?;

        // LLM calls table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS llm_calls (
                id TEXT PRIMARY KEY,
                trace_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL,
                completion_tokens INTEGER NOT NULL,
                latency_ms INTEGER NOT NULL,
                error TEXT,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                FOREIGN KEY (trace_id) REFERENCES execution_traces(trace_id) ON DELETE CASCADE
            )",
            [],
        )
        .context("Failed to create llm_calls table")?;

        // Create indexes for performance
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_execution_traces_work_context ON execution_traces(work_context_id)",
            [],
        )
        .context("Failed to create work_context index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_node_runs_trace ON node_runs(trace_id)",
            [],
        )
        .context("Failed to create node_runs trace index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_calls_trace ON tool_calls(trace_id)",
            [],
        )
        .context("Failed to create tool_calls trace index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_llm_calls_trace ON llm_calls(trace_id)",
            [],
        )
        .context("Failed to create llm_calls trace index")?;

        // Record schema version
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (1, ?)",
            [Utc::now().to_rfc3339()],
        )
        .context("Failed to record schema version")?;

        Ok(())
    }

    /// Migrate to schema version 2 - Enhanced trace storage with metadata
    /// V1.5.2: Adds metadata column and performance indices
    fn migrate_to_v2(&self, conn: &Connection) -> Result<()> {
        // Add metadata column to execution_traces for extensibility
        conn.execute(
            "ALTER TABLE execution_traces ADD COLUMN metadata TEXT DEFAULT '{}'",
            [],
        )
        .ok(); // Ignore error if column already exists

        // Add performance-related indices
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_execution_traces_completed 
             ON execution_traces(completed_at) WHERE completed_at IS NOT NULL",
            [],
        )
        .ok();

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_llm_calls_duration 
             ON llm_calls(latency_ms) WHERE latency_ms > 1000",
            [],
        )
        .ok();

        // Record schema version
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (2, ?)",
            [Utc::now().to_rfc3339()],
        )
        .context("Failed to record schema version v2")?;

        Ok(())
    }

    /// Save a hierarchical trace to storage
    pub fn save_trace(&self, trace: &HierarchicalTrace) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        // Save execution trace
        conn.execute(
            "INSERT OR REPLACE INTO execution_traces 
             (trace_id, work_context_id, flow_run_id, started_at, completed_at, node_runs_count, tool_calls_count, llm_calls_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                trace.trace_id.to_string(),
                trace.work_context_id,
                trace.flow_run_id.to_string(),
                trace.started_at.to_rfc3339(),
                trace.completed_at.map(|t| t.to_rfc3339()),
                trace.node_runs.len() as i64,
                trace.tool_calls.len() as i64,
                trace.llm_calls.len() as i64,
            ],
        )?;

        // Save node runs
        for node_run in &trace.node_runs {
            self.save_node_run(&conn, node_run)?;
        }

        // Save tool calls
        for tool_call in &trace.tool_calls {
            self.save_tool_call(&conn, tool_call)?;
        }

        // Save LLM calls
        for llm_call in &trace.llm_calls {
            self.save_llm_call(&conn, llm_call)?;
        }

        Ok(())
    }

    /// Save a node run
    fn save_node_run(&self, conn: &Connection, node_run: &NodeRun) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO node_runs 
             (id, trace_id, node_id, input_summary, output_summary, status, duration_ms, error, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                uuid::Uuid::new_v4().to_string(),
                node_run.trace_id.to_string(),
                node_run.node_id.to_string(),
                node_run.input_summary,
                node_run.output_summary,
                &node_run.status,
                node_run.duration_ms as i64,
                node_run.error,
                node_run.started_at.to_rfc3339(),
                node_run.completed_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Save a tool call
    fn save_tool_call(&self, conn: &Connection, tool_call: &ToolCall) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO tool_calls 
             (id, trace_id, tool_name, args_hash, result_hash, success, duration_ms, called_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                tool_call.trace_id.to_string(),
                &tool_call.tool_name,
                &tool_call.args_hash,
                &tool_call.result_hash,
                tool_call.success,
                tool_call.duration_ms as i64,
                tool_call.called_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Save an LLM call
    fn save_llm_call(&self, conn: &Connection, llm_call: &LlmCall) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO llm_calls 
             (id, trace_id, node_id, provider, model, prompt_tokens, completion_tokens, latency_ms, error, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                uuid::Uuid::new_v4().to_string(),
                llm_call.trace_id.to_string(),
                llm_call.node_id.to_string(),
                &llm_call.provider,
                &llm_call.model,
                llm_call.prompt_tokens as i64,
                llm_call.completion_tokens as i64,
                llm_call.latency_ms as i64,
                llm_call.error,
                llm_call.started_at.to_rfc3339(),
                llm_call.completed_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    /// Get a trace by ID
    pub fn get_trace(&self, trace_id: &str) -> Result<Option<HierarchicalTrace>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT trace_id, work_context_id, flow_run_id, started_at, completed_at 
             FROM execution_traces WHERE trace_id = ?1",
        )?;

        let mut rows = stmt.query(params![trace_id])?;

        if let Some(row) = rows.next()? {
            let trace_id_str: String = row.get(0)?;
            let work_context_id: Option<String> = row.get(1)?;
            let flow_run_id_str: String = row.get(2)?;
            let started_at_str: String = row.get(3)?;
            let completed_at_str: Option<String> = row.get(4)?;

            let trace_id = trace_id_str.clone();
            let flow_run_id = flow_run_id_str;
            let started_at = DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&Utc);
            let completed_at = completed_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            // Load node runs
            let node_runs = self.get_node_runs(&conn, &trace_id_str)?;

            // Load tool calls
            let tool_calls = self.get_tool_calls(&conn, &trace_id_str)?;

            // Load LLM calls
            let llm_calls = self.get_llm_calls(&conn, &trace_id_str)?;

            Ok(Some(HierarchicalTrace {
                trace_id,
                work_context_id,
                flow_run_id,
                node_runs,
                tool_calls,
                llm_calls,
                started_at,
                completed_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get node runs for a trace
    fn get_node_runs(&self, conn: &Connection, trace_id: &str) -> Result<Vec<NodeRun>> {
        let mut stmt = conn.prepare(
            "SELECT node_id, input_summary, output_summary, status, duration_ms, error, started_at, completed_at
             FROM node_runs WHERE trace_id = ?1",
        )?;

        let mut node_runs = Vec::new();
        let mut rows = stmt.query(params![trace_id])?;

        while let Some(row) = rows.next()? {
            let node_id_str: String = row.get(0)?;
            let started_at_str: String = row.get(6)?;
            let completed_at_str: String = row.get(7)?;

            node_runs.push(NodeRun {
                node_id: node_id_str.parse().unwrap_or_default(),
                trace_id: trace_id.parse().unwrap_or_default(),
                input_summary: row.get(1)?,
                output_summary: row.get(2)?,
                status: row.get(3)?,
                duration_ms: row.get(4)?,
                error: row.get(5)?,
                started_at: DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&Utc),
                completed_at: DateTime::parse_from_rfc3339(&completed_at_str)?.with_timezone(&Utc),
            });
        }

        Ok(node_runs)
    }

    /// Get tool calls for a trace
    fn get_tool_calls(&self, conn: &Connection, trace_id: &str) -> Result<Vec<ToolCall>> {
        let mut stmt = conn.prepare(
            "SELECT tool_name, args_hash, result_hash, success, duration_ms, called_at
             FROM tool_calls WHERE trace_id = ?1",
        )?;

        let mut tool_calls = Vec::new();
        let mut rows = stmt.query(params![trace_id])?;

        while let Some(row) = rows.next()? {
            let called_at_str: String = row.get(5)?;

            tool_calls.push(ToolCall {
                tool_name: row.get(0)?,
                trace_id: trace_id.to_string(),
                args_hash: row.get(1)?,
                result_hash: row.get(2)?,
                success: row.get(3)?,
                duration_ms: row.get(4)?,
                called_at: DateTime::parse_from_rfc3339(&called_at_str)?.with_timezone(&Utc),
            });
        }

        Ok(tool_calls)
    }

    /// Get LLM calls for a trace
    fn get_llm_calls(&self, conn: &Connection, trace_id: &str) -> Result<Vec<LlmCall>> {
        let mut stmt = conn.prepare(
            "SELECT node_id, provider, model, prompt_tokens, completion_tokens, latency_ms, error, started_at, completed_at
             FROM llm_calls WHERE trace_id = ?1",
        )?;

        let mut llm_calls = Vec::new();
        let mut rows = stmt.query(params![trace_id])?;

        while let Some(row) = rows.next()? {
            let node_id_str: String = row.get(0)?;
            let started_at_str: String = row.get(7)?;
            let completed_at_str: Option<String> = row.get(8)?;

            llm_calls.push(LlmCall {
                node_id: node_id_str,
                trace_id: trace_id.to_string(),
                provider: row.get(1)?,
                model: row.get(2)?,
                prompt_tokens: row.get(3)?,
                completion_tokens: row.get(4)?,
                latency_ms: row.get(5)?,
                error: row.get(6)?,
                started_at: DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&Utc),
                completed_at: completed_at_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
            });
        }

        Ok(llm_calls)
    }

    /// Get traces by flow run ID
    pub fn get_traces_by_flow_run(&self, flow_run_id: &str) -> Result<Vec<HierarchicalTrace>> {
        // Collect trace IDs while holding the DB lock, then release the lock before
        // calling `get_trace` (which acquires the same mutex) to avoid self-deadlock.
        let trace_ids: Vec<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

            let mut stmt =
                conn.prepare("SELECT trace_id FROM execution_traces WHERE flow_run_id = ?1")?;
            let mut rows = stmt.query(params![flow_run_id])?;
            let mut ids = Vec::new();

            while let Some(row) = rows.next()? {
                let trace_id: String = row.get(0)?;
                ids.push(trace_id);
            }

            ids
        };

        let mut traces = Vec::new();
        for trace_id in trace_ids {
            if let Some(trace) = self.get_trace(&trace_id)? {
                traces.push(trace);
            }
        }

        Ok(traces)
    }

    /// Delete a trace
    pub fn delete_trace(&self, trace_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let rows_affected = conn.execute(
            "DELETE FROM execution_traces WHERE trace_id = ?1",
            params![trace_id],
        )?;
        Ok(rows_affected > 0)
    }

    /// Delete traces older than a given date
    pub fn delete_traces_older_than(&self, date: DateTime<Utc>) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let date_str = date.to_rfc3339();
        let rows = conn.execute(
            "DELETE FROM execution_traces WHERE started_at < ?1",
            params![date_str],
        )?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_storage_init() {
        let _storage = TraceStorage::in_memory().unwrap();
        // Schema should be created without errors
    }

    #[test]
    fn test_save_and_retrieve_trace() {
        let storage = TraceStorage::in_memory().unwrap();

        let trace_id = uuid::Uuid::new_v4().to_string();
        let flow_run_id = uuid::Uuid::new_v4().to_string();

        let trace = HierarchicalTrace {
            trace_id: trace_id.clone(),
            work_context_id: Some("test-context".to_string()),
            flow_run_id,
            node_runs: Vec::new(),
            tool_calls: Vec::new(),
            llm_calls: Vec::new(),
            started_at: Utc::now(),
            completed_at: None,
        };

        storage.save_trace(&trace).unwrap();

        let retrieved = storage.get_trace(&trace_id).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().trace_id, trace.trace_id);
    }

    #[test]
    fn test_delete_trace() {
        let storage = TraceStorage::in_memory().unwrap();

        let trace_id = uuid::Uuid::new_v4().to_string();
        let flow_run_id = uuid::Uuid::new_v4().to_string();

        let trace = HierarchicalTrace {
            trace_id: trace_id.clone(),
            work_context_id: Some("test-context".to_string()),
            flow_run_id,
            node_runs: Vec::new(),
            tool_calls: Vec::new(),
            llm_calls: Vec::new(),
            started_at: Utc::now(),
            completed_at: None,
        };

        storage.save_trace(&trace).unwrap();

        let deleted = storage.delete_trace(&trace_id).unwrap();
        assert!(deleted);

        let retrieved = storage.get_trace(&trace_id).unwrap();
        assert!(retrieved.is_none());
    }
}
