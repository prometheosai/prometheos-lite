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
    artifact::Artifact,
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
}

impl WorkCommand {
    pub async fn execute(self) -> Result<()> {
        let db_path = "prometheos.db";
        let db = Arc::new(Db::new(db_path)?);
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));

        // Ensure domain templates are installed
        let template_loader = TemplateLoader::default()?;
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
        }

        Ok(())
    }
}
