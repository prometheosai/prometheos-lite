//! Patch Provider Trait
//!
//! Defines the interface for generating patches from failure context.
//! This is the core abstraction that enables the repair loop to generate
//! actual edits rather than returning the same edits repeatedly.

use anyhow::bail;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::harness::{
    edit_protocol::{CreateFileEdit, EditOperation, SearchReplaceEdit, WholeFileEdit},
    failure::{FailureDetails, FailureKind},
    repo_intelligence::RepoMap,
    review::{ReviewIssue, ReviewReport},
    validation::ValidationResult,
};

/// Context available to patch providers for generating or repairing edits
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt_number: u32,
    pub edits: Vec<EditOperation>,
    pub result: AttemptOutcome,
    pub failure: Option<FailureDetails>,
}

/// Outcome of an attempt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttemptOutcome {
    Success,
    PatchFailed,
    ValidationFailed,
    ReviewFailed,
    RiskRejected,
}

/// A candidate patch with metadata from a provider
#[derive(Debug, Clone)]
pub struct ProviderCandidate {
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

/// Deprecated alias - use ProviderCandidate instead
#[deprecated(since = "1.6.0", note = "Use ProviderCandidate instead")]
pub type PatchCandidate = ProviderCandidate;

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
    pub candidates: Vec<ProviderCandidate>,
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
                    replace_all: sr.replace_all,
                    context_lines: sr.context_lines.map(|c| c.saturating_add(3)),
                }));
            }
            _ => repaired.push(edit.clone()),
        }
    }

    Ok(repaired)
}

/// Make search pattern more specific by expanding context
/// 
/// Reads the target file, finds all matches of the search pattern,
/// and expands the search block with surrounding context to make it unique.
pub fn narrow_search_repair(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
    use std::collections::HashMap;
    
    let mut repaired = Vec::new();
    
    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                // Read the file content
                let content = match std::fs::read_to_string(&sr.file) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Failed to read file for narrowing: {}", e);
                        repaired.push(edit.clone());
                        continue;
                    }
                };
                
                // Find all occurrences of the search pattern
                let matches: Vec<usize> = content
                    .match_indices(&sr.search)
                    .map(|(idx, _)| idx)
                    .collect();
                
                if matches.len() <= 1 {
                    // Already unique or not found - keep original
                    repaired.push(edit.clone());
                    continue;
                }
                
                // Multiple matches - need to expand context
                let lines: Vec<&str> = content.lines().collect();
                
                // Find which lines contain each match
                let mut match_lines = Vec::new();
                let mut current_pos = 0;
                for (line_idx, line) in lines.iter().enumerate() {
                    let line_start = current_pos;
                    let line_end = current_pos + line.len();
                    
                    for &match_pos in &matches {
                        if match_pos >= line_start && match_pos < line_end {
                            match_lines.push(line_idx);
                        }
                    }
                    
                    current_pos = line_end + 1; // +1 for newline
                }
                
                // Try to narrow by expanding context lines
                let context_lines = sr.context_lines.unwrap_or(0) as usize;
                let max_context = 10; // Maximum lines to expand
                
                let mut narrowed = None;
                for expand in 1..=max_context {
                    let new_context = context_lines + expand;
                    
                    // Build expanded search for each match location
                    let mut unique_expansions = Vec::new();
                    
                    for &line_idx in &match_lines {
                        let start_line = line_idx.saturating_sub(new_context);
                        let end_line = (line_idx + new_context + 1).min(lines.len());
                        
                        let expanded_search = lines[start_line..end_line].join("\n");
                        unique_expansions.push(expanded_search);
                    }
                    
                    // Check if all expansions are unique
                    let mut seen = HashMap::new();
                    let all_unique = unique_expansions.iter().all(|e| {
                        let count = seen.entry(e.clone()).or_insert(0);
                        *count += 1;
                        *count == 1
                    });
                    
                    if all_unique {
                        // Use the first match's expansion (most common case)
                        let first_match_line = match_lines[0];
                        let start_line = first_match_line.saturating_sub(new_context);
                        let end_line = (first_match_line + new_context + 1).min(lines.len());
                        
                        let expanded_search = lines[start_line..end_line].join("\n");
                        
                        // Build corresponding replace with same context
                        let expanded_replace = if sr.replace.contains('\n') {
                            // Multi-line replace - preserve context around it
                            let context_before = if start_line < first_match_line {
                                lines[start_line..first_match_line].join("\n") + "\n"
                            } else {
                                String::new()
                            };
                            
                            let context_after = if first_match_line + 1 < end_line {
                                "\n".to_string() + &lines[(first_match_line + 1)..end_line].join("\n")
                            } else {
                                String::new()
                            };
                            
                            context_before + &sr.replace + &context_after
                        } else {
                            // Single line replace - wrap with context
                            let context_before = if start_line < first_match_line {
                                lines[start_line..first_match_line].join("\n") + "\n"
                            } else {
                                String::new()
                            };
                            
                            let context_after = if first_match_line + 1 < end_line {
                                "\n".to_string() + &lines[(first_match_line + 1)..end_line].join("\n")
                            } else {
                                String::new()
                            };
                            
                            context_before + &sr.replace + &context_after
                        };
                        
                        narrowed = Some(EditOperation::SearchReplace(SearchReplaceEdit {
                            file: sr.file.clone(),
                            search: expanded_search,
                            replace: expanded_replace,
                            replace_all: sr.replace_all,
                            context_lines: Some(new_context as u16),
                        }));
                        break;
                    }
                }
                
                match narrowed {
                    Some(n) => {
                        tracing::info!(
                            file = %sr.file.display(),
                            "Narrowed search pattern by expanding context"
                        );
                        repaired.push(n);
                    }
                    None => {
                        tracing::warn!(
                            file = %sr.file.display(),
                            matches = matches.len(),
                            "Could not narrow search pattern - too many ambiguous matches"
                        );
                        // Return original with warning - caller should handle
                        repaired.push(edit.clone());
                    }
                }
            }
            _ => {
                // Non-search/replace edits pass through
                repaired.push(edit.clone());
            }
        }
    }
    
    Ok(repaired)
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

