//! WorkContext CLI commands

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

use prometheos_lite::db::Db;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::intent::IntentClassifier;
use prometheos_lite::work::{
    ExecutionLimits, PlaybookResolver, WorkContextService, WorkOrchestrator,
    evolution_engine::EvolutionEngine,
    execution_service::WorkExecutionService,
    template_loader::TemplateLoader,
    types::{WorkDomain, WorkStatus},
};

#[derive(Debug, Parser)]
pub struct WorkCommand {
    #[command(subcommand)]
    command: WorkSubcommand,
}

#[derive(Debug, Subcommand)]
enum WorkSubcommand {
    /// Create a new WorkContext
    Create {
        /// Title for the work context
        title: String,
        /// Domain of work (software, business, marketing, personal, creative, research, operations, general)
        #[arg(short, long, default_value = "general")]
        domain: String,
        /// Goal description
        goal: String,
    },
    /// List all WorkContexts
    List,
    /// Show details of a specific WorkContext
    Show {
        /// WorkContext ID
        id: String,
    },
    /// List artifacts for a WorkContext
    Artifacts {
        /// WorkContext ID
        id: String,
    },
    /// Submit a user intent to create or attach to a WorkContext
    Submit {
        /// User message/intent
        message: String,
        /// Optional conversation ID
        #[arg(short, long)]
        conversation_id: Option<String>,
    },
    /// Continue a blocked WorkContext
    Continue {
        /// WorkContext ID
        id: String,
    },
    /// Run a WorkContext until blocked or complete
    Run {
        /// WorkContext ID
        id: String,
        /// Max iterations
        #[arg(short, long)]
        max_iterations: Option<u32>,
        /// Max runtime in milliseconds
        #[arg(short, long)]
        max_runtime_ms: Option<u64>,
    },
    /// Set status of a WorkContext
    SetStatus {
        /// WorkContext ID
        id: String,
        /// New status (draft, in_progress, awaiting_approval, completed, blocked)
        status: String,
    },
    /// Show persisted harness token/cost metrics
    Cost {
        /// WorkContext ID
        id: String,
    },
    /// Show persisted harness quality metrics
    Quality {
        /// WorkContext ID
        id: String,
    },
    /// Show persisted harness traces
    Traces {
        /// WorkContext ID
        id: String,
        /// Optional run ID filter
        #[arg(short, long)]
        run_id: Option<String>,
    },
    /// Harness commands for v1.6 integration
    Harness {
        #[command(subcommand)]
        command: HarnessSubcommand,
    },
}

#[derive(Debug, Subcommand)]
enum HarnessSubcommand {
    /// Run harness on a WorkContext
    Run {
        /// WorkContext ID
        id: String,
        /// Harness mode (auto, assisted, dry_run)
        #[arg(short, long, default_value = "auto")]
        mode: String,
        /// Repository root path
        #[arg(short, long)]
        repo_root: Option<String>,
    },
    /// Replay harness execution from trajectory
    Replay {
        /// WorkContext ID
        id: String,
        /// Step number to replay from (optional)
        #[arg(short, long)]
        step: Option<usize>,
    },
    /// Run benchmark on WorkContext
    Benchmark {
        /// WorkContext ID
        id: String,
        /// Benchmark type (performance, accuracy, quality)
        #[arg(short, long, default_value = "performance")]
        benchmark_type: String,
    },
    /// Show artifacts for WorkContext
    Artifact {
        /// WorkContext ID
        id: String,
        /// Artifact type (all, patches, evidence, trajectory)
        #[arg(short, long, default_value = "all")]
        artifact_type: String,
    },
    /// Show risk assessment for WorkContext
    Risk {
        /// WorkContext ID
        id: String,
        /// Risk level threshold (low, medium, high, critical)
        #[arg(short, long, default_value = "medium")]
        threshold: String,
    },
    /// Show completion status and evidence
    Completion {
        /// WorkContext ID
        id: String,
        /// Show detailed completion evidence
        #[arg(short, long)]
        detailed: bool,
    },
}

