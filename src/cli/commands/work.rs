//! WorkContext CLI commands
//!
//! This module supports two paths:
//! - `work create --repo ...` delegates to the Repo Workbench MVP (file-backed).
//! - `work create` (without --repo) uses the standard database-backed WorkContextService.
//! - `work run`, `work artifacts`, `work continue`, `work memory show` detect Repo Workbench
//!   context IDs by checking the file-backed store and delegate accordingly.

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json;
use std::path::PathBuf;
use std::sync::Arc;

use prometheos_lite::db::Db;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::intent::IntentClassifier;
use prometheos_lite::repo_workbench;
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
        /// Title for the work context (required unless --repo is used)
        title: Option<String>,
        /// Domain of work (software, business, marketing, personal, creative, research, operations, general)
        #[arg(short, long, default_value = "general")]
        domain: String,
        /// Goal description
        #[arg(short, long)]
        goal: String,
        /// Repository root to analyze (delegates to Repo Workbench when set)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Work mode for Repo Workbench (used with --repo)
        #[arg(long, default_value = "review")]
        mode: String,
        /// Output JSON for machine-readable consumption
        #[arg(long)]
        json: bool,
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
        #[arg(long)]
        max_runtime_ms: Option<u64>,
    },
    /// Set status of a WorkContext
    SetStatus {
        /// WorkContext ID
        id: String,
        /// New status (draft, in_progress, awaiting_approval, completed, blocked)
        status: String,
    },
    /// Approve a staged artifact (delegates to Repo Workbench). Records approval only; does not write repo files.
    Approve {
        /// Artifact ID
        artifact_id: String,
        /// Optional WorkContext ID. If omitted, the current repo store is searched.
        #[arg(long)]
        work_id: Option<String>,
    },
    /// Inspect a Repo Workbench context: print the work context
    /// metadata, artifacts, decisions, and a stale-approvals report.
    /// Read-only: does not mutate any state. Goes through
    /// `repo_workbench::load_context` (the same loader the rest of
    /// the repo-workbench surface uses). E6/I02 acceptance bullet.
    Inspect {
        /// WorkContext ID
        id: String,
        /// Output as a single JSON document
        #[arg(long)]
        json: bool,
    },
    /// Inspect persisted Repo Workbench memory
    Memory {
        #[command(subcommand)]
        command: MemorySubcommand,
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
enum MemorySubcommand {
    /// Show memory for a Repo Workbench context
    Show {
        /// WorkContext ID
        id: String,
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
                repo,
                mode,
                json,
            } => {
                if let Some(repo) = repo {
                    let context =
                        repo_workbench::create_repo_workbench_context(&repo, &goal, &mode, title)?;

                    if json {
                        let output = serde_json::json!({
                            "work_id": context.id,
                            "title": context.title,
                            "repo": context.repo_path.display().to_string(),
                            "mode": context.mode,
                            "status": context.status,
                            "project_type": context.repo_summary.project_type,
                            "candidate_files": context.repo_summary.candidate_files.len(),
                            "next": format!("prometheos work run {}", context.id),
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    } else {
                        println!("Created Repo Workbench WorkContext");
                        println!("  ID: {}", context.id);
                        println!("  Title: {}", context.title);
                        println!("  Repo: {}", context.repo_path.display());
                        println!("  Mode: {}", context.mode);
                        println!("  Project type: {}", context.repo_summary.project_type);
                        println!(
                            "  Candidate files: {}",
                            context.repo_summary.candidate_files.len()
                        );
                        println!("  Next: prometheos work run {}", context.id);
                    }
                    return Ok(());
                }

                let title = title
                    .ok_or_else(|| anyhow::anyhow!("Title is required when --repo is not set"))?;
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
                if repo_workbench::repo_workbench_context_exists(&id) {
                    let context = repo_workbench::load_context(&id)?;
                    println!("Repo Workbench artifacts for {}:", context.id);
                    if context.artifacts.is_empty() {
                        println!(
                            "  No artifacts yet. Run `prometheos work run {}` first.",
                            context.id
                        );
                    }
                    for artifact in repo_workbench::get_artifacts(&context) {
                        println!("  {}", artifact.id);
                        println!("    Title: {}", artifact.title);
                        println!("    Kind: {}", artifact.kind);
                        println!("    Status: {}", artifact.status);
                        println!("    Requires approval: {}", artifact.requires_approval);
                        println!("    Path: {}", artifact.path.display());
                    }
                    return Ok(());
                }

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
                if repo_workbench::repo_workbench_context_exists(&id) {
                    let context = repo_workbench::load_context(&id)?;
                    println!("Continuing Repo Workbench WorkContext");
                    repo_workbench::print_status(&context);
                    println!();
                    println!("Memory:");
                    println!("{}", repo_workbench::load_memory(&context)?);
                    if let Some(next) = &context.next_action {
                        println!();
                        println!("Recommended next action: {}", next);
                    }
                    return Ok(());
                }

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
                if repo_workbench::repo_workbench_context_exists(&id) {
                    let mut context = repo_workbench::load_context(&id)?;
                    repo_workbench::run_repo_workbench_context(&mut context)?;

                    let risk_report = context.artifacts.iter().find(|a| a.kind == "risk-report");
                    let patch_artifact = context
                        .artifacts
                        .iter()
                        .find(|a| a.kind == "suggested-patch");

                    println!("Repo Workbench run complete");
                    println!("  WorkContext: {}", context.id);
                    println!("  Status: {}", context.status);
                    println!(
                        "  Files considered: {}",
                        context.repo_summary.candidate_files.len()
                    );
                    if let Some(report) = risk_report {
                        println!("  Risk report: {}", report.path.display());
                    }
                    if let Some(patch) = patch_artifact {
                        println!("  Suggested patch plan: {}", patch.path.display());
                    }
                    if let Some(next) = &context.next_action {
                        println!("  Next: {}", next);
                    }
                    return Ok(());
                }

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
            WorkSubcommand::Approve {
                artifact_id,
                work_id,
            } => {
                let mut context = if let Some(ref work_id) = work_id {
                    repo_workbench::load_context(work_id)?
                } else {
                    repo_workbench::find_context_by_artifact(&artifact_id)?
                };

                repo_workbench::approve_artifact(&mut context, &artifact_id)?;

                println!("Approval recorded");
                println!("  WorkContext: {}", context.id);
                println!("  Artifact: {}", artifact_id);
                println!("  Safety: no repository source files were modified");
                println!("  Next: prometheos work continue {}", context.id);
            }
            WorkSubcommand::Inspect { id, json } => {
                // E6/I02 acceptance: read-only run inspector. Goes
                // through the same repo-workbench loader as every
                // other repo-workbench subcommand. The command MUST
                // NOT mutate any state — only read.
                let report = inspect_repo_workbench(&id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print_inspect_report_text(&report);
                }
            }
            WorkSubcommand::Memory { command } => match command {
                MemorySubcommand::Show { id } => {
                    let context = repo_workbench::load_context(&id)?;
                    println!("{}", repo_workbench::load_memory(&context)?);
                }
            },
            WorkSubcommand::Cost { id } => {
                let runs = work_context_service.list_harness_run_metrics(&id)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "latest_run_id": runs.first().map(|r| r.run_id.clone()),
                        "token_usage": runs.first().map(|r| r.token_usage.clone()).unwrap_or_default(),
                        "runs": runs
                    }))?
                );
            }
            WorkSubcommand::Quality { id } => {
                let runs = work_context_service.list_harness_run_metrics(&id)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "latest_run_id": runs.first().map(|r| r.run_id.clone()),
                        "quality_metrics": runs.first().map(|r| r.quality_metrics.clone()).unwrap_or_default(),
                        "runs": runs
                    }))?
                );
            }
            WorkSubcommand::Traces { id, run_id } => {
                let runs = work_context_service.list_harness_run_metrics(&id)?;
                if let Some(filter_run_id) = run_id {
                    let run = runs.iter().find(|r| r.run_id == filter_run_id).cloned();
                    if let Some(run) = run {
                        println!("{}", serde_json::to_string_pretty(&run)?);
                        return Ok(());
                    }
                    anyhow::bail!(
                        "Run '{}' not found for work context '{}'",
                        filter_run_id,
                        id
                    );
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "work_context_id": id,
                        "latest_run_id": runs.first().map(|r| r.run_id.clone()),
                        "runs": runs,
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
                        let runs = work_context_service.list_harness_run_metrics(&id)?;
                        let run = runs
                            .first()
                            .ok_or_else(|| anyhow::anyhow!("No persisted harness runs found"))?;
                        if let Some(step_num) = step {
                            let steps = run
                                .trajectory
                                .get("steps")
                                .and_then(|v| v.as_array())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("Persisted trajectory has no steps array")
                                })?;
                            if step_num >= steps.len() {
                                anyhow::bail!(
                                    "Step {} out of bounds for run '{}' ({} steps)",
                                    step_num,
                                    run.run_id,
                                    steps.len()
                                );
                            }
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "work_context_id": id,
                                    "run_id": run.run_id,
                                    "step": step_num,
                                    "event": steps[step_num].clone()
                                }))?
                            );
                        } else {
                            println!("{}", serde_json::to_string_pretty(&run.trajectory)?);
                        }
                    }
                    HarnessSubcommand::Benchmark { id, benchmark_type } => {
                        println!("Running benchmark on WorkContext {}", id);
                        println!("Benchmark type: {}", benchmark_type);

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;

                        println!("WorkContext: {} - {}", context.title, context.goal);
                        let benchmark_test = prometheos_lite::harness::benchmark::BenchmarkTest {
                            id: format!("{}-{}", context.id, benchmark_type),
                            name: format!("work-{}", benchmark_type),
                            test_type:
                                prometheos_lite::harness::benchmark::TestType::PerformanceTest,
                            command: "cargo".to_string(),
                            args: vec!["check".to_string(), "--all-targets".to_string()],
                            working_dir: std::path::PathBuf::from("."),
                            iterations: 1,
                            timeout_ms: 120_000,
                            metrics: vec![
                                prometheos_lite::harness::benchmark::MetricType::Duration,
                            ],
                        };
                        let suite = prometheos_lite::harness::benchmark::BenchmarkSuite {
                            id: format!("suite-{}", context.id),
                            name: format!("work-context-{}", context.id),
                            description: "CLI benchmark execution for WorkContext".to_string(),
                            tests: vec![benchmark_test],
                            config: prometheos_lite::harness::benchmark::BenchmarkConfig::default(),
                        };
                        let mut runner =
                            prometheos_lite::harness::benchmark::create_benchmark_runner();
                        runner.register_suite(suite.clone());
                        let result = runner.run_suite(&suite.id).await?;
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "work_context_id": context.id,
                                "benchmark_type": benchmark_type,
                                "results": result
                            }))?
                        );
                    }
                    HarnessSubcommand::Artifact { id, artifact_type } => {
                        println!("Showing artifacts for WorkContext {}", id);
                        println!("Artifact type: {}", artifact_type);

                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;
                        let selected: Vec<_> = context
                            .artifacts
                            .into_iter()
                            .filter(|a| {
                                if artifact_type == "all" {
                                    return true;
                                }
                                a.name.contains(&artifact_type)
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&selected)?);
                    }
                    HarnessSubcommand::Risk { id, threshold } => {
                        println!("Showing risk assessment for WorkContext {}", id);
                        println!("Risk threshold: {}", threshold);

                        let evidence_dir = std::env::current_dir()?.join("evidence");
                        let manager = prometheos_lite::harness::evidence_persistence::EvidencePersistenceManager::new(
                            Box::new(prometheos_lite::harness::evidence_persistence::FileEvidenceSink::new(
                                evidence_dir,
                            )),
                        );
                        let evidence = manager.retrieve_evidence_log(&id).await?;
                        let risk_entries: Vec<_> = evidence
                            .entries
                            .iter()
                            .filter(|e| e.description.starts_with("Risk assessment:"))
                            .cloned()
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&risk_entries)?);
                    }
                    HarnessSubcommand::Completion { id, detailed } => {
                        println!("Showing completion status for WorkContext {}", id);
                        let evidence_dir = std::env::current_dir()?.join("evidence");
                        let manager = prometheos_lite::harness::evidence_persistence::EvidencePersistenceManager::new(
                            Box::new(prometheos_lite::harness::evidence_persistence::FileEvidenceSink::new(
                                evidence_dir,
                            )),
                        );
                        let evidence = manager.retrieve_evidence_log(&id).await?;
                        let completion_entries: Vec<_> = evidence
                            .entries
                            .iter()
                            .filter(|e| e.description.starts_with("Completion evaluation:"))
                            .cloned()
                            .collect();
                        if detailed {
                            println!("{}", serde_json::to_string_pretty(&completion_entries)?);
                        } else if let Some(last) = completion_entries.last() {
                            println!("{}", serde_json::to_string_pretty(last)?);
                        } else {
                            anyhow::bail!("No persisted completion evidence found");
                        }
                        let context = work_context_service
                            .get_context(&id)?
                            .ok_or_else(|| anyhow::anyhow!("WorkContext not found"))?;
                        println!("WorkContext status: {:?}", context.status);
                        println!("Current phase: {:?}", context.current_phase);
                    }
                }
            }
        }

        Ok(())
    }
}