pub fn fix_unclosed_delimiters(edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
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
                    replace_all: sr.replace_all,
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

/// Provider registry that ensures at least one generator is available
///
/// This is the production entry point for patch generation. It validates
/// that at least one `can_generate = true` provider is registered.
pub struct ProviderRegistry {
    aggregate: AggregatePatchProvider,
}

impl ProviderRegistry {
    /// Create a new provider registry
    ///
    /// Fails if no generator provider is registered (only repair-only providers)
    pub fn new() -> anyhow::Result<Self> {
        let aggregate = AggregatePatchProvider::new();

        // Validate at least one provider can generate
        if !aggregate.capabilities().can_generate {
            bail!(
                "No patch generation provider registered. \
                At least one provider with can_generate=true is required. \
                Add an LLM provider, local model provider, or scripted provider."
            );
        }

        Ok(Self { aggregate })
    }

    /// Create with a specific provider
    pub fn with_provider(provider: Box<dyn PatchProvider>) -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();
        aggregate.add_provider(provider);

        if !aggregate.capabilities().can_generate {
            bail!("Provider cannot generate - at least one generator required");
        }

        Ok(Self { aggregate })
    }

    /// Create with LLM provider for production use
    ///
    /// P0-FIX: This is the production entry point for LLM-based patch generation.
    /// Use this when you have an LLM client configured and want full generation capabilities.
    pub fn with_llm_provider(client: crate::llm::LlmClient, model: String) -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();
        aggregate.add_provider(Box::new(LlmPatchProvider::new(client, model)));
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        Ok(Self { aggregate })
    }

    /// Create from application configuration
    ///
    /// P0-FIX: Production factory that reads LLM configuration from app config.
    /// Falls back to blocking behavior if LLM is not configured.
    pub fn from_config(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        // Check if we have a valid LLM configuration
        if config.provider.is_empty() || config.model.is_empty() {
            bail!(
                "No LLM provider configured. \
                Set PROMETHEOS_PROVIDER and PROMETHEOS_MODEL environment variables \
                or configure in settings file."
            );
        }

        // Create LLM client from config
        let client = crate::llm::LlmClient::new(&config.base_url, &config.model)
            .map_err(|e| anyhow::anyhow!("Failed to create LLM client: {}", e))?;

        Self::with_llm_provider(client, config.model.clone())
    }

    /// Get the aggregate provider
    pub fn provider(&self) -> &AggregatePatchProvider {
        &self.aggregate
    }

    /// Get provider info for evidence log
    pub fn provider_info(&self) -> ProviderInfo {
        let caps = self.aggregate.capabilities();
        ProviderInfo {
            name: "aggregate".into(),
            can_generate: caps.can_generate,
            can_repair: caps.can_repair,
            max_candidates: caps.max_candidates,
        }
    }
}

#[async_trait]
impl PatchProvider for ProviderRegistry {
    fn name(&self) -> &str {
        "provider_registry"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        self.aggregate.generate(request).await
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        self.aggregate.repair(request).await
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        self.aggregate.can_handle(kind)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.aggregate.capabilities()
    }
}

/// Provider information for evidence log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub can_generate: bool,
    pub can_repair: bool,
    pub max_candidates: usize,
}

