//! Patch Provider Trait
//!
//! Defines the interface for generating patches from failure context.
//! This is the core abstraction that enables the repair loop to generate
//! actual edits rather than returning the same edits repeatedly.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::harness::{
    edit_protocol::EditOperation,
    failure::{FailureDetails, FailureKind},
    repo_intelligence::RepoMap,
    review::{ReviewIssue, ReviewReport},
    validation::ValidationResult,
};

/// Context available to patch providers for generating or repairing edits
#[derive(Debug, Clone, Default)]
pub struct PatchProviderContext {
    /// The original task description
    pub task: String,
    /// Requirements that must be satisfied
    pub requirements: Vec<String>,
    /// Repository map with file context
    pub repo_map: Option<RepoMap>,
    /// Files mentioned in the task
    pub mentioned_files: Vec<PathBuf>,
    /// Symbols mentioned in the task
    pub mentioned_symbols: Vec<String>,
    /// Previous attempts and their outcomes
    pub attempt_history: Vec<AttemptRecord>,
    /// Validation stderr/stdout from failed runs
    pub validation_output: Option<String>,
    /// Review issues found in previous attempts
    pub review_issues: Vec<ReviewIssue>,
    /// Maximum number of candidates to generate
    pub max_candidates: usize,
}

/// Record of a previous attempt for learning
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub attempt_number: u32,
    pub edits: Vec<EditOperation>,
    pub result: AttemptOutcome,
    pub failure: Option<FailureDetails>,
}

/// Outcome of an attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttemptOutcome {
    Success,
    PatchFailed,
    ValidationFailed,
    ReviewFailed,
    RiskRejected,
}

/// A candidate patch with metadata
#[derive(Debug, Clone)]
pub struct PatchCandidate {
    /// The edits to apply
    pub edits: Vec<EditOperation>,
    /// Provider that generated this candidate
    pub source: String,
    /// Strategy used (e.g., "search_replace", "whole_file", "synthesis")
    pub strategy: String,
    /// Confidence score (0-100)
    pub confidence: u8,
    /// Reasoning for this approach
    pub reasoning: String,
    /// Estimated risk level
    pub estimated_risk: RiskEstimate,
}

/// Risk estimate for a candidate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskEstimate {
    Low,
    Medium,
    High,
    Critical,
}

/// Request to generate initial patch candidates
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub context: PatchProviderContext,
    pub preferred_strategies: Vec<String>,
}

/// Request to repair failed patches
#[derive(Debug, Clone)]
pub struct RepairRequest {
    pub context: PatchProviderContext,
    /// The failure that triggered repair
    pub failure: FailureDetails,
    /// The edits that failed
    pub failed_edits: Vec<EditOperation>,
    /// Strategy to apply for repair
    pub repair_strategy: RepairStrategy,
}

/// Strategy for repairing failed patches
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairStrategy {
    /// Widen the search pattern with more context
    ExpandContextWindow,
    /// Narrow search to be more specific
    NarrowSearchPattern,
    /// Fix obvious syntax errors
    FixSyntaxError,
    /// Use whole-file edit instead of search/replace
    UseWholeFileEdit,
    /// Add missing imports
    AddMissingImport,
    /// Remove conflicting changes
    RemoveConflictingChange,
    /// Re-synthesize based on error feedback
    ResynthesizeFromFeedback,
    /// Ask for clarification
    RequestClarification,
}

/// Response from patch generation
#[derive(Debug, Clone)]
pub struct GenerateResponse {
    pub candidates: Vec<PatchCandidate>,
    pub generation_time_ms: u64,
    pub provider_notes: Option<String>,
}

/// Response from repair attempt
#[derive(Debug, Clone)]
pub struct RepairResponse {
    pub repaired_edits: Vec<EditOperation>,
    pub repair_applied: bool,
    pub repair_notes: String,
    pub repair_time_ms: u64,
}

/// Trait for patch providers
///
/// Implementations can range from simple heuristics to LLM-based synthesis.
#[async_trait]
pub trait PatchProvider: Send + Sync {
    /// Provider name for identification
    fn name(&self) -> &str;

    /// Generate initial patch candidates for a task
    ///
    /// # Arguments
    /// * `request` - Context and constraints for generation
    ///
    /// # Returns
    /// * `GenerateResponse` - One or more candidate patches
    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse>;

    /// Repair failed patches based on failure context
    ///
    /// # Arguments
    /// * `request` - Failure details, failed edits, and repair strategy
    ///
    /// # Returns
    /// * `RepairResponse` - Repaired edits or empty if repair not possible
    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse>;

