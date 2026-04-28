//! WorkExecutionService - orchestrates flow execution with WorkContext

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use crate::flow::execution_service::{ExecutionOptions, FlowExecutionService};
use crate::flow::loader::{FlowFile, FlowLoader, JsonLoader, YamlLoader};
use crate::work::{
    domain::WorkDomainProfile,
    types::{AutonomyLevel, ApprovalPolicy, WorkPhase, WorkStatus},
    ArtifactMapper, PhaseController, WorkContext, WorkContextService,
};
use crate::db::repository::DomainProfileOperations;

/// WorkExecutionService - orchestrates flow execution with WorkContext
/// This prevents WorkContextService from becoming a god object
pub struct WorkExecutionService {
    work_context_service: Arc<WorkContextService>,
    flow_execution_service: Arc<FlowExecutionService>,
}

impl WorkExecutionService {
    /// Create a new WorkExecutionService
    pub fn new(
        work_context_service: Arc<WorkContextService>,
        flow_execution_service: Arc<FlowExecutionService>,
    ) -> Self {
        Self {
            work_context_service,
            flow_execution_service,
        }
    }

    /// Resolve flow reference to absolute path
    /// Checks domain-specific flows directory first, then falls back to generic flows
    fn resolve_flow_path(&self, flow_ref: &str, domain: &super::WorkDomain) -> Result<PathBuf> {
        use super::WorkDomain;

        // Determine flows directory based on domain
        let domain_dir = match domain {
            WorkDomain::Software => "software",
            WorkDomain::Business => "business",
            WorkDomain::Marketing => "marketing",
            WorkDomain::Personal => "personal",
            WorkDomain::Research => "research",
            WorkDomain::Creative => "creative",
            WorkDomain::Operations => "operations",
            WorkDomain::General => "general",
            WorkDomain::Custom(name) => name.as_str(),
        };

        // Try domain-specific flows first
        let domain_path = PathBuf::from("flows").join(domain_dir).join(flow_ref);
        if domain_path.exists() {
            return Ok(domain_path);
        }

        // Fall back to generic flows directory
        let generic_path = PathBuf::from("flows").join(flow_ref);
        if generic_path.exists() {
            return Ok(generic_path);
        }

        // Try templates directory
        let template_path = PathBuf::from("templates").join(domain_dir).join(flow_ref);
        if template_path.exists() {
            return Ok(template_path);
        }

        Err(anyhow::anyhow!(
            "Flow file not found: {} (tried flows/{}, flows/{}, templates/{}/{})",
            flow_ref,
            domain_dir,
            flow_ref,
            domain_dir,
            flow_ref
        ))
    }