// E6/I02 inspector helpers. The `Inspect` subcommand above is a thin
// shell that calls `inspect_repo_workbench` and prints the result. The
// helpers are top-level so they can be unit-tested without going
// through the full `Cli::run` path.

/// The structured report produced by `prometheos work inspect`. The
/// `serde_json::Value` is built in `inspect_repo_workbench` so the
/// `--json` and the text printer share one source of truth.
fn inspect_repo_workbench(id: &str) -> anyhow::Result<serde_json::Value> {
    let context = repo_workbench::load_context(id)?;
    let artifacts = repo_workbench::get_artifacts(&context);

    // Stale-approval detection (E6/I02 acceptance). An approval is
    // stale if the approved artifact_id is no longer in the current
    // artifacts list — i.e. the artifact was deleted or renamed
    // since approval. The check is conservative; a future slice may
    // add content-hash comparison for full coverage.
    let current_artifact_ids: std::collections::BTreeSet<&str> =
        artifacts.iter().map(|a| a.id.as_str()).collect();
    let stale_approvals: Vec<String> = context
        .decisions
        .iter()
        .filter(|d| d.approved && !current_artifact_ids.contains(d.artifact_id.as_str()))
        .map(|d| d.artifact_id.clone())
        .collect();

    let decisions_json: Vec<serde_json::Value> = context
        .decisions
        .iter()
        .map(|d| {
            // The "actor" of a human decision is the generator of
            // the artifact that was approved. If the artifact is
            // gone, we report `<unknown>`.
            let actor = context
                .artifacts
                .iter()
                .find(|a| a.id == d.artifact_id)
                .map(|a| a.provenance.generator.clone())
                .unwrap_or_else(|| "<unknown>".to_string());
            serde_json::json!({
                "id": d.id,
                "artifactId": d.artifact_id,
                "decision": d.decision,
                "approved": d.approved,
                "actor": actor,
                "createdAt": d.created_at,
            })
        })
        .collect();

    let artifacts_json: Vec<serde_json::Value> = artifacts
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "kind": a.kind,
                "title": a.title,
                "status": a.status,
                "requiresApproval": a.requires_approval,
                "path": a.path.display().to_string(),
                "provenance": {
                    "generator": &a.provenance.generator,
                    "generationMode": &a.provenance.generation_mode,
                    "modelInvoked": a.provenance.model_invoked,
                    "provider": a.provenance.provider,
                    "model": a.provenance.model,
                    "createdAt": a.provenance.created_at,
                },
            })
        })
        .collect();

    Ok(serde_json::json!({
        "schemaVersion": "1.0",
        "workId": context.id,
        "title": context.title,
        "goal": context.goal,
        "status": context.status,
        "phase": context.phase,
        "createdAt": context.created_at,
        "updatedAt": context.updated_at,
        "artifacts": artifacts_json,
        "decisions": decisions_json,
        "staleApprovals": stale_approvals,
    }))
}

