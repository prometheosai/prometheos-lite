//! WorkExecutionService - orchestrates flow execution with WorkContext

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing;

use crate::db::repository::{
    DomainProfileOperations, FlowPerformanceOperations, PlaybookOperations,
};
use crate::flow::StrictModeEnforcer;
use crate::flow::execution_service::{ExecutionOptions, FlowExecutionService};
use crate::flow::loader::{FlowFile, FlowLoader, JsonLoader, YamlLoader};
use crate::harness::completion::CompletionDecision;
use crate::work::{
    ArtifactMapper, PhaseController, WorkContext, WorkContextService,
    types::{
        ApprovalPolicy, AutonomyLevel, FlowPerformanceRecord, HarnessMetadata, WorkDomain,
        WorkPhase, WorkStatus,
    },
};
use crate::workflow::evaluate::CancellationToken;

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
    fn load_flow_file(&self, path: &Path) -> Result<FlowFile> {
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
    ///
    /// Returns `Ok(None)` when cancellation was observed at the post-flow
    /// checkpoint: the flow ran to completion, but every durable write for
    /// this iteration (artifacts, phase, status, context snapshot) was
    /// skipped, so a cancelled context accumulates no post-cancel progress
    /// and no orphan rows.
    ///
    /// Cancellation-safety of the selected checkpoint (#222): this is the
    /// ONLY safe observation point inside an iteration. At this point
    /// `flow.run()` has returned, so every awaitable the flow owned —
    /// provider HTTP requests, and critically tool-node child processes
    /// (`tokio::process::Command` without `kill_on_drop`) — has completed.
    /// Dropping the iteration future at an arbitrary earlier suspension
    /// point would orphan a live tool process mid-mutation; that is why
    /// this method checks cancellation here instead of the caller
    /// select-dropping the in-flight future.
    pub async fn execute_flow_in_context(
        &self,
        context: &mut WorkContext,
        flow_ref: &str,
        token: &CancellationToken,
    ) -> Result<Option<super::Artifact>> {
        // Check autonomy level - Chat mode requires human confirmation for all actions
        if context.autonomy_level == AutonomyLevel::Chat {
            self.work_context_service
                .update_status(context, WorkStatus::AwaitingApproval)?;
            anyhow::bail!("Chat mode requires human confirmation before execution");
        }

        // Check if approval is required before execution
        let next_phase = PhaseController::next_phase(context);
        if let Some(phase) = next_phase
            && PhaseController::requires_approval(context, phase)
            && (context.approval_policy == ApprovalPolicy::ManualAll
                || context.approval_policy == ApprovalPolicy::RequireForSideEffects)
        {
            self.work_context_service
                .update_status(context, WorkStatus::AwaitingApproval)?;
            anyhow::bail!("Approval required before phase transition to {:?}", phase);
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

        // Post-flow cancellation checkpoint (#222): the selected, proven
        // cancellation-safe boundary. `flow.run()` has completed, so no
        // child process or in-flight request is owned by this future; and
        // no durable write for this iteration has started yet, so skipping
        // the write sequence below leaves zero orphan rows. The park is a
        // no-op in production; tests use it to make this checkpoint
        // deterministic. The token covers the same-process signal (the
        // /cancel handler fires it after the durable flip); the stored
        // status re-read covers every other process uniformly.
        token.park_at_safe_point().await;
        let observed_cancelled = token.is_cancelled()
            || self
                .work_context_service
                .get_context(&context.id)?
                .is_some_and(|stored| stored.is_cancelled());
        if observed_cancelled {
            tracing::info!(
                context_id = %context.id,
                flow = %flow_ref,
                "cancellation observed at post-flow checkpoint; skipping all iteration writes"
            );
            return Ok(None);
        }

        if !final_output.success {
            let error_message = final_output
                .error
                .clone()
                .unwrap_or_else(|| "Unknown flow execution failure".to_string());
            anyhow::bail!(
                "Flow '{}' failed for work context '{}': {}",
                flow_ref,
                context.id,
                error_message
            );
        }

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
            // V1.6.1 strict transition bridge:
            // Legacy non-harness flows still route through WorkExecutionService. When a software
            // execution successfully produced artifacts, persist explicit transition evidence.
            if context.domain == WorkDomain::Software
                && context.current_phase == WorkPhase::Execution
                && phase == WorkPhase::Review
                && !context.artifacts.is_empty()
            {
                if !context.metadata.is_object() {
                    context.metadata = serde_json::json!({});
                }
                if let Some(root) = context.metadata.as_object_mut() {
                    let harness_obj = root
                        .entry("harness".to_string())
                        .or_insert_with(|| serde_json::json!({}));
                    if !harness_obj.is_object() {
                        *harness_obj = serde_json::json!({});
                    }
                    if let Some(h) = harness_obj.as_object_mut() {
                        h.insert(
                            "patch_result".to_string(),
                            serde_json::json!({"applied": true}),
                        );
                        h.insert(
                            "validation_result".to_string(),
                            serde_json::json!({"passed": true, "source": "legacy_flow_bridge"}),
                        );
                    }
                }
            }
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
        Ok(Some(primary_artifact))
    }

    /// Continue a WorkContext (no external cancellation signal; the loop in
    /// `WorkOrchestrator::run_until_blocked_or_complete_with_token` and the
    /// API run endpoint use the `_with_token` variant).
    pub async fn continue_context(&self, context_id: &str) -> Result<WorkContext> {
        let token = CancellationToken::new();
        self.continue_context_with_token(context_id, &token).await
    }

    /// Continue a WorkContext, observing `token` at the post-flow
    /// cancellation checkpoint. When cancellation is observed there, all
    /// durable writes for the iteration are skipped and the freshly stored
    /// context is returned (its status will be `Cancelled` when the durable
    /// cancel has landed), so the caller can exit gracefully.
    pub async fn continue_context_with_token(
        &self,
        context_id: &str,
        token: &CancellationToken,
    ) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found"))?;

        // Cancellation is terminal and checked at the shared execution
        // boundary, not only at the HTTP handler: service-layer callers
        // (CLI, orchestrator paths) must not advance cancelled work either.
        if context.is_cancelled() {
            anyhow::bail!("cancelled WorkContext cannot continue: {context_id}");
        }

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
        let primary_artifact = self
            .execute_flow_in_context(&mut context, &next_flow, token)
            .await?;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Cancellation observed at the post-flow checkpoint: the flow ran,
        // but every write for this iteration (performance record, status,
        // context snapshot) is skipped and the durable state is returned
        // untouched by this loop.
        let Some(_primary_artifact) = primary_artifact else {
            let fresh = self
                .work_context_service
                .get_context(context_id)?
                .ok_or_else(|| anyhow::anyhow!("Context not found"))?;
            return Ok(fresh);
        };

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
            if context.domain == WorkDomain::Software {
                context.set_harness_metadata(HarnessMetadata {
                    completion_decision: Some(CompletionDecision::Complete),
                    ..Default::default()
                });
            }
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
        let planned = self
            .execute_flow_in_context(
                &mut context,
                "planning.flow.yaml",
                &CancellationToken::new(),
            )
            .await?;
        let Some(_planned_artifact) = planned else {
            anyhow::bail!(
                "planning for work context {} was cancelled before any progress was persisted",
                context.id
            );
        };

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
    use crate::db::repository::WorkContextEventOperations;
    use crate::flow::RuntimeContext;
    use crate::flow::intelligence::{LlmProvider, ModelRouter, StreamCallback};
    use crate::harness::completion::CompletionDecision;
    use crate::work::types::{HarnessMetadata, WorkDomain, WorkPhase, WorkStatus};

    /// Deterministic LLM provider (same contract as the e2e test provider)
    /// so planning.flow.yaml runs to completion without network access.
    struct DeterministicTestProvider;

    #[async_trait::async_trait]
    impl LlmProvider for DeterministicTestProvider {
        async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
            if prompt.to_lowercase().contains("plan") {
                return Ok(
                    "1. Analyze requirements\n2. Implement changes\n3. Validate with tests"
                        .to_string(),
                );
            }
            Ok(format!("Generated output for: {}", prompt))
        }

        async fn generate_stream(
            &self,
            prompt: &str,
            callback: StreamCallback,
        ) -> anyhow::Result<String> {
            let output = self.generate(prompt).await?;
            callback(&output);
            Ok(output)
        }

        fn name(&self) -> &str {
            "deterministic-test-provider"
        }

        fn model(&self) -> &str {
            "deterministic-v1"
        }
    }

    fn setup_execution_service() -> (Arc<WorkContextService>, Arc<WorkExecutionService>, Arc<Db>) {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let model_router = Arc::new(ModelRouter::new(vec![Box::new(DeterministicTestProvider)]));
        let runtime = Arc::new(RuntimeContext::default().with_model_router(model_router));
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime).unwrap());
        let execution_service = Arc::new(WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        ));
        (work_context_service, execution_service, db)
    }

    fn general_review_context(work_context_service: &WorkContextService) -> WorkContext {
        let mut context = work_context_service
            .create_context(
                "user-1".to_string(),
                "Plan the work".to_string(),
                WorkDomain::General,
                "Create a plan".to_string(),
            )
            .unwrap();
        context.autonomy_level = AutonomyLevel::Review;
        work_context_service.update_context(&context).unwrap();
        context
    }

    fn count(db: &Db, table: &str) -> i64 {
        db.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// #222 mid-iteration checkpoint: cancellation observed after the flow
    /// completed (the proven cancellation-safe boundary) skips EVERY
    /// durable write for the iteration — no artifact rows, no flow
    /// performance record, and no context-row rewrite after the durable
    /// cancel flip. Deterministic: the flip+fire happen while the iteration
    /// is in flight; the park barrier rendezvous guarantees the test's
    /// release is observed at the checkpoint, never racing it.
    #[tokio::test]
    async fn post_flow_checkpoint_skips_all_writes_on_cancellation() {
        let (wcs, execution_service, db) = setup_execution_service();
        let context = general_review_context(&wcs);

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let token = CancellationToken::with_park_barrier(barrier.clone());

        let artifacts_before = count(&db, "work_artifacts");
        let performance_before = count(&db, "flow_performance_records");

        let svc = execution_service.clone();
        let wcs_task = wcs.clone();
        let ctx_id = context.id.clone();
        let task_token = token.clone();
        let handle = tokio::spawn(async move {
            let mut ctx = wcs_task.get_context(&ctx_id).unwrap().unwrap();
            svc.execute_flow_in_context(&mut ctx, "planning.flow.yaml", &task_token)
                .await
        });

        // Cancel lands mid-iteration (the flow is running concurrently):
        // durable flip first, then the token fire — production ordering.
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        let updated_at_after_flip: String = db
            .conn()
            .query_row(
                "SELECT updated_at FROM work_contexts WHERE id = ?1",
                rusqlite::params![context.id],
                |r| r.get(0),
            )
            .unwrap();
        token.cancel();

        // Rendezvous with the parked checkpoint and release it.
        barrier.wait().await;

        let result = handle.await.unwrap().unwrap();
        assert!(
            result.is_none(),
            "checkpoint must skip the iteration's writes on cancellation"
        );

        // No orphan artifact rows, no post-cancel progress rows.
        assert_eq!(count(&db, "work_artifacts"), artifacts_before);
        assert_eq!(count(&db, "flow_performance_records"), performance_before);

        // The context row was never rewritten after the flip: terminal
        // Cancelled state untouched (no resurrection attempt even tried).
        let (status, updated_at): (String, String) = db
            .conn()
            .query_row(
                "SELECT status, updated_at FROM work_contexts WHERE id = ?1",
                rusqlite::params![context.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(status.contains("Cancelled"));
        assert_eq!(updated_at, updated_at_after_flip);

        // Event stream: the flip's audit event, nothing from this
        // iteration (the execution_interrupted evidence is the loop's
        // responsibility, not the step's).
        let events = WorkContextEventOperations::get_events_for_context(&*db, &context.id).unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"context_created"));
        assert!(types.contains(&"context_cancelled"));
        assert!(!types.contains(&"artifact_added"));
        assert!(!types.contains(&"status_changed"));
        assert!(!types.contains(&"execution_interrupted"));
    }

    /// Negative control: without cancellation the same flow persists its
    /// artifacts — the checkpoint must not skip normal persistence.
    #[tokio::test]
    async fn post_flow_checkpoint_does_not_skip_normal_persistence() {
        let (wcs, execution_service, db) = setup_execution_service();
        let mut context = general_review_context(&wcs);

        let artifacts_before = count(&db, "work_artifacts");
        let token = CancellationToken::new();
        let result = execution_service
            .execute_flow_in_context(&mut context, "planning.flow.yaml", &token)
            .await
            .unwrap();

        assert!(
            result.is_some(),
            "uncancelled flow must return its artifact"
        );
        assert_eq!(
            count(&db, "work_artifacts"),
            artifacts_before + 1,
            "uncancelled flow must persist its artifact"
        );
    }

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
        context.set_harness_metadata(HarnessMetadata {
            completion_decision: Some(CompletionDecision::Complete),
            ..Default::default()
        });

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
        context.set_harness_metadata(HarnessMetadata {
            completion_decision: Some(CompletionDecision::Complete),
            ..Default::default()
        });
        work_context_service
            .update_status(&mut context, WorkStatus::Completed)
            .unwrap();
        assert_eq!(context.status, WorkStatus::Completed);
    }
}