    /// Check if this provider can handle the given failure kind
    fn can_handle(&self, kind: FailureKind) -> bool;

    /// Get provider capabilities
    fn capabilities(&self) -> ProviderCapabilities;
}

/// Provider capabilities
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    /// Can generate from scratch (not just repair)
    pub can_generate: bool,
    /// Can repair failed patches
    pub can_repair: bool,
    /// Maximum candidates per request
    pub max_candidates: usize,
    /// Supported edit operation types
    pub supported_operations: Vec<String>,
    /// Average latency for generation (ms)
    pub typical_latency_ms: u64,
}

/// Simple heuristic provider for basic repairs
pub struct HeuristicPatchProvider;

impl HeuristicPatchProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PatchProvider for HeuristicPatchProvider {
    fn name(&self) -> &str {
        "heuristic"
    }

    async fn generate(&self, _request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        // Heuristic provider cannot generate from scratch
        Ok(GenerateResponse {
            candidates: vec![],
            generation_time_ms: 0,
            provider_notes: Some("Heuristic provider cannot generate from scratch".into()),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        let start = std::time::Instant::now();

        let repaired = match request.repair_strategy {
            RepairStrategy::ExpandContextWindow => expand_context_repair(&request.failed_edits),
            RepairStrategy::NarrowSearchPattern => narrow_search_repair(&request.failed_edits),
            RepairStrategy::UseWholeFileEdit => use_whole_file_repair(&request.failed_edits).await,
            RepairStrategy::FixSyntaxError => {
                fix_syntax_repair(&request.failed_edits, &request.failure)
            }
            _ => {
                // Cannot repair with this strategy
                Ok(request.failed_edits.clone())
            }
        };

        let repair_applied =
            repaired.is_ok() && repaired.as_ref().unwrap() != &request.failed_edits;

        Ok(RepairResponse {
            repaired_edits: repaired.unwrap_or(request.failed_edits),
            repair_applied,
            repair_notes: format!("Applied {:?} repair strategy", request.repair_strategy),
            repair_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        matches!(
            kind,
            FailureKind::PatchApplyFailure
                | FailureKind::PatchParseFailure
                | FailureKind::SyntaxError
        )
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: false,
            can_repair: true,
            max_candidates: 1,
            supported_operations: vec!["search_replace".into(), "whole_file".into()],
            typical_latency_ms: 10,
        }
    }
}

/// Expand context lines in search/replace operations
fn expand_context_repair(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
    use crate::harness::edit_protocol::SearchReplaceEdit;

    let mut repaired = Vec::new();

    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                // Add more context lines (simple heuristic)
                let expanded_search = format!("\n{}", sr.search);
                let expanded_replace = format!("\n{}", sr.replace);

                repaired.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: sr.file.clone(),
                    search: expanded_search,
                    replace: expanded_replace,
                    context_lines: sr.context_lines.saturating_add(3),
                }));
            }
            _ => repaired.push(edit.clone()),
        }
    }

    Ok(repaired)
}

/// Make search pattern more specific
fn narrow_search_repair(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
    // This would analyze the search pattern and add more unique context
    // For now, just return the original
    Ok(edits.to_vec())
}

/// Convert search/replace to whole-file edit
async fn use_whole_file_repair(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
    use crate::harness::edit_protocol::WholeFileEdit;

    let mut repaired = Vec::new();

    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                // Read the current file content
                let content = tokio::fs::read_to_string(&sr.file).await?;

                // Apply the replacement to get the new content
                let new_content = content.replace(&sr.search, &sr.replace);

                repaired.push(EditOperation::WholeFile(WholeFileEdit {
                    file: sr.file.clone(),
                    content: new_content,
                }));
            }
            _ => repaired.push(edit.clone()),
        }
    }

    Ok(repaired)
}

/// Attempt to fix syntax errors (basic heuristics)
fn fix_syntax_repair(
    edits: &[EditOperation],
    failure: &FailureDetails,
) -> anyhow::Result<Vec<EditOperation>> {
    // Parse failure message for common patterns
    let msg = failure.message.to_lowercase();

    if msg.contains("unclosed") || msg.contains("missing") {
        // Try to add closing brackets/parentheses
        fix_unclosed_delimiters(edits)
    } else if msg.contains("expected") && msg.contains("found") {
        // Type mismatch or similar
        Ok(edits.to_vec())
    } else {
        Ok(edits.to_vec())
    }
}

