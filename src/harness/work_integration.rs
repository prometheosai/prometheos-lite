use crate::{
    harness::{
        completion::CompletionDecision,
        edit_protocol::EditOperation,
        execution_loop::{
            HarnessExecutionRequest, HarnessExecutionResult, ValidationFailurePolicy,
            execute_harness_task,
        },
        mode_policy::HarnessMode,
    },
    work::{
        artifact::{Artifact, ArtifactKind},
        service::WorkContextService,
        types::{WorkPhase, WorkStatus},
    },
};
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};

/// P0-FIX: Extract mentioned files and symbols from task text
///
/// Uses simple regex patterns to identify likely file paths and code symbols
/// mentioned in the task description and requirements.
pub fn extract_task_hints(task: &str, requirements: &[String]) -> (Vec<PathBuf>, Vec<String>) {
    use regex::Regex;

    let mut files = Vec::new();
    let mut symbols = Vec::new();

    // Combine task and requirements for analysis
    let full_text = format!("{} {}", task, requirements.join(" "));

    // Pattern: file paths like src/harness/execution_loop.rs or execution_loop.rs
    let file_pattern = Regex::new(r"(?:src/|lib/|tests?/|bin/)?[a-zA-Z0-9_/-]+\.(rs|ts|js|py|go|java|cpp|c|h|toml|json|yaml|yml)").unwrap();
    for cap in file_pattern.captures_iter(&full_text) {
        let path = PathBuf::from(&cap[0]);
        if !files.contains(&path) {
            files.push(path);
        }
    }

    // Pattern: quoted file names like `execution_loop.rs`
    let backtick_pattern = Regex::new(r"`([^`]+\.(rs|ts|js|py|go|java|cpp|c|h))`").unwrap();
    for cap in backtick_pattern.captures_iter(&full_text) {
        let path = PathBuf::from(&cap[1]);
        if !files.contains(&path) {
            files.push(path);
        }
    }

    // Pattern: function names like `execute_harness_task` or execute_harness_task()
    let fn_pattern = Regex::new(r"`?([a-z_][a-z0-9_]*(?:_[a-z0-9_]+)*)\(?`?").unwrap();
    for cap in fn_pattern.captures_iter(&full_text) {
        let symbol = &cap[1];
        // Filter out common words
        if symbol.len() > 3
            && !matches!(symbol, "the" | "and" | "for" | "with" | "from" | "into" | "this" | "that")
        {
            let symbol_str = symbol.to_string();
            if !symbols.contains(&symbol_str) {
                symbols.push(symbol_str);
            }
        }
    }

    // Pattern: CamelCase type names
    let type_pattern = Regex::new(r"\b([A-Z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*)\b").unwrap();
    for cap in type_pattern.captures_iter(&full_text) {
        let symbol_str = cap[1].to_string();
        if !symbols.contains(&symbol_str) {
            symbols.push(symbol_str);
        }
    }

    (files, symbols)
}

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

        // P0-FIX: Extract mentioned files and symbols from task
        let (mentioned_files, mentioned_symbols) =
            extract_task_hints(&ctx.goal, &ctx.requirements);

        tracing::info!(
            "P0: Extracted {} files and {} symbols from task",
            mentioned_files.len(),
            mentioned_symbols.len()
        );

        // P0-FIX: Build request with config provider auto-resolution
        let mut req = HarnessExecutionRequest {
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
            mentioned_files: mentioned_files.clone(),
            mentioned_symbols: mentioned_symbols.clone(),
            proposed_edits: proposed_edits.clone(),
            patch_provider: None,
            provider_context: None, // Will be set after repo analysis in execution loop
            progress_callback: None,
            validation_failure_policy: crate::harness::ValidationFailurePolicy::default(),
        };
        
        // P0-B5: Make provider resolution errors explicit instead of swallowed
        let req = req.with_config_provider().map_err(|e| {
            tracing::error!("P0-B5: Provider resolution failed: {}", e);
            e
        })?;

        // P0-FIX: Record provider resolution
        if req.patch_provider.is_some() {
            tracing::info!("P0: Patch provider successfully resolved from config");
        } else if proposed_edits.is_empty() {
            tracing::warn!("P0: No patch provider resolved and no edits supplied - execution will block");

            // P0-FIX: Block early with clear error message
            self.work_context_service
                .set_blocked_reason(&mut ctx, "No patch provider configured. Set PROMETHEOS_PROVIDER and PROMETHEOS_MODEL environment variables.".into())?;
            self.work_context_service.update_context(&ctx)?;
            return Err(anyhow::anyhow!("No patch provider configured and no edits supplied. Set PROMETHEOS_PROVIDER and PROMETHEOS_MODEL environment variables."));
        }

        self.work_context_service
            .update_phase(&mut ctx, WorkPhase::Execution)?;

        let result = execute_harness_task(req).await?;
        ctx.metadata = serde_json::json!({"harness":serde_json::to_value(&result)?});
        
        // P0-8 FIX: Persist EvidenceLog as first-class artifact
        let evidence_log_artifact = Artifact::new(
            uuid::Uuid::new_v4().to_string(),
            ctx.id.clone(),
            ArtifactKind::EvidenceLog,
            "evidence-log".to_string(),
            serde_json::to_value(&result.evidence_log)?,
            "harness".into(),
        );
        self.work_context_service.add_artifact(&mut ctx, evidence_log_artifact)?;
        
        // Persist other harness artifacts
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
