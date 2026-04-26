//! Orchestration layer - Maestro and continuation engine

use crate::flow::{Flow, SharedState};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Run status for tracking flow execution lifecycle
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Paused,
}

/// Flow run metadata for lifecycle tracking
#[derive(Debug, Clone)]
pub struct FlowRun {
    pub id: String,
    pub flow_id: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub state_snapshot: Option<SharedState>,
}

impl FlowRun {
    pub fn new(flow_id: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            flow_id,
            status: RunStatus::Pending,
            started_at: Utc::now(),
            completed_at: None,
            state_snapshot: None,
        }
    }

    pub fn mark_running(&mut self) {
        self.status = RunStatus::Running;
    }

    pub fn mark_completed(&mut self, state: SharedState) {
        self.status = RunStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.state_snapshot = Some(state);
    }

    pub fn mark_failed(&mut self, error: String) {
        self.status = RunStatus::Failed(error);
        self.completed_at = Some(Utc::now());
    }

    pub fn mark_paused(&mut self, state: SharedState) {
        self.status = RunStatus::Paused;
        self.state_snapshot = Some(state);
    }
}

/// Run registry for tracking flow execution history
pub struct RunRegistry {
    runs: HashMap<String, FlowRun>,
    db: Option<RunDb>,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
            db: None,
        }
    }

    /// Create a RunRegistry with SQLite persistence
    pub fn with_persistence(db_path: PathBuf) -> Result<Self> {
        let db = RunDb::new(db_path)?;
        Ok(Self {
            runs: HashMap::new(),
            db: Some(db),
        })
    }

    pub fn register(&mut self, run: FlowRun) {
        let run_id = run.id.clone();
        self.runs.insert(run_id.clone(), run.clone());

        // Persist to database if available
        if let Some(db) = &mut self.db {
            let _ = db.save_run(&run);
        }
    }

    pub fn get(&self, run_id: &str) -> Option<&FlowRun> {
        self.runs.get(run_id)
    }

    pub fn get_mut(&mut self, run_id: &str) -> Option<&mut FlowRun> {
        self.runs.get_mut(run_id)
    }

    pub fn list_by_flow(&self, flow_id: &str) -> Vec<&FlowRun> {
        self.runs
            .values()
            .filter(|r| r.flow_id == flow_id)
            .collect()
    }

    pub fn list_active(&self) -> Vec<&FlowRun> {
        self.runs
            .values()
            .filter(|r| matches!(r.status, RunStatus::Running | RunStatus::Paused))
            .collect()
    }

    /// Load runs from database on startup
    pub fn load_from_db(&mut self) -> Result<()> {
        if let Some(db) = &self.db {
            let runs = db.load_all_runs()?;
            for run in runs {
                self.runs.insert(run.id.clone(), run);
            }
        }
        Ok(())
    }

    /// Update a run in the database
    pub fn update_run(&mut self, run: &FlowRun) -> Result<()> {
        self.runs.insert(run.id.clone(), run.clone());
        if let Some(db) = &mut self.db {
            db.save_run(run)?;
        }
        Ok(())
    }
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// SQLite database for persisting run registry and flow events
pub struct RunDb {
    conn: Connection,
}