fn fix_unclosed_delimiters(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
    use crate::harness::edit_protocol::SearchReplaceEdit;

    let mut repaired = Vec::new();

    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                let mut new_replace = sr.replace.clone();

                // Count opening vs closing brackets
                let open_braces = new_replace.matches('{').count();
                let close_braces = new_replace.matches('}').count();
                let open_parens = new_replace.matches('(').count();
                let close_parens = new_replace.matches(')').count();

                // Add missing closing delimiters
                for _ in 0..(open_braces.saturating_sub(close_braces)) {
                    new_replace.push('}');
                }
                for _ in 0..(open_parens.saturating_sub(close_parens)) {
                    new_replace.push(')');
                }

                repaired.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: sr.file.clone(),
                    search: sr.search.clone(),
                    replace: new_replace,
                    context_lines: sr.context_lines,
                }));
            }
            _ => repaired.push(edit.clone()),
        }
    }

    Ok(repaired)
}

/// Aggregate provider that combines multiple providers
pub struct AggregatePatchProvider {
    providers: Vec<Box<dyn PatchProvider>>,
}

impl AggregatePatchProvider {
    pub fn new() -> Self {
        Self {
            providers: vec![Box::new(HeuristicPatchProvider::new())],
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn PatchProvider>) {
        self.providers.push(provider);
    }
}

#[async_trait]
impl PatchProvider for AggregatePatchProvider {
    fn name(&self) -> &str {
        "aggregate"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        // Try each provider that can generate
        for provider in &self.providers {
            if provider.capabilities().can_generate {
                match provider.generate(request.clone()).await {
                    Ok(response) if !response.candidates.is_empty() => {
                        return Ok(response);
                    }
                    _ => continue,
                }
            }
        }

        Ok(GenerateResponse {
            candidates: vec![],
            generation_time_ms: 0,
            provider_notes: Some("No provider could generate candidates".into()),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        // Try repair with the first capable provider
        for provider in &self.providers {
            if provider.can_handle(request.failure.kind) && provider.capabilities().can_repair {
                return provider.repair(request).await;
            }
        }

        // No provider could repair - return original edits
        Ok(RepairResponse {
            repaired_edits: request.failed_edits,
            repair_applied: false,
            repair_notes: "No provider could repair this failure".into(),
            repair_time_ms: 0,
        })
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        self.providers.iter().any(|p| p.can_handle(kind))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Aggregate capabilities
        let mut caps = ProviderCapabilities::default();

        for provider in &self.providers {
            let pc = provider.capabilities();
            caps.can_generate |= pc.can_generate;
            caps.can_repair |= pc.can_repair;
            caps.max_candidates = caps.max_candidates.max(pc.max_candidates);
            caps.typical_latency_ms = caps.typical_latency_ms.max(pc.typical_latency_ms);
        }

        caps
    }
}

/// LLM-based patch provider for intelligent edit generation
pub struct LlmPatchProvider {
    client: crate::llm::LlmClient,
    model: String,
}

impl LlmPatchProvider {
    pub fn new(client: crate::llm::LlmClient, model: String) -> Self {
        Self { client, model }
    }

    fn build_repair_prompt(&self, request: &RepairRequest) -> String {
        let mut prompt = format!(
            "You are a code repair expert. A patch application failed and needs to be fixed.\n\n"
        );

        // Add task context
        prompt.push_str(&format!("Task: {}\n", request.context.task));

        // Add failure details
        prompt.push_str(&format!("\nFailure Type: {:?}\n", request.failure.kind));
        prompt.push_str(&format!("Failure Message: {}\n", request.failure.message));

        // Add validation output if available
        if let Some(output) = &request.context.validation_output {
            prompt.push_str(&format!("\nValidation Output:\n{}\n", output));
        }

        // Add failed edits
        prompt.push_str("\nFailed Edits:\n");
        for (i, edit) in request.failed_edits.iter().enumerate() {
            prompt.push_str(&format!("Edit {}: {:?}\n", i + 1, edit));
        }

        // Add attempt history
        if !request.context.attempt_history.is_empty() {
            prompt.push_str("\nPrevious Attempts:\n");
            for attempt in &request.context.attempt_history {
                prompt.push_str(&format!(
                    "Attempt {}: {:?}\n",
                    attempt.attempt_number, attempt.result
                ));
            }
        }

        // Add repair strategy guidance
        prompt.push_str(&format!(
            "\nRepair Strategy: {:?}\n",
            request.repair_strategy
        ));
        prompt.push_str("\nPlease provide corrected edits in the same format. ");
        prompt.push_str("Fix the issue that caused the failure while preserving the intent.\n");

        prompt
    }

    fn parse_edits_from_response(&self, response: &str) -> Vec<EditOperation> {
        // Simple parsing: look for code blocks with edit format
        let mut edits = Vec::new();

        // Parse search/replace blocks
        // Format: ```edit
        // FILE: path/to/file.rs
        // SEARCH:
        // <search content>
        // REPLACE:
        // <replace content>
        // ```
        let lines: Vec<&str> = response.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if lines[i].contains("```edit") || lines[i].contains("```") {
                // Look for FILE: marker
                if i + 1 < lines.len() && lines[i + 1].starts_with("FILE:") {
                    let file_line = lines[i + 1];
                    let file_path = file_line.strip_prefix("FILE:").unwrap_or("").trim();

                    // Look for SEARCH: and REPLACE: sections
                    let mut search_content = String::new();
                    let mut replace_content = String::new();
                    let mut in_search = false;
                    let mut in_replace = false;

                    i += 2;
                    while i < lines.len() && !lines[i].contains("```") {
                        if lines[i].starts_with("SEARCH:") {
                            in_search = true;
                            in_replace = false;
                        } else if lines[i].starts_with("REPLACE:") {
                            in_search = false;
                            in_replace = true;
                        } else if in_search {
                            if !search_content.is_empty() {
                                search_content.push('\n');
                            }
                            search_content.push_str(lines[i]);
                        } else if in_replace {
                            if !replace_content.is_empty() {
                                replace_content.push('\n');
                            }
                            replace_content.push_str(lines[i]);
                        }
                        i += 1;
                    }

                    if !file_path.is_empty() && !search_content.is_empty() {
                        use crate::harness::edit_protocol::SearchReplaceEdit;
                        edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                            file: std::path::PathBuf::from(file_path),
                            search: search_content,
                            replace: replace_content,
                            context_lines: 3,
                        }));
                    }
                }
            }
            i += 1;
        }