impl WorkCommand {
    pub async fn execute(self) -> Result<()> {
        let db_path = "prometheos.db";
        let db = Arc::new(Db::new(db_path)?);
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));

        // Ensure domain templates are installed
        let template_loader = TemplateLoader::from_default_templates_dir()?;
        template_loader.install_defaults()?;

        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime)?);
        let playbook_resolver = Arc::new(PlaybookResolver::new(db.clone()));
        let intent_classifier = Arc::new(IntentClassifier::new()?);
        let work_execution_service = Arc::new(WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service.clone(),
        ));
        let evolution_engine = Arc::new(EvolutionEngine::new(db.clone()));
        let work_orchestrator = Arc::new(WorkOrchestrator::new(
            work_context_service.clone(),
            playbook_resolver,
            work_execution_service,
            intent_classifier,
            evolution_engine,
        ));

        match self.command {
            WorkSubcommand::Create {
                title,
                domain,
                goal,
            } => {
                let domain = match domain.to_lowercase().as_str() {
                    "software" => WorkDomain::Software,
                    "business" => WorkDomain::Business,
                    "marketing" => WorkDomain::Marketing,
                    "personal" => WorkDomain::Personal,
                    "creative" => WorkDomain::Creative,
                    "research" => WorkDomain::Research,
                    "operations" => WorkDomain::Operations,
                    _ => WorkDomain::General,
                };

                let context = work_context_service.create_context(
                    "cli-user".to_string(),
                    title,
                    domain,
                    goal,
                )?;

                println!("Created WorkContext:");
                println!("  ID: {}", context.id);
                println!("  Title: {}", context.title);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
            }
            WorkSubcommand::List => {
                let contexts = work_context_service.list_contexts("cli-user")?;

                println!("WorkContexts ({}):", contexts.len());
                for ctx in contexts {
                    println!("  {} - {} ({:?})", ctx.id, ctx.title, ctx.status);
                }
            }
            WorkSubcommand::Show { id } => {
                let context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                println!("WorkContext Details:");
                println!("  ID: {}", context.id);
                println!("  Title: {}", context.title);
                println!("  Domain: {:?}", context.domain);
                println!("  Goal: {}", context.goal);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
                println!("  Priority: {:?}", context.priority);
                println!("  Autonomy: {:?}", context.autonomy_level);
                println!("  Approval Policy: {:?}", context.approval_policy);
                println!("  Artifacts: {}", context.artifacts.len());
                println!(
                    "  Completion Criteria: {}",
                    context.completion_criteria.len()
                );

                if let Some(due) = &context.due_at {
                    println!("  Due At: {}", due);
                }
                if let Some(blocked) = &context.blocked_reason {
                    println!("  Blocked: {}", blocked);
                }
            }
            WorkSubcommand::Artifacts { id } => {
                let context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                println!(
                    "Artifacts for WorkContext {} ({}):",
                    context.id, context.title
                );
                if context.artifacts.is_empty() {
                    println!("  No artifacts");
                } else {
                    for artifact in &context.artifacts {
                        println!(
                            "  {} - {} ({:?})",
                            artifact.id, artifact.name, artifact.kind
                        );
                        println!("    Created by: {}", artifact.created_by);
                        println!("    Storage: {:?}", artifact.storage);
                        println!("    Created at: {}", artifact.created_at);
                    }
                }
            }
            WorkSubcommand::Submit {
                message,
                conversation_id,
            } => {
                let context = work_orchestrator
                    .submit_user_intent("cli-user".to_string(), message, conversation_id)
                    .await?;

                println!("Submitted intent to WorkContext:");
                println!("  ID: {}", context.id);
                println!("  Title: {}", context.title);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
            }
            WorkSubcommand::Continue { id } => {
                let context = work_orchestrator.continue_context(id).await?;

                println!("Continued WorkContext:");
                println!("  ID: {}", context.id);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
            }
            WorkSubcommand::Run {
                id,
                max_iterations,
                max_runtime_ms,
            } => {
                let limits = ExecutionLimits::default()
                    .with_max_iterations(max_iterations.unwrap_or(10))
                    .with_max_runtime_ms(max_runtime_ms.unwrap_or(300_000));

                let context = work_orchestrator
                    .run_until_blocked_or_complete(id, limits)
                    .await?;

                println!("Ran WorkContext:");
                println!("  ID: {}", context.id);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
                if let Some(blocked) = &context.blocked_reason {
                    println!("  Blocked: {}", blocked);
                }
            }
            WorkSubcommand::SetStatus { id, status } => {
                let mut context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                let new_status = match status.to_lowercase().as_str() {
                    "draft" => WorkStatus::Draft,
                    "in_progress" => WorkStatus::InProgress,
                    "awaiting_approval" => WorkStatus::AwaitingApproval,
                    "completed" => WorkStatus::Completed,
                    "blocked" => WorkStatus::Blocked,
                    _ => return Err(anyhow::anyhow!("Invalid status: {}", status)),
                };

                work_context_service.update_status(&mut context, new_status)?;

                println!("Updated WorkContext status to {:?}", new_status);
            }
            WorkSubcommand::Cost { id } => {
                let context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;
                let usage = context.harness_metadata().and_then(|m| m.token_usage);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "token_usage": usage.unwrap_or_default()
                    }))?
                );
            }
            WorkSubcommand::Quality { id } => {
                let context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;
                let hm = context.harness_metadata();
                let quality_metrics = hm
                    .as_ref()
                    .and_then(|m| m.quality_metrics.clone())
                    .unwrap_or_default();
                let harness = context
                    .metadata
                    .get("harness")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "quality_metrics": quality_metrics,
                        "review": harness.get("review_issues").cloned().unwrap_or(serde_json::Value::Null),
                        "risk": harness.get("risk_assessment").cloned().unwrap_or(serde_json::Value::Null),
                        "completion": harness.get("completion_decision").cloned().unwrap_or(serde_json::Value::Null),
                    }))?
                );
            }
            WorkSubcommand::Traces { id, run_id } => {
                let context = work_context_service
                    .get_context(&id)?
                    .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;
                let latest = context.harness_metadata().and_then(|m| m.latest_run_id);
                if let Some(filter_run_id) = run_id
                    && latest.as_deref() != Some(filter_run_id.as_str()) {
                        anyhow::bail!(
                            "Run '{}' not found for work context '{}'",
                            filter_run_id,
                            id
                        );
                    }
                let harness = context
                    .metadata
                    .get("harness")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "latest_run_id": latest,
                        "trace_summary": context.harness_metadata().and_then(|m| m.trace_summary).unwrap_or_default(),
                        "trajectory": harness.get("trajectory").cloned().unwrap_or(serde_json::Value::Null),
                    }))?
                );
            }
            WorkSubcommand::Harness { command } => {
                match command {
                    HarnessSubcommand::Run {
                        id,
                        mode,
                        repo_root,
                    } => {
                        // Create harness service
                        let harness_service =
                            prometheos_lite::harness::HarnessWorkContextService::new(
                                work_context_service.clone(),
                            );

                        let harness_mode = match mode.to_lowercase().as_str() {
                            "auto" => {
                                prometheos_lite::harness::mode_policy::HarnessMode::Autonomous
                            }
                            "assisted" => {
                                prometheos_lite::harness::mode_policy::HarnessMode::Assisted
                            }
                            "dry_run" => {
                                prometheos_lite::harness::mode_policy::HarnessMode::ReviewOnly
                            }
                            _ => return Err(anyhow::anyhow!("Invalid mode: {}", mode)),
                        };

                        let repo_path = repo_root.unwrap_or_else(|| ".".to_string());

                        println!(
                            "Running harness on WorkContext {} with mode {:?}",
                            id, harness_mode
                        );
                        println!("Repository root: {}", repo_path);

                        // Check if context exists
                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        println!("WorkContext found: {} - {}", context.title, context.goal);

                        let result = harness_service
                            .run_for_context(&id, repo_path.into(), harness_mode, Vec::new())
                            .await?;

                        println!("Harness summary: {}", result.summary);
                        println!("Completion: {:?}", result.completion_decision);
                        println!("Risk: {:?}", result.risk_assessment.level);
                        println!("Review issues: {}", result.review_issues.len());
                        println!("Evidence entries: {}", result.evidence_log.entries.len());
                    }
                    HarnessSubcommand::Replay { id, step } => {
                        println!("Replaying harness execution for WorkContext {}", id);
                        if let Some(step_num) = step {
                            println!("From step: {}", step_num);
                        }

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        if let Some(harness_data) = context.metadata.get("harness") {
                            if let Some(trajectory) = harness_data.get("trajectory") {
                                println!(
                                    "Trajectory data: {}",
                                    serde_json::to_string_pretty(trajectory)?
                                );
                            } else {
                                println!("No trajectory data found");
                            }
                        } else {
                            println!("No harness metadata found");
                        }
                    }
                    HarnessSubcommand::Benchmark { id, benchmark_type } => {
                        println!("Running benchmark on WorkContext {}", id);
                        println!("Benchmark type: {}", benchmark_type);

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        println!("WorkContext: {} - {}", context.title, context.goal);
                        println!("Benchmark would run here with type: {}", benchmark_type);
                    }
                    HarnessSubcommand::Artifact { id, artifact_type } => {
                        println!("Showing artifacts for WorkContext {}", id);
                        println!("Artifact type: {}", artifact_type);

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        if let Some(harness_data) = context.metadata.get("harness") {
                            if let Some(artifacts) = harness_data.get("artifacts") {
                                println!("Artifacts: {}", serde_json::to_string_pretty(artifacts)?);
                            } else {
                                println!("No artifacts found");
                            }
                        } else {
                            println!("No harness metadata found");
                        }
                    }
                    HarnessSubcommand::Risk { id, threshold } => {
                        println!("Showing risk assessment for WorkContext {}", id);
                        println!("Risk threshold: {}", threshold);

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        if let Some(harness_data) = context.metadata.get("harness") {
                            if let Some(risk) = harness_data.get("risk_assessment") {
                                println!(
                                    "Risk assessment: {}",
                                    serde_json::to_string_pretty(risk)?
                                );
                            } else {
                                println!("No risk assessment found");
                            }
                        } else {
                            println!("No harness metadata found");
                        }
                    }
                    HarnessSubcommand::Completion { id, detailed } => {
                        println!("Showing completion status for WorkContext {}", id);
                        if detailed {
                            println!("Showing detailed completion evidence");
                        }

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        if let Some(harness_data) = context.metadata.get("harness") {
                            if let Some(completion) = harness_data.get("completion_decision") {
                                println!(
                                    "Completion decision: {}",
                                    serde_json::to_string_pretty(completion)?
                                );
                            } else {
                                println!("No completion decision found");
                            }
                        } else {
                            println!("No harness metadata found");
                        }

                        println!("WorkContext status: {:?}", context.status);
                        println!("Current phase: {:?}", context.current_phase);
                    }
                }
            }
        }

        Ok(())
    }
}