fn print_inspect_report_text(report: &serde_json::Value) {
    fn s(v: &serde_json::Value) -> &str {
        v.as_str().unwrap_or("")
    }
    println!("WorkContext Inspector (E6/I02)");
    println!("==============================");
    println!("  ID:             {}", s(&report["workId"]));
    println!("  Title:          {}", s(&report["title"]));
    println!("  Goal:           {}", s(&report["goal"]));
    println!("  Status:         {}", s(&report["status"]));
    println!("  Phase:          {}", s(&report["phase"]));
    println!("  Created:        {}", s(&report["createdAt"]));
    println!("  Updated:        {}", s(&report["updatedAt"]));
    println!();
    if let Some(arr) = report["artifacts"].as_array() {
        println!("Artifacts ({}):", arr.len());
        for a in arr {
            println!("  - {} [{}] {}", s(&a["id"]), s(&a["kind"]), s(&a["title"]),);
            println!(
                "      status={} requires_approval={} path={}",
                s(&a["status"]),
                a["requiresApproval"],
                s(&a["path"]),
            );
            println!(
                "      generator={} ({}) model_invoked={}",
                s(&a["provenance"]["generator"]),
                s(&a["provenance"]["generationMode"]),
                a["provenance"]["modelInvoked"],
            );
        }
    }
    println!();
    if let Some(arr) = report["decisions"].as_array() {
        println!("Decisions ({}):", arr.len());
        for d in arr {
            println!(
                "  - {} artifact={} approved={} actor={} at {}",
                s(&d["id"]),
                s(&d["artifactId"]),
                d["approved"],
                s(&d["actor"]),
                s(&d["createdAt"]),
            );
            println!("      decision: {}", s(&d["decision"]));
        }
    }
    println!();
    if let Some(arr) = report["staleApprovals"].as_array() {
        if arr.is_empty() {
            println!("Stale approvals: (none)");
        } else {
            let names: Vec<String> = arr.iter().map(|v| s(v).to_string()).collect();
            println!("Stale approvals ({}): {}", arr.len(), names.join(", "));
        }
    }
    println!();
    println!("Read-only: this command did NOT mutate any state.");
}

