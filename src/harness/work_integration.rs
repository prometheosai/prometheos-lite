use crate::{
    harness::{
        completion::CompletionDecision,
        edit_protocol::EditOperation,
        execution_loop::{
            HarnessExecutionRequest, HarnessExecutionResult, HarnessMode, ValidationFailurePolicy,
            execute_harness_task,
        },
    },
    work::{
        artifact::{Artifact, ArtifactKind},
        service::WorkContextService,
        types::{WorkPhase, WorkStatus},
    },
};
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
pub struct HarnessWorkContextService {
    work_context_service: Arc<WorkContextService>,
}
impl HarnessWorkContextService {
    pub fn new(work_context_service: Arc<WorkContextService>) -> Self {
        Self {
            work_context_service,
        }
    }
    pub async fn run_for_context(
        &self,
        context_id: &str,
        repo_root: PathBuf,
        mode: HarnessMode,
        proposed_edits: Vec<EditOperation>,
    ) -> Result<HarnessExecutionResult> {
        let mut ctx = self
            .work_context_service
            .get_context(context_id)?
            .with_context(|| format!("WorkContext not found: {context_id}"))?;
        let req = HarnessExecutionRequest {
            work_context_id: ctx.id.clone(),
            repo_root: repo_root.clone(),
            task: ctx.goal.clone(),
            requirements: ctx.requirements.clone(),
            acceptance_criteria: ctx
                .completion_criteria
                .iter()
                .map(|c| c.description.clone())
                .collect(),
            mode,
            limits: crate::harness::HarnessLimits::default(),
            mentioned_files: vec![],
            mentioned_symbols: vec![],
            proposed_edits: proposed_edits.clone(),
            patch_provider: None,
            provider_context: None,
            progress_callback: None,
            validation_failure_policy: crate::harness::ValidationFailurePolicy::default(),
        };
        self.work_context_service
            .update_phase(&mut ctx, WorkPhase::Execution)?;
        let result = execute_harness_task(req).await?;
        ctx.metadata = serde_json::json!({"harness":serde_json::to_value(&result)?});
        for h in &result.artifacts {
            let context_id = ctx.id.clone();
            let artifact = Artifact::new(
                uuid::Uuid::new_v4().to_string(),
                context_id,
                ArtifactKind::Report,
                format!("harness-{:?}", h.kind),
                serde_json::to_value(h)?,
                "harness".into(),
            );
            self.work_context_service.add_artifact(&mut ctx, artifact)?;
        }
        match &result.completion_decision {
            CompletionDecision::Complete => self
                .work_context_service
                .update_status(&mut ctx, WorkStatus::Completed)?,
            CompletionDecision::NeedsApproval(r) => {
                ctx.blocked_reason = Some(r.clone());
                self.work_context_service
                    .update_status(&mut ctx, WorkStatus::AwaitingApproval)?
            }
            CompletionDecision::NeedsRepair(r) | CompletionDecision::Blocked(r) => self
                .work_context_service
                .set_blocked_reason(&mut ctx, r.clone())?,
        };
        self.work_context_service.update_context(&ctx)?;
        Ok(result)
    }
}
