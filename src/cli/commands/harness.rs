//! P2-014: Harness CLI golden path commands
//!
//! This module provides CLI commands for the V1.6 harness:
//! - harness run: Run harness on a task
//! - harness inspect: Inspect harness results
//! - harness dry-run: Dry-run patch application
//! - harness apply: Apply patches (with --assist flag)
//! - harness rollback: Rollback last harness execution

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Harness commands for autonomous/assisted coding
#[derive(Debug, Parser)]
pub struct HarnessCommand {
    #[command(subcommand)]
    command: HarnessSubcommand,
}

#[derive(Debug, Subcommand)]
enum HarnessSubcommand {
    /// Run harness on a task prompt
    ///
    /// Examples:
    ///   prometheos harness run "fix failing tests"
    ///   prometheos harness run "refactor auth module" --mode assisted
    Run {
        /// The task description
        task: String,
        /// Execution mode (review-only, assisted, autonomous)
        #[arg(short, long, default_value = "assisted")]
        mode: String,
        /// Repository root (defaults to current directory)
        #[arg(short, long)]
        repo: Option<PathBuf>,
        /// Output format (text, json)
        #[arg(short, long, default_value = "text")]
        format: String,
    },
    /// Inspect harness execution results
    ///
    /// Shows detailed information about a harness execution including:
    /// - Repository detected
    /// - Provider used
    /// - Files selected
    /// - Patch candidates
    /// - Validation results
    /// - Risk/review assessment
    Inspect {
        /// Execution ID to inspect
        #[arg(short, long)]
        execution_id: String,
        /// Show full evidence log
        #[arg(long)]
        evidence: bool,
    },
    /// Dry-run patch application
    ///
    /// Applies patches to a temporary workspace without modifying the real repo.
    /// Shows what would change without actually changing anything.
    DryRun {
        /// The task description
        task: String,
        /// Repository root (defaults to current directory)
        #[arg(short, long)]
        repo: Option<PathBuf>,
    },
    /// Apply harness-generated patches
    ///
    /// Applies patches from a previous harness execution.
    /// Use --assist flag for interactive approval.
    Apply {
        /// Execution ID from a previous dry-run
        #[arg(short, long)]
        execution_id: String,
        /// Assisted mode - requires explicit approval
        #[arg(long)]
        assist: bool,
        /// Skip confirmation prompts (dangerous!)
        #[arg(long)]
        force: bool,
    },
    /// Rollback last harness execution
    ///
    /// Restores the repository to the state before the last harness execution.
    Rollback {
        /// Execution ID to rollback
        #[arg(short, long)]
        execution_id: String,
        /// Skip confirmation
        #[arg(long)]
        force: bool,
    },
    /// Show harness status and recent executions
    Status,
}

impl HarnessCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            HarnessSubcommand::Run {
                task,
                mode,
                repo,
                format,
            } => execute_run(task, mode, repo, format).await,
            HarnessSubcommand::Inspect {
                execution_id,
                evidence,
            } => execute_inspect(execution_id, evidence).await,
            HarnessSubcommand::DryRun { task, repo } => execute_dry_run(task, repo).await,
            HarnessSubcommand::Apply {
                execution_id,
                assist,
                force,
            } => execute_apply(execution_id, assist, force).await,
            HarnessSubcommand::Rollback {
                execution_id,
                force,
            } => execute_rollback(execution_id, force).await,
            HarnessSubcommand::Status => execute_status().await,
        }
    }
}