/// Static patch provider for deterministic testing
///
/// Returns predefined edits based on task key. Useful for integration tests
/// where you want predictable behavior without calling external APIs.
///
/// ⚠️ P0-FIX: This provider is TEST-ONLY and gated behind #[cfg(test)].
/// It should never be used in production as it bypasses actual patch generation.
#[cfg(test)]
pub struct StaticPatchProvider {
    name: String,
    predefined_edits: HashMap<String, Vec<EditOperation>>,
}

#[cfg(test)]
impl StaticPatchProvider {
    /// Create a new static provider with no edits
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            predefined_edits: HashMap::new(),
        }
    }

    /// Add a predefined edit set for a specific task
    pub fn with_edits(mut self, task_key: impl Into<String>, edits: Vec<EditOperation>) -> Self {
        self.predefined_edits.insert(task_key.into(), edits);
        self
    }

    /// Create a test provider that always returns specific edits
    pub fn test_provider(edits: Vec<EditOperation>) -> Self {
        Self {
            name: "test_static".into(),
            predefined_edits: [("default".into(), edits)].into(),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl PatchProvider for StaticPatchProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        // Try to find edits for this task
        let task_key = &request.context.task;

        let edits = if let Some(predefined) = self.predefined_edits.get(task_key) {
            predefined.clone()
        } else if let Some(default) = self.predefined_edits.get("default") {
            default.clone()
        } else {
            vec![]
        };

        let candidates = if edits.is_empty() {
            vec![]
        } else {
            vec![ProviderCandidate {
                edits,
                source: self.name.clone(),
                strategy: "static_predefined".into(),
                confidence: 100,
                reasoning: "Predefined test edits".into(),
                estimated_risk: RiskEstimate::Low,
            }]
        };

        Ok(GenerateResponse {
            candidates,
            generation_time_ms: 0,
            provider_notes: Some(format!("Static provider '{}' returned edits", self.name)),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        // Static provider doesn't repair - return original
        Ok(RepairResponse {
            repaired_edits: request.failed_edits,
            repair_applied: false,
            repair_notes: "Static provider does not repair".into(),
            repair_time_ms: 0,
        })
    }

    fn can_handle(&self, _kind: FailureKind) -> bool {
        false // Static provider doesn't handle repairs
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec!["search_replace".into(), "whole_file".into(), "create_file".into()],
            typical_latency_ms: 0,
        }
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
        let mut edits = Vec::new();

        // First, try strict JSON schema parsing (preferred)
        if let Some(json) = Self::parse_json_schema_response(response) {
            if let Some(edits_array) = json.get("edits").and_then(|v| v.as_array()) {
                for edit_json in edits_array {
                    if let Some(edit) = Self::json_to_edit_operation(edit_json) {
                        edits.push(edit);
                    }
                }
                if !edits.is_empty() {
                    return edits;
                }
            }
        }

        // Fall back to legacy markdown parsing for backward compatibility
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
                            replace_all: Some(false),
                            context_lines: Some(3),
                        }));
                    }
                }
            }
            i += 1;
        }

        // If no structured edits found, try to parse as whole file
        if edits.is_empty() {
            edits = Self::parse_whole_file_edits(response);
        }

        edits
    }

    /// Parse whole-file edits from LLM response
    /// 
    /// Supports formats:
    /// - ```whole_file
    ///   FILE: path/to/file.rs
    ///   <content>
    ///   ```
    /// - ```
    ///   FILE: path/to/file.rs
    ///   <content>
    ///   ```
    fn parse_whole_file_edits(response: &str) -> Vec<EditOperation> {
        let mut edits = Vec::new();
        let lines: Vec<&str> = response.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            // Look for code block start with optional language marker
            if lines[i].contains("```") {
                let block_marker = lines[i];
                
                // Check if next line is FILE: marker indicating whole-file content
                if i + 1 < lines.len() && lines[i + 1].starts_with("FILE:") {
                    let file_line = lines[i + 1];
                    let file_path = file_line.strip_prefix("FILE:").unwrap_or("").trim();
                    
                    if !file_path.is_empty() {
                        // Collect all content until closing ```
                        let mut content = String::new();
                        i += 2;
                        
                        while i < lines.len() && !lines[i].contains("```") {
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(lines[i]);
                            i += 1;
                        }
                        
                        if !content.is_empty() {
                            edits.push(EditOperation::WholeFile(WholeFileEdit {
                                file: std::path::PathBuf::from(file_path),
                                content,
                            }));
                        }
                        continue;
                    }
                }
                
                // Also check for WHOLE_FILE: marker
                if i + 1 < lines.len() && lines[i + 1].starts_with("WHOLE_FILE:") {
                    let file_line = lines[i + 1];
                    let file_path = file_line.strip_prefix("WHOLE_FILE:").unwrap_or("").trim();
                    
                    if !file_path.is_empty() {
                        let mut content = String::new();
                        i += 2;
                        
                        while i < lines.len() && !lines[i].contains("```") {
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(lines[i]);
                            i += 1;
                        }
                        
                        if !content.is_empty() {
                            edits.push(EditOperation::WholeFile(WholeFileEdit {
                                file: std::path::PathBuf::from(file_path),
                                content,
                            }));
                        }
                        continue;
                    }
                }
            }
            i += 1;
        }

        // Alternative: Look for explicit CREATE_FILE markers
        let mut i = 0;
        while i < lines.len() {
            if lines[i].starts_with("CREATE_FILE:") || lines[i].starts_with("CREATE:") {
                let file_path = lines[i]
                    .strip_prefix("CREATE_FILE:")
                    .or_else(|| lines[i].strip_prefix("CREATE:"))
                    .unwrap_or("")
                    .trim();
                
                if !file_path.is_empty() && i + 1 < lines.len() {
                    // Check if next line starts a code block
                    if lines[i + 1].contains("```") {
                        let mut content = String::new();
                        i += 2;
                        
                        while i < lines.len() && !lines[i].contains("```") {
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(lines[i]);
                            i += 1;
                        }
                        
                        if !content.is_empty() {
                            edits.push(EditOperation::CreateFile(CreateFileEdit {
                                file: std::path::PathBuf::from(file_path),
                                content,
                                executable: None,
                            }));
                        }
                    }
                }
            }
            i += 1;
        }

        edits
    }

    /// Parse JSON schema response for edits
    /// 
    /// JSON Schema format:
    /// {
    ///   "edits": [
    ///     {
    ///       "type": "search_replace",
    ///       "file": "path/to/file.rs",
    ///       "search": "text to find",
    ///       "replace": "replacement text"
    ///     },
    ///     {
    ///       "type": "whole_file",
    ///       "file": "path/to/file.rs",
    ///       "content": "full file content"
    ///     },
    ///     {
    ///       "type": "create_file",
    ///       "file": "path/to/file.rs",
    ///       "content": "file content",
    ///       "executable": false
    ///     }
    ///   ],
    ///   "reasoning": "explanation of changes",
    ///   "confidence": 85,
    ///   "risks": ["potential risk 1", "risk 2"]
    /// }
    fn parse_json_schema_response(response: &str) -> Option<serde_json::Value> {
        // Try to extract JSON from code blocks or raw JSON
        let trimmed = response.trim();
        
        // Check if wrapped in code block
        if trimmed.starts_with("```json") || trimmed.starts_with("```") {
            // Extract content between code fences
            let start = trimmed.find('\n').unwrap_or(0);
            let end = trimmed.rfind("```").unwrap_or(trimmed.len());
            if start < end {
                let json_content = &trimmed[start..end].trim();
                return serde_json::from_str(json_content).ok();
            }
        }
        
        // Try parsing as raw JSON
        serde_json::from_str(trimmed).ok()
    }

    /// Convert JSON schema to EditOperation
    fn json_to_edit_operation(json: &serde_json::Value) -> Option<EditOperation> {
        let edit_type = json.get("type")?.as_str()?;
        let file = std::path::PathBuf::from(json.get("file")?.as_str()?);
        
        match edit_type {
            "search_replace" => {
                let search = json.get("search")?.as_str()?.to_string();
                let replace = json.get("replace")?.as_str()?.to_string();
                Some(EditOperation::SearchReplace(SearchReplaceEdit {
                    file,
                    search,
                    replace,
                    replace_all: json.get("replace_all").and_then(|v| v.as_bool()).or(Some(false)),
                    context_lines: json.get("context_lines").and_then(|v| v.as_u64()).map(|v| v as u16).or(Some(3)),
                }))
            }
            "whole_file" => {
                let content = json.get("content")?.as_str()?.to_string();
                Some(EditOperation::WholeFile(WholeFileEdit {
                    file,
                    content,
                }))
            }
            "create_file" => {
                let content = json.get("content")?.as_str()?.to_string();
                let executable = json.get("executable").and_then(|v| v.as_bool());
                Some(EditOperation::CreateFile(CreateFileEdit {
                    file,
                    content,
                    executable,
                }))
            }
            "delete_file" => {
                Some(EditOperation::DeleteFile(crate::harness::edit_protocol::DeleteFileEdit { file }))
            }
            _ => None,
        }
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
                    vec![ProviderCandidate {
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
