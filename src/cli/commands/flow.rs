//! Flow command handler

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use prometheos_lite::{
    config::AppConfig,
    flow::execution::{ContinuationEngine, FlowRun, RunDb},
    flow::testing::{FlowTestRunner, TestFixture},
    logger::Logger,
    personality::{ModeSelector, PersonalityMode},
};

use super::super::runner::FlowRunner;
use super::super::runtime_builder::RuntimeBuilder;

#[derive(Debug, Parser)]
pub struct FlowCommand {
    #[command(subcommand)]
    pub action: FlowAction,
}

#[derive(Debug, Subcommand)]
pub enum FlowAction {
    /// Run a flow from a JSON or YAML file
    Run(RunFlowCommand),
    /// Resume a paused flow run
    Resume(ResumeCommand),
    /// View events for a flow run
    Events(EventsCommand),
    /// Replay a previous flow run (observational only)
    Replay(ReplayCommand),
    /// Test a flow with fixtures
    Test(TestCommand),
}

#[derive(Debug, Parser)]
pub struct RunFlowCommand {
    /// Path to the flow file
    pub path: PathBuf,
    /// Input data for the flow (JSON string)
    #[arg(short, long)]
    pub input: Option<String>,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Enable debug mode with step-by-step execution
    #[arg(short, long)]
    pub debug: bool,
    /// Export timeline to file
    #[arg(long)]
    pub export_timeline: Option<PathBuf>,
    /// Maximum number of steps
    #[arg(long)]
    pub max_steps: Option<u32>,
    /// Maximum number of LLM calls
    #[arg(long)]
    pub max_llm_calls: Option<u32>,
    /// Maximum runtime in milliseconds
    #[arg(long)]
    pub max_runtime_ms: Option<u64>,
}

#[derive(Debug, Parser)]
pub struct ReplayCommand {
    /// Run ID to replay
    pub run_id: String,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Parser)]
pub struct ResumeCommand {
    /// Run ID to resume
    pub run_id: String,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Parser)]
pub struct EventsCommand {
    /// Run ID to view events for
    pub run_id: String,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Parser)]
pub struct TestCommand {
    /// Path to the flow file
    pub path: PathBuf,
    /// Path to fixture file (JSON)
    #[arg(short, long)]
    pub fixture: PathBuf,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

impl FlowCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        match &self.action {
            FlowAction::Run(cmd) => cmd.execute().await,
            FlowAction::Resume(cmd) => cmd.execute().await,
            FlowAction::Events(cmd) => cmd.execute().await,
            FlowAction::Replay(cmd) => cmd.execute().await,
            FlowAction::Test(cmd) => cmd.execute().await,
        }
    }
}

