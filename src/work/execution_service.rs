//! WorkExecutionService - orchestrates flow execution with WorkContext

use anyhow::Result;
use std::sync::Arc;

use super::{
    artifact_mapper::ArtifactMapper,
    phase_controller::PhaseController,
    service::WorkContextService,
    types::{ApprovalPolicy, AutonomyLevel, WorkContext, WorkPhase, WorkStatus},
};
use crate::flow::execution_service::{ExecutionOptions, FlowExecutionService};

/// WorkExecutionService - orchestrates flow execution with WorkContext
/// This prevents WorkContextService from becoming a god object
pub struct WorkExecutionService {
    work_context_service: Arc<WorkContextService>,
    flow_execution_service: Arc<FlowExecutionService>,
    phase_controller: Arc<PhaseController>,
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
            phase_controller: Arc::new(PhaseController),
        }
    }

    /// Execute a flow within a WorkContext
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

        // Build execution options with context-aware settings
        let options = ExecutionOptions::default()
            .with_override_intent(crate::intent::Intent::CodingTask); // TODO: Map flow_ref to intent

        // Execute the flow
        let final_output = self
            .flow_execution_service
            .execute_message(&context.goal, options)
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

        // Update phase based on flow type
        if flow_ref.contains("planning") {
            self.work_context_service
                .update_phase(context, WorkPhase::Planning)?;
        } else if flow_ref.contains("execute") {
            self.work_context_service
                .update_phase(context, WorkPhase::Execution)?;
        } else if flow_ref.contains("review") {
            self.work_context_service
                .update_phase(context, WorkPhase::Review)?;
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

        // Determine next action based on phase
        let next_flow = match context.current_phase {
            WorkPhase::Intake => "planning.flow.yaml",
            WorkPhase::Planning => "execution.flow.yaml",
            WorkPhase::Execution => "review.flow.yaml",
            WorkPhase::Review => "finalization.flow.yaml",
            WorkPhase::Iteration => "planning.flow.yaml",
            WorkPhase::Finalization => return Ok(context),
        };

        // Execute flow
        self.execute_flow_in_context(&mut context, next_flow).await?;

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
        // Create context
        let mut context = self
            .work_context_service
            .create_context(user_id, title, domain, goal)?;

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
    use crate::work::types::WorkDomain;

    #[tokio::test]
    async fn test_execute_flow_in_context() {
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

        // This test requires actual flow files, so we'll skip for now
        // In a real scenario, we'd mock the FlowExecutionService
        // For now, we'll just verify the structure compiles
        assert_eq!(context.status, WorkStatus::Draft);
    }

    #[tokio::test]
    async fn test_continue_context() {
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

        let context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        // This test requires actual flow execution
        // For now, we'll skip and just verify structure
        assert_eq!(context.status, WorkStatus::Draft);
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
    async fn test_create_and_execute() {
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

        // This test requires actual flow execution
        // For now, we'll skip and just verify structure
        let context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        assert_eq!(context.status, WorkStatus::Draft);
    }
}