    /// Load a flow file from path
    fn load_flow_file(&self, path: &PathBuf) -> Result<FlowFile> {
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") || path.extension().and_then(|s| s.to_str()) == Some("yml") {
            let loader = YamlLoader::new();
            loader.load_from_path(path).context("Failed to load YAML flow")
        } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let loader = JsonLoader::new();
            loader.load_from_path(path).context("Failed to load JSON flow")
        } else {
            Err(anyhow::anyhow!("Unsupported flow file extension: {:?}", path.extension()))
        }
    }

    /// Execute a flow within a WorkContext using direct flow file loading
    /// This bypasses intent classification and loads the flow directly from flow_ref
    pub async fn execute_flow_in_context(
        &self,
        context: &mut WorkContext,
        flow_ref: &str,
    ) -> Result<super::Artifact> {
        // Check autonomy level - Chat mode requires human confirmation for all actions
        if context.autonomy_level == AutonomyLevel::Chat {
            self.work_context_service
                .update_status(context, WorkStatus::AwaitingApproval)?;
            anyhow::bail!("Chat mode requires human confirmation before execution");
        }

        // Check if approval is required before execution
        let next_phase = PhaseController::next_phase(context);
        if let Some(phase) = next_phase {
            if PhaseController::requires_approval(context, phase) {
                if context.approval_policy == ApprovalPolicy::ManualAll
                    || context.approval_policy == ApprovalPolicy::RequireForSideEffects
                {
                    self.work_context_service
                        .update_status(context, WorkStatus::AwaitingApproval)?;
                    anyhow::bail!("Approval required before phase transition to {:?}", phase);
                }
            }
        }

        // Load flow file directly from flow_ref (bypass intent classification)
        let flow_path = self.resolve_flow_path(flow_ref, &context.domain)?;
        let flow_file = self.load_flow_file(&flow_path)?;

        // Execute flow directly without intent override
        let options = ExecutionOptions::default();
        let final_output = self
            .flow_execution_service
            .execute_flow_file(&flow_file, &context.goal, options)
            .await?;

        // Map outputs to artifacts
        let artifacts = ArtifactMapper::map_flow_output(
            context.id.clone(),
            flow_ref.to_string(),
            final_output.primary,
            final_output.additional,
        );

        // Add all artifacts to context
        for artifact in artifacts.clone() {
            self.work_context_service.add_artifact(context, artifact)?;
        }

        // Update phase based on flow type using PhaseController
        // Derive phase from current context state and flow metadata
        let next_phase = PhaseController::next_phase(context);
        if let Some(phase) = next_phase {
            self.work_context_service.update_phase(context, phase)?;
        }

        // Review mode requires approval after execution
        if context.autonomy_level == AutonomyLevel::Review {
            self.work_context_service
                .update_status(context, WorkStatus::AwaitingApproval)?;
        }

        // Return the primary artifact
        Ok(artifacts.into_iter().next().unwrap())
    }

    /// Continue a WorkContext
    pub async fn continue_context(&self, context_id: &str) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found"))?;

        // Check if context is blocked
        if context.is_blocked() {
            anyhow::bail!("Context is blocked: {:?}", context.blocked_reason);
        }

        // Check if context is complete
        if context.is_complete() {
            anyhow::bail!("Context is already complete");
        }

        // Determine next action based on phase using PhaseController
        // Load domain profile if available to use playbook flow preferences
        let domain_profile = if let Some(profile_id) = &context.domain_profile_id {
            let db = self.work_context_service.get_db();
            DomainProfileOperations::get_domain_profile(&**db, profile_id)?
        } else {
            None
        };
        
        let next_flow = PhaseController::flow_for_phase(
            context.current_phase,
            domain_profile.as_ref()
        );
        
        if context.current_phase == WorkPhase::Finalization {
            return Ok(context);
        }

        // Execute flow
        self.execute_flow_in_context(&mut context, &next_flow).await?;

        // Update status
        if context.current_phase == WorkPhase::Finalization {
            self.work_context_service
                .update_status(&mut context, WorkStatus::Completed)?;
        } else {
            self.work_context_service
                .update_status(&mut context, WorkStatus::InProgress)?;
        }

        // Save updated context
        self.work_context_service.update_context(&context)?;

        Ok(context)
    }

    /// Create a new WorkContext and execute initial flow
    pub async fn create_and_execute(
        &self,
        user_id: String,
        title: String,
        domain: super::WorkDomain,
        goal: String,
    ) -> Result<WorkContext> {
        // Create context with Review mode to allow initial planning
        let mut context = self
            .work_context_service
            .create_context(user_id, title, domain, goal)?;

        // Override autonomy to Review for initial planning to avoid Chat mode block
        context.autonomy_level = AutonomyLevel::Review;

        // Execute initial planning flow
        self.execute_flow_in_context(&mut context, "planning.flow.yaml")
            .await?;

        // Update phase to AwaitingApproval after planning
        self.work_context_service
            .update_phase(&mut context, WorkPhase::Planning)?;
        self.work_context_service
            .update_status(&mut context, WorkStatus::AwaitingApproval)?;

        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::flow::RuntimeContext;
    use crate::work::types::{WorkDomain, WorkPhase, WorkStatus};

    #[tokio::test]
    async fn test_work_execution_service_creation() {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(
            FlowExecutionService::new(runtime.clone()).unwrap()
        );
        let execution_service = WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        );

        // Verify service creation
        assert_eq!(execution_service.work_context_service, work_context_service);
    }

    #[tokio::test]
    async fn test_continue_blocked_context() {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(
            FlowExecutionService::new(runtime.clone()).unwrap()
        );
        let execution_service = WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        );

        let mut context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        work_context_service
            .set_blocked_reason(&mut context, "Waiting for approval".to_string())
            .unwrap();

        let result = execution_service.continue_context(&context.id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn test_continue_complete_context() {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(
            FlowExecutionService::new(runtime.clone()).unwrap()
        );
        let execution_service = WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        );

        let mut context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        work_context_service
            .update_status(&mut context, WorkStatus::Completed)
            .unwrap();

        let result = execution_service.continue_context(&context.id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("complete"));
    }

    #[tokio::test]
    async fn test_work_context_lifecycle() {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));

        let mut context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        // Test initial state
        assert_eq!(context.status, WorkStatus::Draft);
        assert_eq!(context.current_phase, WorkPhase::Intake);

        // Test phase transition
        work_context_service
            .update_phase(&mut context, WorkPhase::Planning)
            .unwrap();
        assert_eq!(context.current_phase, WorkPhase::Planning);

        // Test status transition
        work_context_service
            .update_status(&mut context, WorkStatus::InProgress)
            .unwrap();
        assert_eq!(context.status, WorkStatus::InProgress);

        // Test completion
        work_context_service
            .update_status(&mut context, WorkStatus::Completed)
            .unwrap();
        assert_eq!(context.status, WorkStatus::Completed);
    }
}