impl RunFlowCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Loading flow from: {}", self.path.display()));

        // Select personality mode based on input
        let mode_selector = ModeSelector::new(PersonalityMode::default());
        let input_text = self.input.as_deref().unwrap_or("");
        let selected_mode = mode_selector.select_from_text(input_text);

        if self.verbose {
            logger.info(&format!(
                "Personality mode: {}",
                selected_mode.display_name()
            ));
            logger.info(&format!("  {}", selected_mode.description()));
        }

        // Build runtime
        let config = AppConfig::load()?;
        let runtime = RuntimeBuilder::new(config)
            .build_full()
            .context("Failed to build runtime")?;

        // Initialize RunDb and ContinuationEngine
        let db_path = PathBuf::from(".prometheos/runs.db");
        let run_db = RunDb::new(db_path.clone())?;
        logger.info(&format!("RunDb initialized at: {}", db_path.display()));

        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = ContinuationEngine::new(checkpoint_dir.clone());
        logger.info(&format!(
            "ContinuationEngine initialized at: {}",
            checkpoint_dir.display()
        ));

        // Load flow based on file extension
        let mut flow_runner = if let Some(ext) = self.path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if ext == "yaml" || ext == "yml" {
                FlowRunner::from_yaml_file_with_runtime(self.path.clone(), Some(runtime))?
            } else {
                FlowRunner::from_json_file_with_runtime(self.path.clone(), Some(runtime))?
            }
        } else {
            FlowRunner::from_json_file_with_runtime(self.path.clone(), Some(runtime))?
        };
        logger.info("Flow loaded successfully");

        // Create FlowRun
        let flow_id = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let mut flow_run = FlowRun::new(flow_id);
        flow_run.mark_running();
        run_db.save_run(&flow_run)?;
        logger.info(&format!("FlowRun created with ID: {}", flow_run.id));

        let input_value = if let Some(input_str) = &self.input {
            serde_json::from_str(input_str).context("Failed to parse input JSON")?
        } else {
            serde_json::json!({})
        };

        // Enable debug mode if requested
        if self.debug {
            logger.info("Debug mode enabled");
            flow_runner.enable_debug_mode();
        }

        logger.info("Executing flow...");

        let execution_result = flow_runner.run_with_input(input_value).await;

        match &execution_result {
            Ok(state) => {
                flow_run.mark_completed(state.clone());
                continuation_engine.save_checkpoint(&flow_run.id, state)?;
                logger.info(&format!("Checkpoint saved for run: {}", flow_run.id));
            }
            Err(e) => {
                flow_run.mark_failed(e.to_string());
                logger.error(&format!("Flow execution failed: {}", e));
            }
        }

        run_db.save_run(&flow_run)?;
        logger.info(&format!("FlowRun saved with status: {:?}", flow_run.status));

        if let Ok(state) = execution_result {
            logger.success("Flow execution completed");

            // Render output
            println!(
                "\n[output]\n{}",
                serde_json::to_string_pretty(&state.get_all_outputs())?
            );

            // Export timeline if requested
            if let Some(timeline_path) = &self.export_timeline {
                if let Some(tracer) = flow_runner.tracer() {
                    if let Ok(t) = tracer.lock() {
                        let timeline_json =
                            t.export_timeline().context("Failed to export timeline")?;
                        std::fs::write(timeline_path, timeline_json)
                            .context("Failed to write timeline file")?;
                        logger.info(&format!(
                            "Timeline exported to: {}",
                            timeline_path.display()
                        ));
                    }
                } else {
                    eprintln!("Warning: No tracer available for timeline export");
                }
            }
        }

        Ok(())
    }
}

impl ReplayCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Replaying run: {}", self.run_id));

        // Initialize RunDb and ContinuationEngine
        let db_path = PathBuf::from(".prometheos/runs.db");
        let _run_db = RunDb::new(db_path.clone())?;
        logger.info(&format!("RunDb initialized at: {}", db_path.display()));

        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = ContinuationEngine::new(checkpoint_dir.clone());
        logger.info(&format!(
            "ContinuationEngine initialized at: {}",
            checkpoint_dir.display()
        ));

        // Check if checkpoint exists
        if !continuation_engine.has_checkpoint(&self.run_id) {
            anyhow::bail!("Checkpoint not found for run: {}", self.run_id);
        }

        // Load checkpoint
        let state = continuation_engine.load_checkpoint(&self.run_id)?;
        logger.info("Checkpoint loaded successfully");

        // Display run information
        println!("\n[run_id] {}", self.run_id);
        println!("[status] completed");
        println!("\n[outputs]");
        println!(
            "{}",
            serde_json::to_string_pretty(&state.get_all_outputs())?
        );

        // Display trace events if available
        if let Some(trace_events) = state.get_input("trace_events") {
            println!("\n[trace_events]");
            println!("{}", serde_json::to_string_pretty(trace_events)?);
        }

        logger.success("Replay completed");
        Ok(())
    }
}

