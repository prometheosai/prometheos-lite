//! Orchestration layer - Maestro and continuation engine

use crate::flow::{Flow, SharedState};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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
}

impl RunRegistry {
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
        }
    }

    pub fn register(&mut self, run: FlowRun) {
        self.runs.insert(run.id.clone(), run);
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
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::new()
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
        if let Some(run) = self.registry.get_mut(&run_id) {
            match result {
                Ok(_) => run.mark_completed(initial_state),
                Err(e) => run.mark_failed(e.to_string()),
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowBuilder, Input, Node, NodeConfig, Output};
    use async_trait::async_trait;
    use std::sync::Arc;

    // Simple mock node for testing
    struct SimpleNode {
        id: String,
        config: NodeConfig,
    }

    impl SimpleNode {
        fn new(id: String) -> Self {
            Self {
                id,
                config: NodeConfig::default(),
            }
        }
    }

    #[async_trait]
    impl Node for SimpleNode {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn prep(&self, _state: &SharedState) -> Result<Input> {
            Ok(serde_json::json!({}))
        }

        async fn exec(&self, _input: Input) -> Result<Output> {
            Ok(serde_json::json!({}))
        }

        fn post(&self, _state: &mut SharedState, _output: Output) -> String {
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            self.config.clone()
        }
    }

    #[test]
    fn test_flow_run_creation() {
        let run = FlowRun::new("test_flow".to_string());
        assert_eq!(run.status, RunStatus::Pending);
        assert!(run.completed_at.is_none());
    }

    #[test]
    fn test_flow_run_status_transitions() {
        let mut run = FlowRun::new("test_flow".to_string());

        run.mark_running();
        assert_eq!(run.status, RunStatus::Running);

        let state = SharedState::new();
        run.mark_completed(state);
        assert_eq!(run.status, RunStatus::Completed);
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn test_flow_run_failure() {
        let mut run = FlowRun::new("test_flow".to_string());
        run.mark_failed("test error".to_string());

        assert!(matches!(run.status, RunStatus::Failed(_)));
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn test_run_registry() {
        let mut registry = RunRegistry::new();
        let run = FlowRun::new("test_flow".to_string());
        let run_id = run.id.clone();

        registry.register(run);

        assert!(registry.get(&run_id).is_some());
        assert_eq!(registry.list_by_flow("test_flow").len(), 1);
    }

    #[test]
    fn test_run_registry_list_active() {
        let mut registry = RunRegistry::new();

        let mut run1 = FlowRun::new("flow1".to_string());
        run1.mark_running();
        registry.register(run1);

        let mut run2 = FlowRun::new("flow2".to_string());
        run2.mark_completed(SharedState::new());
        registry.register(run2);

        let active = registry.list_active();
        assert_eq!(active.len(), 1);
    }

    #[tokio::test]
    async fn test_maestro_schedule_flow() {
        let mut maestro = Maestro::new();

        let node = SimpleNode::new("node1".to_string());
        let flow = FlowBuilder::new()
            .start("node1".to_string())
            .add_node("node1".to_string(), Arc::new(node))
            .build()
            .unwrap();

        let state = SharedState::new();
        let run_id = maestro
            .schedule_flow("test_flow".to_string(), flow, state)
            .await
            .unwrap();

        assert!(maestro.registry().get(&run_id).is_some());
        assert_eq!(
            maestro.registry().get(&run_id).unwrap().status,
            RunStatus::Completed
        );
    }

    #[test]
    fn test_continuation_engine_save_load() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = ContinuationEngine::new(temp_dir.path().to_path_buf());

        let mut state = SharedState::new();
        state.set_input("test".to_string(), serde_json::json!("value"));

        let run_id = "test_run_123";
        engine.save_checkpoint(run_id, &state).unwrap();

        assert!(engine.has_checkpoint(run_id));

        let loaded_state = engine.load_checkpoint(run_id).unwrap();
        assert_eq!(
            loaded_state.get_input("test"),
            Some(&serde_json::json!("value"))
        );
    }

    #[test]
    fn test_continuation_engine_list_checkpoints() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = ContinuationEngine::new(temp_dir.path().to_path_buf());

        let state = SharedState::new();
        engine.save_checkpoint("run1", &state).unwrap();
        engine.save_checkpoint("run2", &state).unwrap();

        let checkpoints = engine.list_checkpoints().unwrap();
        assert_eq!(checkpoints.len(), 2);
        assert!(checkpoints.contains(&"run1".to_string()));
        assert!(checkpoints.contains(&"run2".to_string()));
    }

    #[test]
    fn test_continuation_engine_delete_checkpoint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = ContinuationEngine::new(temp_dir.path().to_path_buf());

        let state = SharedState::new();
        engine.save_checkpoint("run1", &state).unwrap();

        assert!(engine.has_checkpoint("run1"));
        engine.delete_checkpoint("run1").unwrap();
        assert!(!engine.has_checkpoint("run1"));
    }
}
