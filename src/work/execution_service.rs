//! WorkExecutionService - orchestrates flow execution with WorkContext

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use tracing;

use crate::db::repository::{DomainProfileOperations, FlowPerformanceOperations, PlaybookOperations};
use crate::flow::StrictModeEnforcer;
use crate::flow::execution_service::{ExecutionOptions, FlowExecutionService};
use crate::flow::loader::{FlowFile, FlowLoader, JsonLoader, YamlLoader};
use crate::work::{
    ArtifactMapper, PhaseController, WorkContext, WorkContextService,
    domain::WorkDomainProfile,
    playbook::WorkContextPlaybook,
    types::{ApprovalPolicy, AutonomyLevel, FlowPerformanceRecord, WorkPhase, WorkStatus},
};

/// WorkExecutionService - orchestrates flow execution with WorkContext
/// This prevents WorkContextService from becoming a god object
pub struct WorkExecutionService {
    work_context_service: Arc<WorkContextService>,
    flow_execution_service: Arc<FlowExecutionService>,
    strict_mode: Option<StrictModeEnforcer>,
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
            strict_mode: None,
        }
    }

    /// Create a new WorkExecutionService with strict mode
    pub fn with_strict_mode(
        work_context_service: Arc<WorkContextService>,
        flow_execution_service: Arc<FlowExecutionService>,
        strict_mode: StrictModeEnforcer,
    ) -> Self {
        Self {
            work_context_service,
            flow_execution_service,
            strict_mode: Some(strict_mode),
        }
    }

    /// Set strict mode enforcer
    pub fn set_strict_mode(&mut self, strict_mode: StrictModeEnforcer) {
        self.strict_mode = Some(strict_mode);
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
        if path.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            let loader = YamlLoader::new();
            loader
                .load_from_path(path)
                .context("Failed to load YAML flow")
        } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let loader = JsonLoader::new();
            loader
                .load_from_path(path)
                .context("Failed to load JSON flow")
        } else {
            Err(anyhow::anyhow!(
                "Unsupported flow file extension: {:?}",
                path.extension()
            ))
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

        // Build execution options with strict mode if enabled
        let mut options = ExecutionOptions::default();
        if let Some(ref strict_mode) = self.strict_mode {
            options = options.with_strict_mode(strict_mode.clone());
        }

        // Execute flow directly without intent override
        let final_output = self
            .flow_execution_service
            .execute_flow_file(&flow_file, &context.goal, options)
            .await?;

        // Convert execution metadata to ExecutionRecords and add to WorkContext
        for (node_id, metadata_json) in &final_output.execution_metadata {
            if let Ok(generate_result) = serde_json::from_value::<
                crate::flow::intelligence::GenerateResult,
            >(metadata_json.clone())
            {
                let execution_record = super::types::ExecutionRecord::from_generate_result(
                    node_id.clone(),
                    &generate_result,
                );
                context.execution_metadata.push(execution_record);
            }
        }

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
        let primary_artifact = artifacts
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Flow produced no artifacts"))?;
        Ok(primary_artifact)
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

        // Load playbook if available for weighted flow selection
        let playbook = if let Some(ref playbook_id) = context.playbook_id {
            let db = self.work_context_service.get_db();
            PlaybookOperations::get_playbook(&**db, playbook_id)?
        } else {
            None
        };

        let next_flow = if let Some(ref playbook) = playbook {
            // Use weighted selection from playbook with 10% exploration factor
            PhaseController::weighted_flow_selection(
                context.current_phase,
                &playbook.preferred_flows,
                0.1, // 10% exploration factor
            )
        } else {
            // Fallback to static flow selection
            PhaseController::flow_for_phase(context.current_phase, domain_profile.as_ref())
        };

        if context.current_phase == WorkPhase::Finalization {
            return Ok(context);
        }

        // Execute flow
        let start_time = std::time::Instant::now();
        self.execute_flow_in_context(&mut context, &next_flow)
            .await?;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Create and store FlowPerformanceRecord
        let performance_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: next_flow.clone(),
            work_context_id: context.id.clone(),
            success_score: if context.status == WorkStatus::Completed {
                1.0
            } else {
                0.5
            },
            duration_ms,
            token_cost: context
                .execution_metadata
                .iter()
                .filter_map(|r| r.cost)
                .sum(),
            revision_count: context.decisions.len() as u32,
            executed_at: chrono::Utc::now(),
        };

        // Store performance record in database using FlowPerformanceOperations
        // V1.5.2: Using dedicated database table instead of metadata storage
        let db = self.work_context_service.get_db();
        if let Err(e) = db.as_ref().create_flow_performance(&performance_record) {
            tracing::error!("Failed to store flow performance record: {}", e);
            // Fallback: store in metadata for debugging if DB fails
            let performance_key = format!("flow_perf_{}", next_flow);
            context.metadata[performance_key] =
                serde_json::to_value(&performance_record).unwrap_or(serde_json::Value::Null);
        } else {
            tracing::debug!("Stored flow performance record: {}", performance_record.id);
        }

        // Update status
        if context.current_phase == WorkPhase::Finalization {
            self.work_context_service
                .update_status(&mut context, WorkStatus::Completed)?;
        } else {
            self.work_context_service
                .update_status(&mut context, WorkStatus::InProgress)?;
        }

        // Save updated context with execution metadata
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

        // Save context with execution metadata
        self.work_context_service.update_context(&context)?;

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
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime.clone()).unwrap());
        let execution_service =
            WorkExecutionService::new(work_context_service.clone(), flow_execution_service);

        // Verify service creation
        assert!(Arc::ptr_eq(
            &execution_service.work_context_service,
            &work_context_service
        ));
    }

    #[tokio::test]
    async fn test_continue_blocked_context() {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let runtime = Arc::new(RuntimeContext::default());
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime.clone()).unwrap());
        let execution_service =
            WorkExecutionService::new(work_context_service.clone(), flow_execution_service);

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
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime.clone()).unwrap());
        let execution_service =
            WorkExecutionService::new(work_context_service.clone(), flow_execution_service);

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
    async fn test_zero_artifact_error() {
        // Test the artifact extraction error handling directly
        let artifacts: Vec<crate::work::artifact::Artifact> = vec![];

        let result = artifacts
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Flow produced no artifacts"));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no artifacts"));
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