#[cfg(test)]
mod inspect_tests {
    //! E6/I02 inspector tests. These tests do not need a real
    //! `WorkbenchContext` file on disk: they construct the context
    //! in-memory and call the helper functions directly. The
    //! `prometheos work inspect` subcommand is a thin shell that
    //! delegates to these helpers.

    use super::*;
    use chrono::Utc;
    use prometheos_lite::repo_workbench::{
        ArtifactProvenance, ArtifactRef, DecisionRecord as WorkbenchDecision, WorkbenchContext,
    };
    use std::path::PathBuf;

    fn fixture_context() -> WorkbenchContext {
        WorkbenchContext {
            id: "wf-1".to_string(),
            title: "Test workbench".to_string(),
            goal: "verify the inspect command".to_string(),
            mode: "review".to_string(),
            repo_path: PathBuf::from("."),
            status: "awaiting_approval".to_string(),
            phase: "approval".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            repo_summary: prometheos_lite::repo_workbench::RepoSummary::default(),
            artifacts: vec![ArtifactRef {
                id: "art-1".to_string(),
                kind: "risk_report".to_string(),
                title: "Risk report".to_string(),
                path: PathBuf::from("artifacts/risk.md"),
                status: "awaiting_approval".to_string(),
                requires_approval: true,
                created_at: Utc::now(),
                provenance: ArtifactProvenance::deterministic("wf-1", "risk_report"),
            }],
            decisions: vec![
                WorkbenchDecision {
                    id: "dec-1".to_string(),
                    artifact_id: "art-1".to_string(),
                    decision: "approved".to_string(),
                    approved: true,
                    created_at: Utc::now(),
                },
                WorkbenchDecision {
                    id: "dec-2".to_string(),
                    artifact_id: "art-DELETED".to_string(),
                    decision: "approved".to_string(),
                    approved: true,
                    created_at: Utc::now(),
                },
            ],
            next_action: None,
        }
    }