/// P2-014: Execute harness run command
async fn execute_run(
    task: String,
    mode: String,
    repo: Option<PathBuf>,
    format: String,
) -> Result<()> {
    let repo_root = repo.unwrap_or_else(|| std::env::current_dir().unwrap());
    let mode = parse_harness_mode(&mode)?;

    let request = prometheos_lite::harness::HarnessExecutionRequest {
        work_context_id: uuid::Uuid::new_v4().to_string(),
        repo_root,
        task,
        requirements: Vec::new(),
        acceptance_criteria: Vec::new(),
        mode,
        limits: prometheos_lite::harness::HarnessLimits::default(),
        mentioned_files: Vec::new(),
        mentioned_symbols: Vec::new(),
        proposed_edits: Vec::new(),
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy:
            prometheos_lite::harness::ValidationFailurePolicy::RollbackAutomatically,
        sandbox_policy: Some(prometheos_lite::harness::SandboxPolicy::from_mode(mode)),
    }
    .with_config_provider()?;

    let result = prometheos_lite::harness::execute_harness_task(request).await?;
    print_execution_result(&result, &format)?;

    Ok(())
}

/// P2-014: Execute harness inspect command
async fn execute_inspect(execution_id: String, show_evidence: bool) -> Result<()> {
    let _ = show_evidence;
    bail!(
        "Harness execution inspection requires a persisted execution store; execution '{}' was not found in the standalone CLI context",
        execution_id
    )
}

/// P2-014: Execute harness dry-run command
async fn execute_dry_run(task: String, repo: Option<PathBuf>) -> Result<()> {
    execute_run(task, "review-only".to_string(), repo, "text".to_string()).await
}

/// P2-014: Execute harness apply command
async fn execute_apply(execution_id: String, assist: bool, force: bool) -> Result<()> {
    let _ = (assist, force);
    bail!(
        "Harness apply requires a persisted execution store and rollback metadata; execution '{}' is not available in the standalone CLI context",
        execution_id
    )
}

/// P2-014: Execute harness rollback command
async fn execute_rollback(execution_id: String, force: bool) -> Result<()> {
    let _ = force;
    bail!(
        "Harness rollback requires persisted checkpoint metadata; execution '{}' is not available in the standalone CLI context",
        execution_id
    )
}

/// P2-014: Execute harness status command
async fn execute_status() -> Result<()> {
    println!("📊 Harness Status");
    println!("═════════════════");
    println!();
    println!("Recent executions: unavailable in standalone CLI context");
    println!();
    println!("Available commands:");
    println!("  prometheos harness run \"<task>\"     - Run harness on a task");
    println!("  prometheos harness inspect --execution-id <id> - Inspect results");
    println!("  prometheos harness dry-run \"<task>\" - Dry-run without applying");
    println!("  prometheos harness apply --execution-id <id>  - Apply patches");
    println!("  prometheos harness rollback --execution-id <id> - Rollback changes");

    Ok(())
}

fn parse_harness_mode(mode: &str) -> Result<prometheos_lite::harness::mode_policy::HarnessMode> {
    match mode.to_lowercase().replace('_', "-").as_str() {
        "review" | "review-only" | "dry-run" => {
            Ok(prometheos_lite::harness::mode_policy::HarnessMode::ReviewOnly)
        }
        "assisted" => Ok(prometheos_lite::harness::mode_policy::HarnessMode::Assisted),
        "auto" | "autonomous" => Ok(prometheos_lite::harness::mode_policy::HarnessMode::Autonomous),
        "benchmark" => Ok(prometheos_lite::harness::mode_policy::HarnessMode::Benchmark),
        other => bail!(
            "Invalid harness mode '{}'. Expected review-only, assisted, autonomous, or benchmark",
            other
        ),
    }
}

fn print_execution_result(
    result: &prometheos_lite::harness::HarnessExecutionResult,
    format: &str,
) -> Result<()> {
    match format.to_lowercase().as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(result)?);
        }
        "text" => {
            println!("Harness execution complete");
            println!("Work context: {}", result.work_context_id);
            println!("Summary: {}", result.summary);
            println!("Completion: {:?}", result.completion_decision);
            println!("Risk: {:?}", result.risk_assessment.level);
            println!("Review issues: {}", result.review_issues.len());
            println!("Failures: {}", result.failures.len());
            println!("Evidence entries: {}", result.evidence_log.entries.len());
        }
        other => bail!(
            "Unsupported output format '{}'. Expected text or json",
            other
        ),
    }

    Ok(())
}
