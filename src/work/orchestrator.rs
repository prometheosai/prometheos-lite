//! WorkOrchestrator - Central service for persistent work execution
//!
//! This module provides the WorkOrchestrator, which owns the high-level
//! execution loop for persistent work contexts with hard stop contracts.

use anyhow::Result;
use std::sync::Arc;

use super::execution_service::WorkExecutionService;
use super::service::WorkContextService;
use super::types::{AutonomyLevel, WorkContext, WorkPhase, WorkStatus};
use crate::flow::execution_service::FlowExecutionService;
use crate::intent::{Intent, IntentClassifier};

/// ExecutionLimits - hard stop contracts for autonomous execution
#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    pub max_iterations: u32,
    pub max_runtime_ms: u64,
    pub max_tool_calls: u32,
    pub max_cost: f64,
    pub approval_required_for_side_effects: bool,
    pub completion_criteria: Vec<String>,
    pub failure_threshold: f32,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_runtime_ms: 300_000, // 5 minutes
            max_tool_calls: 50,
            max_cost: 1.0, // $1.00
            approval_required_for_side_effects: true,
            completion_criteria: Vec::new(),
            failure_threshold: 0.3,
        }
    }
}

impl ExecutionLimits {
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn with_max_runtime_ms(mut self, max: u64) -> Self {
        self.max_runtime_ms = max;
        self
    }

    pub fn with_max_tool_calls(mut self, max: u32) -> Self {
        self.max_tool_calls = max;
        self
    }

    pub fn with_max_cost(mut self, max: f64) -> Self {
        self.max_cost = max;
        self
    }
}

/// WorkOrchestrator - Central service owning the high-level execution loop
pub struct WorkOrchestrator {
    work_context_service: Arc<WorkContextService>,
    playbook_resolver: Arc<super::playbook_resolver::PlaybookResolver>,
    work_execution_service: Arc<WorkExecutionService>,
    intent_classifier: IntentClassifier,
}

// Ensure WorkOrchestrator is Send + Sync for use in async handlers
unsafe impl Send for WorkOrchestrator {}
unsafe impl Sync for WorkOrchestrator {}

impl WorkOrchestrator {
    pub fn new(
        work_context_service: Arc<WorkContextService>,
        playbook_resolver: Arc<super::playbook_resolver::PlaybookResolver>,
        work_execution_service: Arc<WorkExecutionService>,
        intent_classifier: IntentClassifier,
    ) -> Self {
        Self {
            work_context_service,
            playbook_resolver,
            work_execution_service,
            intent_classifier,
        }
    }

    /// Submit a user intent to create or attach to a WorkContext
    pub async fn submit_user_intent(
        &self,
        user_id: String,
        message: String,
        conversation_id: Option<String>,
    ) -> Result<WorkContext> {
        // 1. Classify intent
        let classification = self
            .intent_classifier
            .classify_with_override(&message, None)
            .await?;

        // 2. Route to context (create new or attach to existing)
        let mut context = match self.work_context_service.route_context(
            &user_id,
            conversation_id.as_deref(),
            None,
        )? {
            Some(ctx) => ctx,
            None => {
                // Create new context
                let domain = self.infer_domain_from_intent(&classification.intent);
                let mut context = self.work_context_service.create_context(
                    user_id.clone(),
                    self.generate_title(&message),
                    domain,
                    message.clone(),
                )?;

                // Set autonomy level based on intent type
                match classification.intent {
                    crate::intent::Intent::CodingTask | crate::intent::Intent::ProjectAction => {
                        context.autonomy_level = AutonomyLevel::Review;
                    }
                    _ => {
                        context.autonomy_level = AutonomyLevel::Chat;
                    }
                }

                context
            }
        };

        // 3. Attach to conversation if provided
        if let Some(ref conv_id) = conversation_id {
            context.conversation_id = Some(conv_id.clone());
            self.work_context_service
                .set_active_context_for_conversation(conv_id, &context.id)?;
        }

        // 4. Select playbook
        if let Some(playbook) = self.playbook_resolver.resolve_playbook(&context)? {
            // Apply playbook settings
            context.domain_profile_id = Some(playbook.domain_profile_id.clone());
            context.approval_policy = playbook.default_approval_policy;
            self.work_context_service.update_context(&context)?;

            // Update playbook usage
            self.playbook_resolver.update_playbook_usage(&playbook.id)?;
        }

        // 5. Execute flow based on autonomy level
        // Chat mode: create + set AwaitingApproval (no execution)
        // Review mode: execute planning → Await approval
        // Autonomous mode: execute immediately
        if context.autonomy_level == AutonomyLevel::Chat {
            self.work_context_service
                .update_status(&mut context, WorkStatus::AwaitingApproval)?;
            self.work_context_service.update_context(&context)?;
        } else if context.autonomy_level == AutonomyLevel::Review {
            // Review mode: execute planning flow
            self.work_execution_service
                .continue_context(&context.id)
                .await?;

            // Reload context to get updated state
            context = self
                .work_context_service
                .get_context(&context.id)?
                .ok_or_else(|| anyhow::anyhow!("Context not found after execution: {}", context.id))?;
        } else {
            // Autonomous mode: execute immediately
            self.work_execution_service
                .continue_context(&context.id)
                .await?;

            // Reload context to get updated state
            context = self
                .work_context_service
                .get_context(&context.id)?
                .ok_or_else(|| anyhow::anyhow!("Context not found after execution: {}", context.id))?;
        }

        Ok(context)
    }