impl RunDb {
    /// Create or open a run database at the given path
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open run database at: {}", db_path.display()))?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS flow_runs (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                state_snapshot TEXT
            )",
            [],
        )
        .context("Failed to create flow_runs table")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS flow_events (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                node_id TEXT,
                timestamp TEXT NOT NULL,
                data TEXT,
                FOREIGN KEY (run_id) REFERENCES flow_runs(id)
            )",
            [],
        )
        .context("Failed to create flow_events table")?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_flow_runs_flow_id ON flow_runs(flow_id)",
            [],
        )
        .context("Failed to create flow_runs index")?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_flow_events_run_id ON flow_events(run_id)",
            [],
        )
        .context("Failed to create flow_events index")?;

        Ok(())
    }

    /// Save a flow run to the database
    pub fn save_run(&self, run: &FlowRun) -> Result<()> {
        let status_str = match run.status {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed(_) => "failed",
            RunStatus::Paused => "paused",
        };

        let state_json = run.state_snapshot.as_ref()
            .and_then(|s| serde_json::to_string(s).ok());

        let completed_at_str = run.completed_at
            .map(|t| t.to_rfc3339());

        self.conn.execute(
            "INSERT OR REPLACE INTO flow_runs (id, flow_id, status, started_at, completed_at, state_snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run.id,
                run.flow_id,
                status_str,
                run.started_at.to_rfc3339(),
                completed_at_str,
                state_json,
            ],
        )
        .context("Failed to save flow run")?;

        Ok(())
    }

    /// Load all flow runs from the database
    pub fn load_all_runs(&self) -> Result<Vec<FlowRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, flow_id, status, started_at, completed_at, state_snapshot
             FROM flow_runs"
        )
        .context("Failed to prepare load_all_runs query")?;

        let rows = stmt.query_map([], |row| {
            let status_str: String = row.get(2)?;
            let status = match status_str.as_str() {
                "pending" => RunStatus::Pending,
                "running" => RunStatus::Running,
                "completed" => RunStatus::Completed,
                "paused" => RunStatus::Paused,
                "failed" => RunStatus::Failed("Unknown error".to_string()),
                _ => RunStatus::Pending,
            };

            let started_at = DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                .unwrap()
                .with_timezone(&Utc);

            let completed_at = row.get::<_, Option<String>>(4)?
                .map(|s| DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&Utc));

            let state_snapshot = row.get::<_, Option<String>>(5)?
                .and_then(|s| serde_json::from_str(&s).ok());

            Ok(FlowRun {
                id: row.get(0)?,
                flow_id: row.get(1)?,
                status,
                started_at,
                completed_at,
                state_snapshot,
            })
        })
        .context("Failed to query flow runs")?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row.map_err(|e| anyhow::anyhow!(e))?);
        }
        Ok(runs)
    }

    /// Save a flow event to the database
    pub fn save_event(&self, event: &FlowEvent) -> Result<()> {
        let data_json = serde_json::to_string(&event.data)
            .context("Failed to serialize event data")?;

        self.conn.execute(
            "INSERT INTO flow_events (id, run_id, event_type, node_id, timestamp, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                event.run_id,
                event.event_type,
                event.node_id,
                event.timestamp.to_rfc3339(),
                data_json,
            ],
        )
        .context("Failed to save flow event")?;

        Ok(())
    }

    /// Load events for a specific run
    pub fn load_events_for_run(&self, run_id: &str) -> Result<Vec<FlowEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, event_type, node_id, timestamp, data
             FROM flow_events WHERE run_id = ?1 ORDER BY timestamp"
        )
        .context("Failed to prepare load_events_for_run query")?;

        let rows = stmt.query_map(params![run_id], |row| {
            let timestamp = DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                .unwrap()
                .with_timezone(&Utc);

            let data_json: String = row.get(5)?;
            let data = serde_json::from_str(&data_json)
                .unwrap_or(serde_json::Value::Null);

            Ok(FlowEvent {
                id: row.get(0)?,
                run_id: row.get(1)?,
                event_type: row.get(2)?,
                node_id: row.get(3)?,
                timestamp,
                data,
            })
        })
        .context("Failed to query flow events")?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| anyhow::anyhow!(e))?);
        }
        Ok(events)
    }
}

/// Flow event for tracking execution timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEvent {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub node_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl FlowEvent {
    pub fn new(run_id: String, event_type: String, node_id: Option<String>, data: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            run_id,
            event_type,
            node_id,
            timestamp: Utc::now(),
            data,
        }
    }
}

/// Maestro - flow scheduling and orchestration
pub struct Maestro {
    registry: RunRegistry,
}

impl Maestro {
    pub fn new() -> Self {
        Self {
            registry: RunRegistry::new(),
        }
    }

    /// Create a Maestro with SQLite persistence
    pub fn with_persistence(db_path: PathBuf) -> Result<Self> {
        let registry = RunRegistry::with_persistence(db_path)?;
        Ok(Self { registry })
    }