impl ResumeCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Resuming run: {}", self.run_id));

        // Initialize RunDb and ContinuationEngine
        let db_path = PathBuf::from(".prometheos/runs.db");
        let run_db = RunDb::new(db_path.clone())?;
        logger.info(&format!("RunDb initialized at: {}", db_path.display()));

        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = ContinuationEngine::new(checkpoint_dir.clone());
        logger.info(&format!(
            "ContinuationEngine initialized at: {}",
            checkpoint_dir.display()
        ));

        // Check if checkpoint exists
        if !continuation_engine.has_checkpoint(&self.run_id) {
            anyhow::bail!("Checkpoint not found for run: {}", self.run_id);
        }

        // Load checkpoint
        let state = continuation_engine.load_checkpoint(&self.run_id)?;
        logger.info("Checkpoint loaded successfully");

        // Validate flow snapshot if available
        if let Some(flow_name) = state.get_input("flow_name").and_then(|v| v.as_str()) {
            let db = prometheos_lite::db::repository::Db::new(&db_path.to_string_lossy())?;
            if let Ok(snapshot) =
                prometheos_lite::db::repository::FlowSnapshotOperations::get_latest_flow_snapshot(
                    &db, flow_name,
                )
            {
                if let Some(stored_snapshot) = snapshot {
                    // Get current flow source from state
                    if let Some(current_source) =
                        state.get_input("flow_source").and_then(|v| v.as_str())
                    {
                        let current_hash =
                            prometheos_lite::flow::FlowSnapshot::compute_hash(current_source);

                        if current_hash != stored_snapshot.source_hash {
                            logger.error("Flow source hash mismatch!");
                            logger.error(&format!("Stored hash: {}", stored_snapshot.source_hash));
                            logger.error(&format!("Current hash: {}", current_hash));
                            anyhow::bail!(
                                "Cannot resume: flow definition has changed since run started"
                            );
                        }
                        logger.info("Flow snapshot validation passed");
                    }
                }
            }
        }

        // Resume execution from checkpoint
        // Load the flow and re-execute with checkpointed state
        if let Some(flow_name) = state.get_input("flow_name").and_then(|v| v.as_str()) {
            let flows_dir = PathBuf::from("flows");
            let flow_path = flows_dir.join(format!("{}.flow.yaml", flow_name));

            if !flow_path.exists() {
                // Try .yml extension
                let flow_path_yml = flows_dir.join(format!("{}.flow.yml", flow_name));
                if flow_path_yml.exists() {
                    logger.info(&format!("Found flow at: {}", flow_path_yml.display()));
                    // Re-execute the flow with loaded state
                    self.execute_resumed_flow(&flow_path_yml, state, &logger).await?;
                } else {
                    logger.warn(&format!("Flow file not found for: {}", flow_name));
                    anyhow::bail!("Cannot resume: flow file not found");
                }
            } else {
                logger.info(&format!("Found flow at: {}", flow_path.display()));
                // Re-execute the flow with loaded state
                self.execute_resumed_flow(&flow_path, state, &logger).await?;
            }
        } else {
            logger.warn("No flow_name found in checkpoint state");
            anyhow::bail!("Cannot resume: no flow_name in checkpoint");
        }

        logger.success("Resume command completed");
        Ok(())
    }

    /// Execute a resumed flow with checkpointed state
    async fn execute_resumed_flow(
        &self,
        flow_path: &PathBuf,
        mut state: prometheos_lite::flow::SharedState,
        logger: &Logger,
    ) -> anyhow::Result<()> {
        use prometheos_lite::flow::loader::{FlowLoader, JsonLoader, YamlLoader};
        use prometheos_lite::flow::{
            DefaultNodeFactory, Flow, FlowBuilder, NodeFactory, SharedState,
        };

        // Load the flow file based on extension
        let flow_file = if let Some(ext) = flow_path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if ext == "yaml" || ext == "yml" {
                let loader = YamlLoader::new();
                loader.load_from_path(flow_path)?
            } else {
                let loader = JsonLoader::new();
                loader.load_from_path(flow_path)?
            }
        } else {
            let loader = JsonLoader::new();
            loader.load_from_path(flow_path)?
        };
        logger.info(&format!("Loaded flow: {}", flow_file.name));

        // Build the flow
        let factory = DefaultNodeFactory::new();
        let mut builder = FlowBuilder::new();

        // Add nodes from flow file
        for node_def in &flow_file.nodes {
            let node = factory.create(&node_def.node_type, node_def.config.clone())?;
            builder = builder.add_node(node_def.id.clone(), node);
        }

        // Add transitions
        for trans in &flow_file.transitions {
            builder = builder
                .add_transition(trans.from.clone(), trans.action.clone(), trans.to.clone());
        }

        // Set start node
        builder = builder.start(flow_file.start_node.clone());

        // Build the flow
        let mut flow = builder.build()?;

        // Execute the flow with the loaded state
        logger.info("Resuming flow execution...");
        match flow.run(&mut state).await {
            Ok(()) => {
                logger.success("Flow execution completed successfully");

                // Save updated checkpoint
                let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
                let continuation_engine = ContinuationEngine::new(checkpoint_dir);
                continuation_engine.save_checkpoint(&self.run_id, &state)?;
                logger.info("Checkpoint saved with updated state");

                // Display results
                println!("\n[run_id] {}", self.run_id);
                println!("[status] completed");
                println!(
                    "\n[outputs]\n{}",
                    serde_json::to_string_pretty(&state.get_all_outputs())?
                );
            }
            Err(e) => {
                logger.error(&format!("Flow execution failed: {}", e));

                // Save checkpoint even on failure (preserves partial progress)
                let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
                let continuation_engine = ContinuationEngine::new(checkpoint_dir);
                let _ = continuation_engine.save_checkpoint(&self.run_id, &state);

                anyhow::bail!("Flow execution failed during resume: {}", e);
            }
        }

        Ok(())
    }
}