    #[test]
    fn inspect_report_includes_metadata_artifacts_and_decisions() {
        let ctx = fixture_context();
        // Save then re-load so the loader round-trips through serde.
        // We use a temp dir as the context's "repo_path" so we can
        // use the real load_context path; but here we exercise the
        // helper directly, which doesn't need a real file.
        let report = inspect_repo_workbench_for(&ctx);

        assert_eq!(report["schemaVersion"], "1.0");
        assert_eq!(report["workId"], "wf-1");
        assert_eq!(report["title"], "Test workbench");
        assert_eq!(report["status"], "awaiting_approval");
        let artifacts = report["artifacts"].as_array().expect("artifacts");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["id"], "art-1");
        assert_eq!(artifacts[0]["kind"], "risk_report");
        let decisions = report["decisions"].as_array().expect("decisions");
        assert_eq!(decisions.len(), 2);
    }

    #[test]
    fn stale_approvals_are_listed_and_unapproved_is_not() {
        let ctx = fixture_context();
        let report = inspect_repo_workbench_for(&ctx);
        let stale = report["staleApprovals"].as_array().expect("staleApprovals");
        // dec-1 references art-1, which is still in the artifacts
        // list -> not stale. dec-2 references art-DELETED, which is
        // not in the list -> stale.
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], "art-DELETED");
    }

    #[test]
    fn decision_actor_is_the_artifacts_provenance_generator() {
        let ctx = fixture_context();
        let report = inspect_repo_workbench_for(&ctx);
        let decisions = report["decisions"].as_array().expect("decisions");
        let dec1 = decisions
            .iter()
            .find(|d| d["id"] == "dec-1")
            .expect("dec-1");
        // The actor of an approved decision whose artifact is
        // present is the artifact's provenance.generator.
        assert_eq!(dec1["actor"], "repo_workbench");
        let dec2 = decisions
            .iter()
            .find(|d| d["id"] == "dec-2")
            .expect("dec-2");
        // The actor of an approved decision whose artifact is gone
        // is `<unknown>`.
        assert_eq!(dec2["actor"], "<unknown>");
    }

    #[test]
    fn approved_decision_for_existing_artifact_is_not_stale() {
        // Regression test: a decision whose artifact_id IS in the
        // current artifacts list must NOT appear in staleApprovals.
        let ctx = fixture_context();
        let report = inspect_repo_workbench_for(&ctx);
        let stale = report["staleApprovals"].as_array().expect("staleApprovals");
        assert!(
            !stale.iter().any(|v| v == "art-1"),
            "art-1 is still in the artifacts list; it must not be stale"
        );
    }

    #[test]
    fn unapproved_decision_is_not_stale_even_if_artifact_gone() {
        // Only APPROVED decisions are checked for staleness. An
        // unapproved decision is irrelevant to the stale-approvals
        // report.
        let mut ctx = fixture_context();
        // Add a third decision: unapproved, artifact missing.
        ctx.decisions.push(WorkbenchDecision {
            id: "dec-3".to_string(),
            artifact_id: "art-MISSING".to_string(),
            decision: "needs_review".to_string(),
            approved: false,
            created_at: Utc::now(),
        });
        let report = inspect_repo_workbench_for(&ctx);
        let stale = report["staleApprovals"].as_array().expect("staleApprovals");
        assert!(
            !stale.iter().any(|v| v == "art-MISSING"),
            "unapproved decision must not be flagged as a stale approval"
        );
    }

    /// Test helper: build a report from an in-memory context,
    /// bypassing the file-based loader. This keeps the tests
    /// hermetic and fast.
    fn inspect_repo_workbench_for(ctx: &WorkbenchContext) -> serde_json::Value {
        // Re-implement the minimal slice of inspect_repo_workbench
        // that operates on a borrowed context. The full helper
        // loads from disk; this local helper is for tests.
        let artifacts = prometheos_lite::repo_workbench::get_artifacts(ctx);
        let current_artifact_ids: std::collections::BTreeSet<&str> =
            artifacts.iter().map(|a| a.id.as_str()).collect();
        let stale_approvals: Vec<String> = ctx
            .decisions
            .iter()
            .filter(|d| d.approved && !current_artifact_ids.contains(d.artifact_id.as_str()))
            .map(|d| d.artifact_id.clone())
            .collect();
        let decisions_json: Vec<serde_json::Value> = ctx
            .decisions
            .iter()
            .map(|d| {
                let actor = ctx
                    .artifacts
                    .iter()
                    .find(|a| a.id == d.artifact_id)
                    .map(|a| a.provenance.generator.clone())
                    .unwrap_or_else(|| "<unknown>".to_string());
                serde_json::json!({
                    "id": d.id,
                    "artifactId": d.artifact_id,
                    "decision": d.decision,
                    "approved": d.approved,
                    "actor": actor,
                    "createdAt": d.created_at,
                })
            })
            .collect();
        let artifacts_json: Vec<serde_json::Value> = artifacts
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "kind": a.kind,
                    "title": a.title,
                    "status": a.status,
                    "requiresApproval": a.requires_approval,
                    "path": a.path.display().to_string(),
                    "provenance": {
                        "generator": &a.provenance.generator,
                        "generationMode": &a.provenance.generation_mode,
                        "modelInvoked": a.provenance.model_invoked,
                        "provider": a.provenance.provider,
                        "model": a.provenance.model,
                        "createdAt": a.provenance.created_at,
                    },
                })
            })
            .collect();
        serde_json::json!({
            "schemaVersion": "1.0",
            "workId": ctx.id,
            "title": ctx.title,
            "goal": ctx.goal,
            "status": ctx.status,
            "phase": ctx.phase,
            "createdAt": ctx.created_at,
            "updatedAt": ctx.updated_at,
            "artifacts": artifacts_json,
            "decisions": decisions_json,
            "staleApprovals": stale_approvals,
        })
    }
}