    /// Schedule and execute a flow
    pub async fn schedule_flow(
        &mut self,
        flow_id: String,
        mut flow: Flow,
        mut initial_state: SharedState,
    ) -> Result<String> {
        let mut run = FlowRun::new(flow_id.clone());
        run.mark_running();

        let run_id = run.id.clone();
        self.registry.register(run);

        // Execute the flow
        let result = flow.run(&mut initial_state).await;

        // Update run status
        let run_status = if let Some(run) = self.registry.get_mut(&run_id) {
            match result {
                Ok(_) => run.mark_completed(initial_state),
                Err(e) => run.mark_failed(e.to_string()),
            }
            run.status.clone()
        } else {
            return Err(anyhow::anyhow!("Run not found"));
        };

        // Persist the updated run
        if let Some(run) = self.registry.get(&run_id) {
            let mut run_clone = run.clone();
            run_clone.status = run_status;
            let _ = self.registry.update_run(&run_clone);
        }

        Ok(run_id)
    }

    /// Get the run registry
    pub fn registry(&self) -> &RunRegistry {
        &self.registry
    }

    /// Get mutable reference to the run registry
    pub fn registry_mut(&mut self) -> &mut RunRegistry {
        &mut self.registry
    }

    /// Load runs from database on startup
    pub fn load_from_db(&mut self) -> Result<()> {
        self.registry.load_from_db()
    }
}

impl Default for Maestro {
    fn default() -> Self {
        Self::new()
    }
}

/// Continuation engine - checkpointing and resume functionality
pub struct ContinuationEngine {
    checkpoint_dir: PathBuf,
}

impl ContinuationEngine {
    /// Create a new continuation engine with a checkpoint directory
    pub fn new(checkpoint_dir: PathBuf) -> Self {
        Self { checkpoint_dir }
    }

    /// Create checkpoint directory if it doesn't exist
    fn ensure_checkpoint_dir(&self) -> Result<()> {
        if !self.checkpoint_dir.exists() {
            fs::create_dir_all(&self.checkpoint_dir).with_context(|| {
                format!(
                    "Failed to create checkpoint directory: {}",
                    self.checkpoint_dir.display()
                )
            })?;
        }
        Ok(())
    }

    /// Save a checkpoint for a flow run
    pub fn save_checkpoint(&self, run_id: &str, state: &SharedState) -> Result<PathBuf> {
        self.ensure_checkpoint_dir()?;

        let checkpoint_path = self.checkpoint_dir.join(format!("{}.json", run_id));
        let state_json = serde_json::to_string_pretty(state)
            .with_context(|| "Failed to serialize SharedState")?;

        fs::write(&checkpoint_path, state_json).with_context(|| {
            format!("Failed to write checkpoint: {}", checkpoint_path.display())
        })?;

        Ok(checkpoint_path)
    }

    /// Load a checkpoint for a flow run
    pub fn load_checkpoint(&self, run_id: &str) -> Result<SharedState> {
        let checkpoint_path = self.checkpoint_dir.join(format!("{}.json", run_id));

        if !checkpoint_path.exists() {
            anyhow::bail!("Checkpoint not found: {}", checkpoint_path.display());
        }

        let state_json = fs::read_to_string(&checkpoint_path)
            .with_context(|| format!("Failed to read checkpoint: {}", checkpoint_path.display()))?;

        let state: SharedState = serde_json::from_str(&state_json)
            .with_context(|| "Failed to deserialize SharedState")?;

        Ok(state)
    }

    /// Check if a checkpoint exists
    pub fn has_checkpoint(&self, run_id: &str) -> bool {
        let checkpoint_path = self.checkpoint_dir.join(format!("{}.json", run_id));
        checkpoint_path.exists()
    }

    /// Delete a checkpoint
    pub fn delete_checkpoint(&self, run_id: &str) -> Result<()> {
        let checkpoint_path = self.checkpoint_dir.join(format!("{}.json", run_id));

        if checkpoint_path.exists() {
            fs::remove_file(&checkpoint_path).with_context(|| {
                format!("Failed to delete checkpoint: {}", checkpoint_path.display())
            })?;
        }

        Ok(())
    }

    /// List all available checkpoints
    pub fn list_checkpoints(&self) -> Result<Vec<String>> {
        self.ensure_checkpoint_dir()?;

        let mut checkpoints = Vec::new();

        for entry in fs::read_dir(&self.checkpoint_dir).with_context(|| {
            format!(
                "Failed to read checkpoint directory: {}",
                self.checkpoint_dir.display()
            )
        })? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    checkpoints.push(stem.to_string());
                }
            }
        }

        Ok(checkpoints)
    }
}

impl Default for ContinuationEngine {
    fn default() -> Self {
        Self::new(PathBuf::from(".checkpoints"))
    }
}