impl EventsCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Viewing events for run: {}", self.run_id));

        // Initialize RunDb and ContinuationEngine
        let db_path = PathBuf::from(".prometheos/runs.db");
        let _run_db = RunDb::new(db_path.clone())?;
        logger.info(&format!("RunDb initialized at: {}", db_path.display()));

        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = ContinuationEngine::new(checkpoint_dir.clone());
        logger.info(&format!(
            "ContinuationEngine initialized at: {}",
            checkpoint_dir.display()
        ));

        // Check if checkpoint exists
        if !continuation_engine.has_checkpoint(&self.run_id) {
            anyhow::bail!("Checkpoint not found for run: {}", self.run_id);
        }

        // Load checkpoint
        let state = continuation_engine.load_checkpoint(&self.run_id)?;
        logger.info("Checkpoint loaded successfully");

        // Extract trace events from state if available
        let events = if let Some(trace_events) = state.get_input("trace_events") {
            trace_events.clone()
        } else {
            serde_json::json!([])
        };

        let result = serde_json::json!({
            "run_id": self.run_id,
            "events": events,
            "count": if events.is_array() { events.as_array().map(|a| a.len()).unwrap_or(0) } else { 0 }
        });
        println!("{}", serde_json::to_string_pretty(&result)?);

        logger.success("Events command completed");
        Ok(())
    }
}

impl TestCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Testing flow: {}", self.path.display()));

        // Load fixture
        let fixture = TestFixture::from_json(&self.fixture)?;
        logger.info(&format!("Fixture loaded from: {}", self.fixture.display()));

        // Create test runner
        let test_runner = FlowTestRunner::new(self.path.clone()).with_tracing();

        // Run test
        logger.info("Running test...");
        let result = test_runner.run_test(&fixture).await?;

        if result.success {
            logger.success("Test passed");
            println!("\n[outputs]");
            println!("{}", serde_json::to_string_pretty(&result.outputs)?);
        } else {
            logger.error("Test failed");
            if let Some(error) = result.error {
                println!("\n[error] {}", error);
            }
        }

        Ok(())
    }
}