        // If no structured edits found, try to parse as whole file
        if edits.is_empty() {
            // TODO: Implement whole-file parsing from response
        }

        edits
    }
}

#[async_trait]
impl PatchProvider for LlmPatchProvider {
    fn name(&self) -> &str {
        "llm"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let start = std::time::Instant::now();

        let prompt = format!(
            "You are a coding assistant. Generate edits to complete this task.\n\n\
            Task: {}\n\
            Requirements: {:?}\n\n\
            Generate edits using the format:\n\
            ```edit\n\
            FILE: path/to/file.rs\n\
            SEARCH:\n\
            <content to find>\n\
            REPLACE:\n\
            <new content>\n\
            ```",
            request.context.task, request.context.requirements
        );

        match self.client.generate(&prompt).await {
            Ok(response) => {
                let edits = self.parse_edits_from_response(&response);
                let candidates = if edits.is_empty() {
                    vec![]
                } else {
                    vec![PatchCandidate {
                        edits,
                        source: "llm".to_string(),
                        strategy: "generation".to_string(),
                        confidence: 70,
                        reasoning: "Generated by LLM based on task description".to_string(),
                        estimated_risk: RiskEstimate::Medium,
                    }]
                };

                Ok(GenerateResponse {
                    candidates,
                    generation_time_ms: start.elapsed().as_millis() as u64,
                    provider_notes: Some(format!("Model: {}", self.model)),
                })
            }
            Err(e) => {
                anyhow::bail!("LLM generation failed: {}", e);
            }
        }
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        let start = std::time::Instant::now();
        let prompt = self.build_repair_prompt(&request);

        match self.client.generate(&prompt).await {
            Ok(response) => {
                let repaired_edits = self.parse_edits_from_response(&response);
                let repair_applied =
                    !repaired_edits.is_empty() && repaired_edits != request.failed_edits;

                Ok(RepairResponse {
                    repaired_edits: if repair_applied {
                        repaired_edits
                    } else {
                        request.failed_edits
                    },
                    repair_applied,
                    repair_notes: format!("LLM repair using {}", self.model),
                    repair_time_ms: start.elapsed().as_millis() as u64,
                })
            }
            Err(e) => Ok(RepairResponse {
                repaired_edits: request.failed_edits,
                repair_applied: false,
                repair_notes: format!("LLM repair failed: {}", e),
                repair_time_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        // LLM can handle any failure type
        true
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: true,
            max_candidates: 3,
            supported_operations: vec![
                "search_replace".into(),
                "whole_file".into(),
                "create_file".into(),
            ],
            typical_latency_ms: 5000,
        }
    }
}
