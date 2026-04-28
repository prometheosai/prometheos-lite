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

        let mut flow_runner =
            FlowRunner::from_json_file_with_runtime(self.path.clone(), Some(runtime))?;
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

        // TODO: Resume execution from checkpoint
        // This requires integration with FlowExecutionService
        let result = serde_json::json!({
            "run_id": self.run_id,
            "status": "resumed",
            "message": "Flow resume not yet fully implemented - requires FlowExecutionService integration"
        });
        println!("{}", serde_json::to_string_pretty(&result)?);

        logger.success("Resume command completed");
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

        // TODO: Extract and display trace events from state
        let result = serde_json::json!({
            "run_id": self.run_id,
            "events": [],
            "message": "Events command not yet fully implemented - requires tracer integration"
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
