//! WorkContext CLI commands

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

use prometheos_lite::db::Db;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::work::{
    types::{WorkDomain, WorkStatus},
    WorkContextService, WorkExecutionService,
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
    /// Continue a WorkContext
    Continue {
        /// WorkContext ID
        id: String,
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

        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime)?);
        let work_execution_service = Arc::new(WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        ));

        match self.command {
            WorkSubcommand::Create { title, domain, goal } => {
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
                println!("  Completion Criteria: {}", context.completion_criteria.len());
                
                if let Some(due) = &context.due_at {
                    println!("  Due At: {}", due);
                }
                if let Some(blocked) = &context.blocked_reason {
                    println!("  Blocked: {}", blocked);
                }
            }
            WorkSubcommand::Continue { id } => {
                let context = work_execution_service.continue_context(&id).await?;
                
                println!("Continued WorkContext:");
                println!("  ID: {}", context.id);
                println!("  Status: {:?}", context.status);
                println!("  Phase: {:?}", context.current_phase);
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