    /// Continue a blocked context
    pub async fn continue_context(&self, context_id: String) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        // Clear blocked reason if set, then execute
        if context.is_blocked() {
            self.work_context_service.clear_blocked_reason(&mut context)?;
        }

        let context = self
            .work_execution_service
            .continue_context(&context_id)
            .await?;

        Ok(context)
    }

    /// Run context until blocked or complete, respecting limits
    pub async fn run_until_blocked_or_complete(
        &self,
        context_id: String,
        limits: ExecutionLimits,
    ) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        let mut iterations = 0;
        let start = std::time::Instant::now();

        loop {
            // Check limits
            if iterations >= limits.max_iterations {
                self.work_context_service.set_blocked_reason(
                    &mut context,
                    "Max iterations reached".to_string(),
                )?;
                break;
            }

            if start.elapsed().as_millis() as u64 >= limits.max_runtime_ms {
                self.work_context_service.set_blocked_reason(
                    &mut context,
                    "Max runtime exceeded".to_string(),
                )?;
                break;
            }

            // Check completion - empty criteria should NOT mean complete
            if context.is_complete() || (!context.completion_criteria.is_empty() && context.is_completion_satisfied()) {
                context.status = WorkStatus::Completed;
                self.work_context_service.update_context(&context)?;
                break;
            }

            // Check blocked
            if context.is_blocked() {
                break;
            }

            // Execute next step using WorkExecutionService
            context = self.work_execution_service.continue_context(&context.id).await?;

            iterations += 1;
        }

        Ok(context)
    }

    /// Route to the appropriate context based on priority
    pub fn route_to_context(
        &self,
        user_id: &str,
        conversation_id: Option<&str>,
        explicit_context_id: Option<&str>,
    ) -> Result<Option<WorkContext>> {
        self.work_context_service
            .route_context(user_id, conversation_id, explicit_context_id)
    }

    fn infer_domain_from_intent(&self, intent: &Intent) -> super::types::WorkDomain {
        match intent {
            Intent::CodingTask => super::types::WorkDomain::Software,
            Intent::FileEdit => super::types::WorkDomain::Software,
            Intent::ProjectAction => super::types::WorkDomain::Software,
            _ => super::types::WorkDomain::General,
        }
    }

    fn generate_title(&self, message: &str) -> String {
        // Simple title generation - take first 50 chars
        let title = message.chars().take(50).collect::<String>();
        if message.len() > 50 {
            format!("{}...", title)
        } else {
            title
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_limits_default() {
        let limits = ExecutionLimits::default();
        assert_eq!(limits.max_iterations, 10);
        assert_eq!(limits.max_runtime_ms, 300_000);
        assert_eq!(limits.max_tool_calls, 50);
        assert_eq!(limits.max_cost, 1.0);
        assert!(limits.approval_required_for_side_effects);
    }

    #[test]
    fn test_execution_limits_builder() {
        let limits = ExecutionLimits::default()
            .with_max_iterations(20)
            .with_max_runtime_ms(600_000);

        assert_eq!(limits.max_iterations, 20);
        assert_eq!(limits.max_runtime_ms, 600_000);
    }
}
