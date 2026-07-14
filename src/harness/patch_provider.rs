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
    edit_protocol::{
        CreateFileEdit, EditOperation, SearchReplaceEdit, UnifiedDiffEdit, WholeFileEdit,
    },
    failure::{FailureDetails, FailureKind},
    repo_intelligence::RepoMap,
    review::ReviewIssue,
};

/// Context available to patch providers for generating or repairing edits
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// Backward-compatible alias for ProviderCandidate.
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
    async fn repair(&self, _request: RepairRequest) -> anyhow::Result<RepairResponse> {
        Ok(RepairResponse {
            repaired_edits: Vec::new(),
            repair_applied: false,
            repair_notes: "Provider does not implement repair".to_string(),
            repair_time_ms: 0,
        })
    }

    /// Check if this provider can handle the given failure kind
    fn can_handle(&self, _kind: FailureKind) -> bool {
        false
    }

    /// Get provider capabilities
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }
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

impl Default for HeuristicPatchProvider {
    fn default() -> Self {
        Self::new()
    }
}

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

        let repaired_edits = repaired.unwrap_or_else(|_| request.failed_edits.clone());
        let repair_applied = repaired_edits != request.failed_edits;
        let repair_notes = if repair_applied {
            format!("Applied {:?} repair strategy", request.repair_strategy)
        } else {
            format!(
                "{:?} repair strategy did not modify the patch",
                request.repair_strategy
            )
        };

        Ok(RepairResponse {
            repaired_edits,
            repair_applied,
            repair_notes,
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
                                "\n".to_string()
                                    + &lines[(first_match_line + 1)..end_line].join("\n")
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
                                "\n".to_string()
                                    + &lines[(first_match_line + 1)..end_line].join("\n")
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

impl Default for AggregatePatchProvider {
    fn default() -> Self {
        Self::new()
    }
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
        let mut last_error: Option<anyhow::Error> = None;
        let mut last_empty_with_notes: Option<GenerateResponse> = None;
        for provider in &self.providers {
            if provider.capabilities().can_generate {
                match provider.generate(request.clone()).await {
                    Ok(response) if response.candidates.is_empty() => {
                        if response.provider_notes.is_none() {
                            anyhow::bail!(
                                "Provider returned empty candidates without diagnostic information. \
                                Provider must include diagnostic details when no candidates are generated."
                            );
                        }
                        last_empty_with_notes = Some(response);
                        continue;
                    }
                    Ok(response) => {
                        return Ok(response);
                    }
                    Err(err) => {
                        last_error = Some(err);
                        continue;
                    }
                }
            }
        }

        if let Some(response) = last_empty_with_notes {
            return Ok(response);
        }
        if let Some(err) = last_error {
            return Err(err);
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
    /// V1.6-P0-001: Create provider registry for testing only
    ///
    /// This constructor is explicitly for testing environments and may use
    /// template/static providers that are not suitable for production.
    #[cfg(test)]
    pub fn for_testing() -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();

        // Add static provider for deterministic tests
        aggregate.add_provider(Box::new(StaticPatchProvider::new("testing-static")));
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        Ok(Self { aggregate })
    }

    /// V1.6-P0-001: Create provider registry for review-only mode
    ///
    /// Review-only mode cannot apply patches, only analyze them.
    /// Uses safe providers that cannot generate side effects.
    pub fn for_review_only() -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();

        // Review-only: only analysis providers, no generation
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        // Template provider only for recognized patterns
        aggregate.add_provider(Box::new(TemplatePatchProvider::new()));

        Ok(Self { aggregate })
    }

    /// V1.6-P0-001: Create provider registry for assisted mode
    ///
    /// Assisted mode requires a real generator (LLM/script/codemod) with user oversight.
    pub fn for_assisted(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();

        // Require real generator for assisted mode
        if !config.base_url.is_empty() && !config.model.is_empty() {
            let client = crate::llm::LlmClient::new(&config.base_url, &config.model)?;
            aggregate.add_provider(Box::new(LlmPatchProvider::new(
                client,
                config.model.clone(),
            )));
        } else if let Ok(script_path) = std::env::var("PROMETHEOS_SCRIPT_PROVIDER") {
            let path = PathBuf::from(script_path);
            if path.exists() {
                aggregate.add_provider(Box::new(ScriptPatchProvider::new(path)));
            } else {
                bail!("Script provider path does not exist: {}", path.display());
            }
        } else {
            bail!("Assisted mode requires LLM configuration or script provider");
        }

        // Add heuristic provider as repair fallback
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        // Validate we have a real generator
        if !aggregate.capabilities().can_generate {
            bail!("Assisted mode requires at least one provider with can_generate=true");
        }

        Ok(Self { aggregate })
    }

    /// V1.6-P0-001: Create provider registry for autonomous mode
    ///
    /// Autonomous mode requires the most restrictive and reliable providers.
    pub fn for_autonomous(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        let mut aggregate = AggregatePatchProvider::new();

        // Autonomous mode: require LLM provider for highest reliability
        if !config.base_url.is_empty() && !config.model.is_empty() {
            let client = crate::llm::LlmClient::new(&config.base_url, &config.model)?;
            aggregate.add_provider(Box::new(LlmPatchProvider::new(
                client,
                config.model.clone(),
            )));
        } else {
            bail!("Autonomous mode requires LLM configuration");
        }

        // Add heuristic provider as repair fallback only
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        // Validate strict requirements for autonomous mode
        let caps = aggregate.capabilities();
        if !caps.can_generate {
            bail!("Autonomous mode requires a provider with can_generate=true");
        }

        Ok(Self { aggregate })
    }

    /// Retained constructor that fails fast and points callers to mode-specific constructors.
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!(
            "ProviderRegistry::new() is deprecated. \
            Use for_testing(), for_review_only(), for_assisted(), or for_autonomous() instead. \
            This prevents accidental use of weak generators in production."
        );
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
        aggregate.add_provider(Box::new(LlmPatchProvider::with_fallback_mode(
            client, model,
        )));
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        Ok(Self { aggregate })
    }

    /// V1.6-P0-001: Create provider registry from configuration with mode awareness
    ///
    /// This method determines the appropriate mode based on configuration
    /// and creates a provider registry with the correct safety constraints.
    pub fn from_config_with_mode(
        config: &crate::config::AppConfig,
        mode: crate::harness::mode_policy::HarnessMode,
    ) -> anyhow::Result<Self> {
        match mode {
            crate::harness::mode_policy::HarnessMode::Autonomous => Self::for_autonomous(config),
            crate::harness::mode_policy::HarnessMode::Assisted => Self::for_assisted(config),
            crate::harness::mode_policy::HarnessMode::ReviewOnly => Self::for_review_only(),
            crate::harness::mode_policy::HarnessMode::Review => Self::for_review_only(),
            crate::harness::mode_policy::HarnessMode::Benchmark => Self::for_review_only(),
        }
    }

    /// Compatibility constructor that maps configuration to an explicit mode.
    pub fn from_config(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        // Check if we have a valid LLM configuration
        if config.provider.is_empty() || config.model.is_empty() {
            // P1-Issue7: Provide actionable error messages for missing provider config
            let provider_var =
                std::env::var("PROMETHEOS_PROVIDER").unwrap_or_else(|_| "not set".to_string());
            let model_var =
                std::env::var("PROMETHEOS_MODEL").unwrap_or_else(|_| "not set".to_string());
            let base_url_var =
                std::env::var("PROMETHEOS_BASE_URL").unwrap_or_else(|_| "not set".to_string());

            bail!(
                "No LLM provider configured. This is required for patch generation.\n\n
                Current configuration:\n
                - Config provider: '{}'\n
                - Config model: '{}'\n
                - Environment PROMETHEOS_PROVIDER: {}\n
                - Environment PROMETHEOS_MODEL: {}\n
                - Environment PROMETHEOS_BASE_URL: {}\n\n
                To fix this issue:\n\n
                1. Set environment variables:\n    export PROMETHEOS_PROVIDER=lmstudio\n    export PROMETHEOS_MODEL=qwen2.5-coder\n    export PROMETHEOS_BASE_URL=http://localhost:1234/v1\n\n
                2. Or create a config file at ~/.config/prometheos/config.json:\n    {{\n      \"provider\": \"lmstudio\",\n      \"model\": \"qwen2.5-coder\",\n      \"base_url\": \"http://localhost:1234/v1\"\n    }}\n\n
                3. Supported providers: lmstudio, ollama, openai, anthropic\n\n
                4. For local models, ensure your LLM server is running and accessible\n
                ",
                config.provider, config.model, provider_var, model_var, base_url_var
            );
        }

        // Authenticate with the OpenRouter API key when set (the env var the
        // rest of the codebase uses for OpenRouter-compatible providers).
        let api_key = std::env::var("OPENROUTER_API_KEY").ok();

        // Create LLM client from config
        let client = crate::llm::LlmClient::new(&config.base_url, &config.model)
            .map_err(|e| {
                // P1-Issue7: Provide actionable error messages for LLM client creation failures
                anyhow::anyhow!(
                    "Failed to create LLM client with configuration:\n\n
                    Provider: {}\n                    Model: {}\n                    Base URL: {}\n\n                    Error: {}\n\n                    To fix this issue:\n\n                    1. Check that the base URL is correct and accessible\n                    2. Verify the model name matches what the provider supports\n                    3. Ensure the LLM server is running\n                    4. Check network connectivity and firewall settings\n                    5. For local models, verify the server is started with the correct model\n\n                    Example working configurations:\n                    - LM Studio: http://localhost:1234/v1\n                    - Ollama: http://localhost:11434\n                    - OpenAI: https://api.openai.com/v1\n                    ",
                    config.provider, config.model, config.base_url, e
                )
            })?
            .with_api_key(api_key);

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
            supported_operations: vec![
                "search_replace".into(),
                "whole_file".into(),
                "create_file".into(),
            ],
            typical_latency_ms: 0,
        }
    }
}

/// V1.6-P0-004: Script-based patch provider with sandboxing
pub struct ScriptPatchProvider {
    script_path: PathBuf,
    supported_languages: Vec<String>,
    /// Runtime for sandboxed script execution
    runtime: Option<std::sync::Arc<dyn crate::harness::sandbox::CommandRuntime + Send + Sync>>,
    /// Schema validation for script output
    output_schema: ScriptOutputSchema,
    /// Maximum output size to prevent resource exhaustion
    max_output_size: usize,
}

/// V1.6-P0-004: Schema for script provider output validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOutputSchema {
    pub version: String,
    pub candidates_required: bool,
    pub max_candidates: usize,
    pub allowed_operations: Vec<String>,
}

impl Default for ScriptOutputSchema {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            candidates_required: true,
            max_candidates: 10,
            allowed_operations: vec![
                "search_replace".to_string(),
                "whole_file".to_string(),
                "create_file".to_string(),
                "delete_file".to_string(),
            ],
        }
    }
}

impl ScriptPatchProvider {
    pub fn new(script_path: PathBuf) -> Self {
        Self {
            script_path,
            supported_languages: vec!["rust".into(), "python".into(), "javascript".into()],
            runtime: None,
            output_schema: ScriptOutputSchema::default(),
            max_output_size: 1024 * 1024, // 1MB max output
        }
    }

    pub fn with_languages(mut self, languages: Vec<String>) -> Self {
        self.supported_languages = languages;
        self
    }

    /// V1.6-P0-004: Set sandbox runtime for script execution
    pub fn with_runtime(
        mut self,
        runtime: std::sync::Arc<dyn crate::harness::sandbox::CommandRuntime + Send + Sync>,
    ) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// V1.6-P0-004: Set output schema for validation
    pub fn with_schema(mut self, schema: ScriptOutputSchema) -> Self {
        self.output_schema = schema;
        self
    }

    /// V1.6-P0-004: Set maximum output size
    pub fn with_max_output_size(mut self, size: usize) -> Self {
        self.max_output_size = size;
        self
    }

    /// V1.6-P0-004: Validate script path is allowed
    fn validate_script_path(&self) -> anyhow::Result<()> {
        // Check if script path is within repository or allowlisted directory
        let script_path = &self.script_path;

        // Allow scripts in common tool directories
        let allowed_dirs = vec![
            "tools/",
            "scripts/",
            ".prometheos/scripts/",
            "node_modules/.bin/",
        ];

        let script_str = script_path.to_string_lossy();
        let is_allowed = allowed_dirs.iter().any(|dir| script_str.contains(dir));

        if !is_allowed {
            anyhow::bail!(
                "Script path '{}' is not in an allowed directory. \
                Allowed directories: {:?}. \
                This prevents execution of arbitrary scripts.",
                script_path.display(),
                allowed_dirs
            );
        }

        if !script_path.exists() {
            anyhow::bail!("Script path does not exist: {}", script_path.display());
        }

        Ok(())
    }

    /// V1.6-P0-004: Validate script output against schema
    fn validate_output(&self, output: &str) -> anyhow::Result<()> {
        // Check output size
        if output.len() > self.max_output_size {
            anyhow::bail!(
                "Script output too large: {} bytes (max: {})",
                output.len(),
                self.max_output_size
            );
        }

        // Parse as JSON and validate schema
        let parsed: serde_json::Value = serde_json::from_str(output)
            .map_err(|e| anyhow::anyhow!("Invalid JSON output from script: {}", e))?;

        // Validate required fields
        if self.output_schema.candidates_required {
            if let Some(candidates) = parsed.get("candidates") {
                if let Some(candidates_array) = candidates.as_array() {
                    if candidates_array.len() > self.output_schema.max_candidates {
                        anyhow::bail!(
                            "Too many candidates: {} (max: {})",
                            candidates_array.len(),
                            self.output_schema.max_candidates
                        );
                    }
                } else {
                    anyhow::bail!("Candidates field must be an array");
                }
            } else {
                anyhow::bail!("Candidates field is required but missing");
            }
        }

        // Validate operations if present
        if let Some(candidates) = parsed.get("candidates")
            && let Some(candidates_array) = candidates.as_array()
        {
            for candidate in candidates_array {
                if let Some(operations) = candidate.get("edits")
                    && let Some(operations_array) = operations.as_array()
                {
                    for operation in operations_array {
                        if let Some(op_type) = operation.get("type")
                            && let Some(op_str) = op_type.as_str()
                            && !self
                                .output_schema
                                .allowed_operations
                                .contains(&op_str.to_string())
                        {
                            anyhow::bail!(
                                "Operation '{}' not allowed. Allowed: {:?}",
                                op_str,
                                self.output_schema.allowed_operations
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl PatchProvider for ScriptPatchProvider {
    fn name(&self) -> &str {
        "script"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let start = std::time::Instant::now();

        // V1.6-P0-004: Validate script path before execution
        self.validate_script_path()?;

        // Build script command
        let script_command = format!(
            "{} generate --task {} --repo .",
            self.script_path.display(),
            request.context.task
        );

        // V1.6-P0-004: Execute through sandboxed runtime
        let output = if let Some(ref runtime) = self.runtime {
            // Use sandboxed runtime
            runtime
                .run_command(
                    std::path::Path::new("."),
                    &script_command,
                    30000, // 30 second timeout
                )
                .await?
        } else {
            // Fallback to local execution with warning
            tracing::warn!(
                "V1.6-P0-004: Script provider using local runtime - sandboxing disabled"
            );

            let local_output = tokio::process::Command::new(&self.script_path)
                .arg("generate")
                .arg("--task")
                .arg(&request.context.task)
                .arg("--repo")
                .arg(".")
                .output()
                .await?;

            crate::harness::validation::CommandResult {
                command: script_command,
                exit_code: local_output.status.code(),
                stdout: String::from_utf8_lossy(&local_output.stdout).into(),
                stderr: String::from_utf8_lossy(&local_output.stderr).into(),
                duration_ms: start.elapsed().as_millis() as u64,
                cached: false,
                cache_key: None,
                timed_out: false,
            }
        };

        if output.exit_code != Some(0) {
            return Ok(GenerateResponse {
                candidates: vec![],
                generation_time_ms: output.duration_ms,
                provider_notes: Some(format!("Script failed: {}", output.stderr)),
            });
        }

        // V1.6-P0-004: Validate output against schema
        self.validate_output(&output.stdout)?;

        // Parse script output (JSON format expected)
        let script_output: serde_json::Value = serde_json::from_str(&output.stdout)
            .map_err(|e| anyhow::anyhow!("Invalid JSON output from script: {}", e))?;

        let candidates = parse_script_candidates(script_output, "script")?;

        Ok(GenerateResponse {
            candidates,
            generation_time_ms: output.duration_ms,
            provider_notes: Some("Generated by sandboxed script provider".into()),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        let start = std::time::Instant::now();

        // V1.6-P0-004: Validate script path before execution
        self.validate_script_path()?;

        // Build script command
        let script_command = format!(
            "{} repair --failure {:?} --message {}",
            self.script_path.display(),
            request.failure.kind,
            request.failure.message
        );

        // V1.6-P0-004: Execute through sandboxed runtime
        let output = if let Some(ref runtime) = self.runtime {
            // Use sandboxed runtime
            runtime
                .run_command(
                    std::path::Path::new("."),
                    &script_command,
                    30000, // 30 second timeout
                )
                .await?
        } else {
            // Fallback to local execution with warning
            tracing::warn!(
                "V1.6-P0-004: Script provider repair using local runtime - sandboxing disabled"
            );

            let local_output = tokio::process::Command::new(&self.script_path)
                .arg("repair")
                .arg("--failure")
                .arg(format!("{:?}", request.failure.kind))
                .arg("--message")
                .arg(&request.failure.message)
                .output()
                .await?;

            crate::harness::validation::CommandResult {
                command: script_command,
                exit_code: local_output.status.code(),
                stdout: String::from_utf8_lossy(&local_output.stdout).into(),
                stderr: String::from_utf8_lossy(&local_output.stderr).into(),
                duration_ms: start.elapsed().as_millis() as u64,
                cached: false,
                cache_key: None,
                timed_out: false,
            }
        };

        let repaired = if output.exit_code == Some(0) {
            // V1.6-P0-004: Validate output against schema
            self.validate_output(&output.stdout)?;

            // Parse repaired edits from script output
            let script_output: serde_json::Value = serde_json::from_str(&output.stdout)
                .map_err(|e| anyhow::anyhow!("Invalid JSON output from script repair: {}", e))?;
            parse_script_edits(script_output)?
        } else {
            request.failed_edits.clone()
        };

        let repair_applied = repaired != request.failed_edits;

        Ok(RepairResponse {
            repaired_edits: repaired,
            repair_applied,
            repair_notes: "Sandboxed script-based repair".into(),
            repair_time_ms: output.duration_ms,
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
            can_generate: true,
            can_repair: true,
            max_candidates: 3,
            supported_operations: vec!["search_replace".into(), "whole_file".into()],
            typical_latency_ms: 100,
        }
    }
}

/// Template-based patch provider for common patterns
pub struct TemplatePatchProvider {
    templates: HashMap<String, Template>,
}

#[derive(Debug, Clone)]
pub(crate) struct Template {
    name: String,
    pattern: String,
    _replacements: HashMap<String, String>,
    confidence: u8,
}

impl Default for TemplatePatchProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplatePatchProvider {
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        // Add common templates
        templates.insert(
            "add_import".into(),
            Template {
                name: "add_import".into(),
                pattern: "use {import};".into(),
                _replacements: HashMap::new(),
                confidence: 85,
            },
        );

        templates.insert(
            "fix_missing_semicolon".into(),
            Template {
                name: "fix_missing_semicolon".into(),
                pattern: "{line};".into(),
                _replacements: HashMap::new(),
                confidence: 90,
            },
        );

        templates.insert("add_error_handling".into(), Template {
            name: "add_error_handling".into(),
            pattern: "match {expr} {{\n    Ok(result) => result,\n    Err(e) => return Err(e.into()),\n}}".into(),
            _replacements: HashMap::new(),
            confidence: 75,
        });

        Self { templates }
    }
}

#[async_trait]
impl PatchProvider for TemplatePatchProvider {
    fn name(&self) -> &str {
        "template"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let start = std::time::Instant::now();
        let mut candidates = Vec::new();

        // Analyze task to find matching templates
        let task_lower = request.context.task.to_lowercase();

        for template in self.templates.values() {
            if task_lower.contains(&template.name.to_lowercase().replace('_', " ")) {
                // Apply template with context
                if let Some(template_edits) = apply_template(template, &request.context) {
                    candidates.push(ProviderCandidate {
                        edits: template_edits,
                        source: "template".into(),
                        strategy: format!("template_{}", template.name),
                        confidence: template.confidence,
                        reasoning: format!("Applied {} template", template.name),
                        estimated_risk: RiskEstimate::Low,
                    });
                }
            }
        }

        let candidates_count = candidates.len();
        Ok(GenerateResponse {
            candidates,
            generation_time_ms: start.elapsed().as_millis() as u64,
            provider_notes: Some(format!(
                "Generated {} template candidates",
                candidates_count
            )),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        let start = std::time::Instant::now();
        let mut repaired = request.failed_edits.clone();

        // Try to apply template-based repairs
        let repair_applied = if request.failure.kind == FailureKind::SyntaxError {
            for template in self.templates.values() {
                if template.name == "fix_missing_semicolon" {
                    // Add semicolons to lines that might be missing them
                    let fixed_repaired = apply_semicolon_fix(&repaired);
                    repaired = fixed_repaired;
                    break;
                }
            }
            repaired != request.failed_edits
        } else {
            false
        };

        Ok(RepairResponse {
            repaired_edits: repaired,
            repair_applied,
            repair_notes: "Template-based repair".into(),
            repair_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        matches!(
            kind,
            FailureKind::SyntaxError | FailureKind::PatchParseFailure
        )
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: true,
            max_candidates: 5,
            supported_operations: vec!["search_replace".into()],
            typical_latency_ms: 5,
        }
    }
}

/// Machine-readable reasons a provider response was rejected during parsing.
///
/// These are the exact values emitted in [`ProviderParseDiagnostics`]`::rejection_reason`.
/// They let operators distinguish *why* output was unusable instead of treating
/// every failure as an opaque black box.
pub const REJECT_EMPTY_RESPONSE: &str = "empty_response";
pub const REJECT_JSON_PARSE_FAILED: &str = "json_parse_failed";
pub const REJECT_JSON_SCHEMA_INVALID: &str = "json_schema_invalid";
pub const REJECT_UNSUPPORTED_EDIT_OPERATION: &str = "unsupported_edit_operation";
pub const REJECT_EDIT_FENCE_MISSING: &str = "edit_fence_missing";
pub const REJECT_MULTIPLE_EDIT_BLOCKS: &str = "multiple_edit_blocks";
pub const REJECT_PROSE_OUTSIDE_FENCE: &str = "prose_outside_fence";
pub const REJECT_MALFORMED_MARKER_ORDER: &str = "malformed_marker_order";
pub const REJECT_UNSAFE_PATH: &str = "unsafe_path";
pub const REJECT_EMPTY_REQUIRED_SECTION: &str = "empty_required_section";
pub const REJECT_MIXED_OR_AMBIGUOUS_FORMAT: &str = "mixed_or_ambiguous_format";
pub const REJECT_NO_USABLE_EDITS: &str = "no_usable_edits";

/// Structured, content-free diagnostics for a single provider parse attempt.
///
/// Persisted by default (hash + structured fields only). Raw model output is
/// never included unless `PROMETHEOS_CAPTURE_PROVIDER_RESPONSE` is set, in which
/// case it is written separately under `.prometheos/diagnostics/` with obvious
/// secrets redacted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderParseDiagnostics {
    /// Whether any non-empty response was received from the provider.
    pub provider_response_received: bool,
    /// Length in bytes of the raw response.
    pub response_length: usize,
    /// Whether the response looked like canonical JSON (a parseable JSON object).
    pub canonical_json_detected: bool,
    /// Whether the response opened with an ```edit fence.
    pub edit_fence_detected: bool,
    /// Which parse route was attempted/selected: `canonical_json` or
    /// `edit_block_fallback`.
    pub parse_route_attempted: String,
    /// Machine-readable rejection reason (one of the `REJECT_*` constants), or
    /// empty when edits were successfully recovered.
    pub rejection_reason: String,
    /// Number of usable edits recovered from the response.
    pub usable_edit_count: usize,
    /// SHA-256 of the raw response, used as the diagnostics file name and for
    /// correlation.
    pub response_sha256: String,
    /// Whether the raw response was persisted (only when capture is enabled).
    pub raw_response_persisted: bool,
    /// First parse route evaluated (`canonical_json` once JSON was recognized,
    /// otherwise `empty_response`/`json_parse_failed` when no JSON was seen).
    pub primary_route: String,
    /// Rejection reason from the canonical-JSON route when it recognized a
    /// supported format but produced no usable edits. Preserved even if the
    /// fallback route also fails, so richer evidence is never overwritten.
    pub primary_rejection_reason: String,
    /// Fallback parse route evaluated (`edit_block_fallback`), if any.
    pub fallback_route: String,
    /// Rejection reason from the fenced edit-block fallback route.
    pub fallback_rejection_reason: String,
    /// Terminal outcome: `usable_edits` or `no_usable_edits`.
    pub final_outcome: String,
}

/// LLM-based patch provider for intelligent edit generation
pub struct LlmPatchProvider {
    client: crate::llm::LlmClient,
    model: String,
    /// P0-FIX: When true, only accept strict JSON schema responses
    /// When false, allows markdown fallback for compatibility
    strict_mode: bool,
}

impl LlmPatchProvider {
    pub fn new(client: crate::llm::LlmClient, model: String) -> Self {
        Self {
            client,
            model,
            strict_mode: true, // P0-FIX: Default to strict mode for production safety
        }
    }

    /// Create with strict mode disabled, enabling the narrow, explicitly
    /// validated fenced ```edit``` fallback in addition to the canonical JSON
    /// schema. Use this for production LLM generation so local models that emit
    /// the ```edit``` format can still produce candidates. The fallback rejects
    /// prose, malformed blocks, and unsafe paths; it is not a permissive parser.
    pub fn with_fallback_mode(client: crate::llm::LlmClient, model: String) -> Self {
        Self {
            client,
            model,
            strict_mode: false,
        }
    }

    /// P0-FIX: Explicitly set strict mode
    pub fn with_strict_mode(mut self, strict: bool) -> Self {
        self.strict_mode = strict;
        self
    }

    fn build_repair_prompt(&self, request: &RepairRequest) -> String {
        let mut prompt =
            "You are a code repair expert. A patch application failed and needs to be fixed.\n\n"
                .to_string();

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

    /// Parse provider output into edits and attach structured diagnostics.
    ///
    /// This is the single source of truth for both `parse_edits_from_response`
    /// (used by tests and the repair path) and the live `generate` path. It
    /// performs no I/O; the caller persists the diagnostics so tests and repair
    /// stay side-effect free.
    fn parse_edits_with_diagnostics(
        &self,
        response: &str,
    ) -> (Vec<EditOperation>, ProviderParseDiagnostics) {
        let response_received = !response.trim().is_empty();
        let response_length = response.len();
        let response_sha256 = sha256_hex(response);
        let trimmed = response.trim();

        let mut diag = ProviderParseDiagnostics {
            provider_response_received: response_received,
            response_length,
            canonical_json_detected: false,
            edit_fence_detected: trimmed.starts_with("```edit") || trimmed.starts_with("``` edit"),
            parse_route_attempted: if self.strict_mode {
                "canonical_json".to_string()
            } else {
                "edit_block_fallback".to_string()
            },
            rejection_reason: String::new(),
            usable_edit_count: 0,
            response_sha256: response_sha256.clone(),
            raw_response_persisted: false,
            primary_route: String::new(),
            primary_rejection_reason: String::new(),
            fallback_route: String::new(),
            fallback_rejection_reason: String::new(),
            final_outcome: String::new(),
        };

        // Route 1: canonical JSON schema. This is the *primary* route. When it
        // recognizes JSON but yields no usable edits, its rejection reason is the
        // richest evidence and MUST be preserved even if the fallback route
        // also fails. The fallback must not overwrite it.
        if let Some(json) = Self::parse_json_schema_response(response) {
            diag.canonical_json_detected = true;
            diag.primary_route = "canonical_json".to_string();
            let (edits, reason) = Self::json_edits_with_reason(&json);
            if !edits.is_empty() {
                diag.usable_edit_count = edits.len();
                diag.parse_route_attempted = "canonical_json".to_string();
                diag.primary_rejection_reason = String::new();
                diag.final_outcome = "usable_edits".to_string();
                return (edits, diag);
            }
            // Recognized JSON but no usable edits: keep the primary reason.
            diag.primary_rejection_reason = reason.to_string();
        } else if !response_received {
            diag.primary_rejection_reason = REJECT_EMPTY_RESPONSE.to_string();
        } else {
            diag.primary_rejection_reason = REJECT_JSON_PARSE_FAILED.to_string();
        }

        // Route 2: fenced ```edit``` fallback (only when not strict). This is
        // secondary. It records its own rejection reason but must NOT overwrite a
        // richer primary-route rejection reason already captured above.
        if !self.strict_mode {
            diag.fallback_route = "edit_block_fallback".to_string();
            match self.parse_edit_block(response) {
                Ok(edits) if !edits.is_empty() => {
                    diag.usable_edit_count = edits.len();
                    diag.fallback_rejection_reason = String::new();
                    diag.rejection_reason = String::new();
                    diag.final_outcome = "usable_edits".to_string();
                    return (edits, diag);
                }
                Ok(_) => {
                    diag.fallback_rejection_reason = REJECT_EMPTY_REQUIRED_SECTION.to_string();
                }
                Err(reason) => {
                    diag.fallback_rejection_reason = reason.to_string();
                }
            }
        }

        // Terminal reason precedence (richest evidence wins):
        //   1. recognized JSON but invalid schema/operations (primary);
        //   2. recognized edit block but invalid grammar (fallback);
        //   3. no supported format detected (primary or fallback).
        // The fallback must never mask a primary-route diagnosis, and a
        // recognized-but-invalid edit block outranks a mere "no format" result.
        if diag.rejection_reason.is_empty() {
            if diag.canonical_json_detected {
                diag.rejection_reason = diag.primary_rejection_reason.clone();
            } else if diag.edit_fence_detected {
                diag.rejection_reason = diag.fallback_rejection_reason.clone();
            } else if !diag.primary_rejection_reason.is_empty() {
                diag.rejection_reason = diag.primary_rejection_reason.clone();
            } else {
                diag.rejection_reason = diag.fallback_rejection_reason.clone();
            }
        }
        if diag.rejection_reason.is_empty() {
            diag.rejection_reason = REJECT_NO_USABLE_EDITS.to_string();
        }
        diag.final_outcome = "no_usable_edits".to_string();

        (Vec::new(), diag)
    }

    /// Convenience wrapper returning only the edits (no diagnostics side channel).
    /// Used by tests and the repair path; intentionally performs no persistence.
    fn parse_edits_from_response(&self, response: &str) -> Vec<EditOperation> {
        self.parse_edits_with_diagnostics(response).0
    }

    /// Extract edits from a parsed JSON schema value, returning a rejection
    /// reason when no usable edit could be derived.
    fn json_edits_with_reason(json: &serde_json::Value) -> (Vec<EditOperation>, &'static str) {
        let edits_array = match json.get("edits").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return (Vec::new(), REJECT_JSON_SCHEMA_INVALID),
        };
        let mut edits = Vec::new();
        for edit_json in edits_array {
            if let Some(edit) = Self::json_to_edit_operation(edit_json) {
                edits.push(edit);
            }
        }
        if edits.is_empty() {
            (Vec::new(), REJECT_JSON_SCHEMA_INVALID)
        } else {
            (edits, "")
        }
    }

    /// Strict single-grammar parser for fenced ```edit``` blocks.
    ///
    /// Accepts exactly one of:
    /// ```edit
    /// FILE: <relative path>
    /// SEARCH:
    /// <search lines>
    /// REPLACE:
    /// <replace lines>
    /// ```
    /// or the whole-file variant (`FILE:` / `CONTENT:`). Rejects everything else:
    /// prose outside the fence, malformed blocks, absolute/UNC/drive/`..` paths,
    /// mixed/ambiguous output, or unsupported operations. This is intentionally
    /// non-permissive: it is not a prose-rummaging parser. It enforces one exact
    /// grammar using a state machine (see validation below) and rejects every
    /// deviation: nested/embedded/multiple fences, prose outside the block,
    /// malformed or reversed markers, repeated markers, inline text on marker
    /// lines, multiple files, and unsafe paths.
    /// Strict single-grammar parser for fenced ```edit``` blocks.
    ///
    /// Returns `Ok(edits)` on a recognized grammar, or `Err(reason)` where
    /// `reason` is one of the `REJECT_*` constants. The rejection reason is the
    /// machine-readable signal that lets callers record *why* provider output
    /// was rejected instead of treating every failure as an opaque black box.
    fn parse_edit_block(&self, response: &str) -> Result<Vec<EditOperation>, &'static str> {
        let trimmed = response.trim();

        // Exactly one opening ```edit fence and one closing ``` fence.
        // No other triple-backtick sequence is allowed (no nested/embedded
        // fences), and nothing except whitespace may appear outside the block.
        if !trimmed.starts_with("```edit") && !trimmed.starts_with("``` edit") {
            return Err(REJECT_EDIT_FENCE_MISSING);
        }
        if trimmed.matches("```").count() != 2 {
            return Err(REJECT_MULTIPLE_EDIT_BLOCKS);
        }
        let open_len = if trimmed.starts_with("```edit") {
            "```edit".len()
        } else {
            "``` edit".len()
        };
        let after_open = match trimmed[open_len..].strip_prefix('\n') {
            Some(s) => s,
            None => return Err(REJECT_EDIT_FENCE_MISSING),
        };
        let close = match after_open.rfind("```") {
            Some(i) => i,
            None => return Err(REJECT_EDIT_FENCE_MISSING),
        };
        if !after_open[close + 3..].trim().is_empty() {
            return Err(REJECT_PROSE_OUTSIDE_FENCE);
        }
        let body = after_open[..close].trim();

        // Normalize CRLF and split into lines.
        let lines: Vec<&str> = body
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        if lines.is_empty() || !lines[0].trim().starts_with("FILE:") {
            return Err(REJECT_MIXED_OR_AMBIGUOUS_FORMAT);
        }
        let file_token = lines[0].trim().strip_prefix("FILE:").unwrap_or("").trim();
        if file_token.is_empty() || file_token.split_whitespace().count() != 1 {
            return Err(REJECT_MALFORMED_MARKER_ORDER);
        }
        if !is_safe_relative_path(file_token) {
            return Err(REJECT_UNSAFE_PATH);
        }

        // Strict single-grammar state machine. Exactly one of:
        //   FILE / SEARCH / REPLACE   or   FILE / CONTENT
        #[derive(PartialEq)]
        enum Mode {
            None,
            Search,
            Replace,
            Content,
        }
        let mut mode = Mode::None;
        let mut markers: Vec<&'static str> = Vec::new();
        let mut search = String::new();
        let mut replace = String::new();
        let mut content = String::new();

        for line in &lines[1..] {
            let marker = line.trim();
            if marker == "SEARCH:" {
                if mode != Mode::None {
                    return Err(REJECT_MALFORMED_MARKER_ORDER);
                }
                mode = Mode::Search;
                markers.push("SEARCH");
            } else if marker == "REPLACE:" {
                if mode == Mode::None {
                    return Err(REJECT_MALFORMED_MARKER_ORDER);
                }
                mode = Mode::Replace;
                markers.push("REPLACE");
            } else if marker == "CONTENT:" {
                if mode != Mode::None {
                    return Err(REJECT_MALFORMED_MARKER_ORDER);
                }
                mode = Mode::Content;
                markers.push("CONTENT");
            } else if marker.starts_with("FILE:") {
                // A second FILE line means multiple blocks in one fence.
                return Err(REJECT_MULTIPLE_EDIT_BLOCKS);
            } else if marker.starts_with("SEARCH:")
                || marker.starts_with("REPLACE:")
                || marker.starts_with("CONTENT:")
            {
                // Inline text on a marker line => not the exact grammar.
                return Err(REJECT_MALFORMED_MARKER_ORDER);
            } else if mode == Mode::None {
                // Prose before any section marker.
                return Err(REJECT_PROSE_OUTSIDE_FENCE);
            } else {
                let target = match mode {
                    Mode::Search => &mut search,
                    Mode::Replace => &mut replace,
                    Mode::Content => &mut content,
                    Mode::None => &mut search,
                };
                if !target.is_empty() {
                    target.push('\n');
                }
                target.push_str(line);
            }
        }

        // Accept exactly one of the two grammars.
        if markers == vec!["SEARCH", "REPLACE"] {
            if search.is_empty() {
                return Err(REJECT_EMPTY_REQUIRED_SECTION);
            }
            // Empty REPLACE is allowed (deletion).
            Ok(vec![EditOperation::SearchReplace(SearchReplaceEdit {
                file: std::path::PathBuf::from(file_token),
                search,
                replace,
                replace_all: Some(false),
                context_lines: Some(3),
            })])
        } else if markers == vec!["CONTENT"] {
            // Empty CONTENT is rejected: ambiguous (use delete_file explicitly),
            // keeping the grammar unambiguous.
            if content.is_empty() {
                return Err(REJECT_EMPTY_REQUIRED_SECTION);
            }
            Ok(vec![EditOperation::WholeFile(WholeFileEdit {
                file: std::path::PathBuf::from(file_token),
                content,
            })])
        } else {
            Err(REJECT_MIXED_OR_AMBIGUOUS_FORMAT)
        }
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
                    replace_all: json
                        .get("replace_all")
                        .and_then(|v| v.as_bool())
                        .or(Some(false)),
                    context_lines: json
                        .get("context_lines")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u16)
                        .or(Some(3)),
                }))
            }
            "whole_file" => {
                let content = json.get("content")?.as_str()?.to_string();
                Some(EditOperation::WholeFile(WholeFileEdit { file, content }))
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
            "delete_file" => Some(EditOperation::DeleteFile(
                crate::harness::edit_protocol::DeleteFileEdit { file },
            )),
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
            Respond with a SINGLE JSON object and nothing else. Do not write any \
            prose, explanations, or markdown outside the JSON object.\n\n\
            Use exactly this schema:\n\
            ```json\n\
            {{\n\
              \"edits\": [\n\
                {{\"type\": \"search_replace\", \"file\": \"path/to/file.rs\", \"search\": \"exact text to find\", \"replace\": \"replacement text\"}},\n\
                {{\"type\": \"whole_file\", \"file\": \"path/to/file.rs\", \"content\": \"full new file content\"}},\n\
                {{\"type\": \"create_file\", \"file\": \"path/to/file.rs\", \"content\": \"file content\", \"executable\": false}},\n\
                {{\"type\": \"delete_file\", \"file\": \"path/to/file.rs\"}}\n\
              ],\n\
              \"reasoning\": \"explanation of the changes\",\n\
              \"confidence\": 85\n\
            }}\n\
            ```\n\
            Rules:\n\
            - Use only repository-relative paths (no absolute paths, no `..`, no drive letters).\n\
            - Only emit edits you are confident about.\n\
            - Do not emit any text outside the JSON object.",
            request.context.task, request.context.requirements
        );

        match self.client.generate(&prompt).await {
            Ok(response) => {
                let (edits, mut diag) = self.parse_edits_with_diagnostics(&response);
                let capture = std::env::var_os("PROMETHEOS_CAPTURE_PROVIDER_RESPONSE").is_some();
                diag.raw_response_persisted = capture;
                persist_provider_parse_diagnostics(
                    &diag,
                    if capture { Some(&response) } else { None },
                );
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
                    provider_notes: Some(format!(
                        "Model: {}; response_sha256: {}",
                        self.model, diag.response_sha256
                    )),
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

    fn can_handle(&self, _kind: FailureKind) -> bool {
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

/// Reject absolute paths, Windows drive letters, UNC shares, URL schemes, and
/// `..` traversal. Only safe repository-relative paths are accepted by the
/// strict ```edit``` fallback.
fn is_safe_relative_path(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return false;
    }
    if p.contains(":\\") || p.contains("://") {
        return false;
    }
    if p.starts_with("\\\\") {
        return false;
    }
    let normalized = p.replace('\\', "/");
    for seg in normalized.split('/') {
        if seg == ".." {
            return false;
        }
    }
    true
}

/// SHA-256 hex digest of a string, used to name diagnostics files and to
/// correlate a proposal with its parse diagnostics without storing content.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let out = hasher.finalize();
    let mut hex = String::with_capacity(out.len() * 2);
    for b in out {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// Redact obvious credentials and authorization material from a raw response
/// before it is ever written to disk. Best-effort pattern matching only.
fn redact_secrets(s: &str) -> String {
    let patterns: &[(&str, &str)] = &[
        (r"sk-[A-Za-z0-9]{8,}", "sk-***REDACTED***"),
        (r"Bearer\s+[A-Za-z0-9._-]+", "Bearer ***REDACTED***"),
        (r"(?i)api[_-]?key[=:]\S+", "api_key=***REDACTED***"),
        (r"(?i)authorization:\s*\S+", "Authorization: ***REDACTED***"),
    ];
    let mut out = s.to_string();
    for (pat, repl) in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            out = re.replace_all(&out, *repl).into_owned();
        }
    }
    out
}

/// Restrict a diagnostics file's permissions where the platform supports it.
/// On Unix, the file is made readable/writable only by the owner. On Windows
/// the ACL story is left to the OS; this is a best-effort, supported-only step.
fn restrict_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = std::fs::metadata(path).map(|m| m.permissions()) {
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Persist structured parse diagnostics under `.prometheos/diagnostics/`.
///
/// The structured JSON (hash + fields, no raw content) is always written.
/// When `raw` is `Some`, the redacted raw response is written alongside it.
/// All failures are swallowed: diagnostics are best-effort observability, never
/// a reason to fail the surrounding workflow.
fn persist_provider_parse_diagnostics(diag: &ProviderParseDiagnostics, raw: Option<&str>) {
    let base = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let dir = base.join(".prometheos").join("diagnostics");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let sha = &diag.response_sha256;
    if let Ok(json) = serde_json::to_string_pretty(diag) {
        let json_path = dir.join(format!("{}.json", sha));
        let _ = std::fs::write(&json_path, json);
        restrict_permissions(&json_path);
    }

    if let Some(raw) = raw {
        let raw_path = dir.join(format!("{}.response.txt", sha));
        let _ = std::fs::write(&raw_path, redact_secrets(raw));
        restrict_permissions(&raw_path);
    }
}

// Helper functions for new providers

/// Parse script output into provider candidates
fn parse_script_candidates(
    output: serde_json::Value,
    source: &str,
) -> anyhow::Result<Vec<ProviderCandidate>> {
    let mut candidates = Vec::new();

    if let Some(candidates_array) = output.get("candidates").and_then(|v| v.as_array()) {
        for candidate_json in candidates_array {
            let edits =
                if let Some(edits_array) = candidate_json.get("edits").and_then(|v| v.as_array()) {
                    parse_script_edits_from_array(edits_array)?
                } else {
                    vec![]
                };

            candidates.push(ProviderCandidate {
                edits,
                source: source.to_string(),
                strategy: candidate_json
                    .get("strategy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("script")
                    .to_string(),
                confidence: candidate_json
                    .get("confidence")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(80) as u8,
                reasoning: candidate_json
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Generated by script")
                    .to_string(),
                estimated_risk: RiskEstimate::Low,
            });
        }
    }

    Ok(candidates)
}

/// Parse edits from script output
fn parse_script_edits(output: serde_json::Value) -> anyhow::Result<Vec<EditOperation>> {
    if let Some(edits_array) = output.get("edits").and_then(|v| v.as_array()) {
        parse_script_edits_from_array(edits_array)
    } else {
        Ok(vec![])
    }
}

/// Parse edits from JSON array
fn parse_script_edits_from_array(
    edits_array: &[serde_json::Value],
) -> anyhow::Result<Vec<EditOperation>> {
    let mut edits = Vec::new();

    for edit_json in edits_array {
        let edit_type = edit_json
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing edit type"))?;

        match edit_type {
            "search_replace" => {
                let file = edit_json
                    .get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let search = edit_json
                    .get("search")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing search pattern"))?;
                let replace = edit_json
                    .get("replace")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing replace pattern"))?;

                edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from(file),
                    search: search.to_string(),
                    replace: replace.to_string(),
                    replace_all: edit_json.get("replace_all").and_then(|v| v.as_bool()),
                    context_lines: edit_json
                        .get("context_lines")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u16),
                }));
            }
            "whole_file" => {
                let file = edit_json
                    .get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let content = edit_json
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file content"))?;

                edits.push(EditOperation::WholeFile(WholeFileEdit {
                    file: PathBuf::from(file),
                    content: content.to_string(),
                }));
            }
            "create_file" => {
                let file = edit_json
                    .get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let content = edit_json
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                edits.push(EditOperation::CreateFile(CreateFileEdit {
                    file: PathBuf::from(file),
                    content: content.to_string(),
                    executable: None,
                }));
            }
            _ => {
                return Err(anyhow::anyhow!("Unsupported edit type: {}", edit_type));
            }
        }
    }

    Ok(edits)
}

/// Apply template to generate edits
fn apply_template(
    template: &Template,
    context: &PatchProviderContext,
) -> Option<Vec<EditOperation>> {
    let mut edits = Vec::new();

    match template.name.as_str() {
        "add_import" => {
            // Extract import from task description
            let task_lower = context.task.to_lowercase();
            if let Some(start) = task_lower.find("import ") {
                let remaining = &task_lower[start..];
                if let Some(end) =
                    remaining.find(|c: char| c.is_whitespace() && !c.is_alphanumeric())
                {
                    let import = &remaining[7..end];
                    edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                        file: PathBuf::from("src/main.rs"), // Default assumption
                        search: "".to_string(),
                        replace: format!("use {};", import),
                        replace_all: None,
                        context_lines: Some(0),
                    }));
                }
            }
        }
        "fix_missing_semicolon" => {
            // This would be handled in repair
        }
        "add_error_handling" => {
            // Look for function calls that might need error handling
            let task_lower = context.task.to_lowercase();
            if task_lower.contains("error") || task_lower.contains("result") {
                edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from("src/main.rs"), // Default assumption
                    search: "{expr}".to_string(),
                    replace: template.pattern.clone(),
                    replace_all: None,
                    context_lines: Some(2),
                }));
            }
        }
        _ => {}
    }

    if edits.is_empty() { None } else { Some(edits) }
}

/// Apply semicolon fix to edits
fn apply_semicolon_fix(edits: &[EditOperation]) -> Vec<EditOperation> {
    let mut fixed = Vec::new();

    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                let mut new_sr = sr.clone();

                // Add semicolon if missing and it looks like a statement
                if !sr.replace.trim_end().ends_with(';')
                    && (sr.replace.contains("let ")
                        || sr.replace.contains("fn ")
                        || sr.replace.contains("return "))
                {
                    new_sr.replace.push(';');
                }

                fixed.push(EditOperation::SearchReplace(new_sr));
            }
            _ => fixed.push(edit.clone()),
        }
    }

    fixed
}

/// Deterministic safe patch provider that generates predictable patches
///
/// This provider uses deterministic algorithms to generate safe patches
/// without relying on external services or random generation.
pub struct DeterministicPatchProvider {
    /// Seed for deterministic behavior
    _seed: u64,
}

impl DeterministicPatchProvider {
    /// Create a new deterministic patch provider with the given seed
    pub fn new(seed: u64) -> Self {
        Self { _seed: seed }
    }

    /// Create a deterministic patch provider with default seed
    pub fn new_default() -> Self {
        Self { _seed: 42 }
    }
}

#[async_trait]
impl PatchProvider for DeterministicPatchProvider {
    fn name(&self) -> &str {
        "deterministic"
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let start = std::time::Instant::now();
        let mut candidates = Vec::new();

        // Generate deterministic candidates based on task analysis
        if let Some(candidate) = self.generate_safe_candidate(&request.context) {
            candidates.push(candidate);
        }

        Ok(GenerateResponse {
            candidates,
            generation_time_ms: start.elapsed().as_millis() as u64,
            provider_notes: Some("Generated deterministic safe patches".to_string()),
        })
    }

    async fn repair(&self, request: RepairRequest) -> anyhow::Result<RepairResponse> {
        let start = std::time::Instant::now();

        // Apply deterministic repair strategies
        let repaired_edits = match request.repair_strategy {
            RepairStrategy::FixSyntaxError => {
                self.deterministic_syntax_repair(&request.failed_edits, &request.failure)
            }
            RepairStrategy::ExpandContextWindow => {
                self.deterministic_expand_context(&request.failed_edits)
            }
            RepairStrategy::NarrowSearchPattern => {
                self.deterministic_narrow_search(&request.failed_edits)
            }
            RepairStrategy::AddMissingImport => {
                self.deterministic_add_import(&request.failed_edits, &request.context)
            }
            _ => Ok(request.failed_edits.clone()),
        };

        let repair_applied = repaired_edits
            .as_ref()
            .is_ok_and(|edits| edits != &request.failed_edits);

        Ok(RepairResponse {
            repaired_edits: repaired_edits.unwrap_or(request.failed_edits),
            repair_applied,
            repair_notes: format!("Applied deterministic {:?} repair", request.repair_strategy),
            repair_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    fn can_handle(&self, kind: FailureKind) -> bool {
        // Can handle common failure types deterministically
        matches!(
            kind,
            FailureKind::PatchApplyFailure
                | FailureKind::PatchParseFailure
                | FailureKind::SyntaxError
                | FailureKind::ValidationFailed
        )
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: true,
            max_candidates: 2,
            supported_operations: vec![
                "search_replace".to_string(),
                "whole_file".to_string(),
                "create_file".to_string(),
            ],
            typical_latency_ms: 5,
        }
    }
}

impl DeterministicPatchProvider {
    /// Generate a safe candidate based on deterministic analysis
    fn generate_safe_candidate(&self, context: &PatchProviderContext) -> Option<ProviderCandidate> {
        let task_lower = context.task.to_lowercase();

        // Use deterministic pattern matching to generate safe edits
        let edits = if task_lower.contains("import") {
            self.generate_import_edit(&task_lower)
        } else if task_lower.contains("function") || task_lower.contains("fn ") {
            self.generate_function_edit(&task_lower)
        } else if task_lower.contains("fix") || task_lower.contains("error") {
            self.generate_fix_edit(&task_lower)
        } else {
            Vec::new()
        };

        if edits.is_empty() {
            None
        } else {
            Some(ProviderCandidate {
                edits,
                source: "deterministic".to_string(),
                strategy: "safe_pattern".to_string(),
                confidence: 75, // Moderate confidence for deterministic patches
                reasoning: "Generated using deterministic safe patterns".to_string(),
                estimated_risk: RiskEstimate::Low,
            })
        }
    }

    /// Generate import edits deterministically
    fn generate_import_edit(&self, task: &str) -> Vec<EditOperation> {
        let mut edits = Vec::new();

        // Look for import patterns in the task
        if let Some(import_start) = task.find("import ") {
            let remaining = &task[import_start + 7..];
            if let Some(import_end) =
                remaining.find(|c: char| c.is_whitespace() && !c.is_alphanumeric())
            {
                let import_name = &remaining[..import_end];
                edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from("src/main.rs"),
                    search: "".to_string(),
                    replace: format!("use {};\n", import_name),
                    replace_all: None,
                    context_lines: Some(0),
                }));
            }
        }

        edits
    }

    /// Generate function edits deterministically
    fn generate_function_edit(&self, task: &str) -> Vec<EditOperation> {
        let mut edits = Vec::new();

        // Simple function template
        let function_name = if let Some(fn_start) = task.find("fn ") {
            let remaining = &task[fn_start + 3..];
            if let Some(fn_end) = remaining.find(|c: char| !c.is_alphanumeric() && c != '_') {
                &remaining[..fn_end]
            } else {
                "new_function"
            }
        } else {
            "new_function"
        };

        edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/main.rs"),
            search: "".to_string(),
            replace: format!(
                "fn {}() -> Result<(), Box<dyn std::error::Error>> {{\n    Ok(())\n}}\n",
                function_name
            ),
            replace_all: None,
            context_lines: Some(0),
        }));

        edits
    }

    /// Generate fix edits deterministically
    fn generate_fix_edit(&self, task: &str) -> Vec<EditOperation> {
        let mut edits = Vec::new();

        // Common fix patterns
        if task.contains("semicolon") {
            edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                file: PathBuf::from("src/main.rs"),
                search: r"([a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;])".to_string(),
                replace: "$1;".to_string(),
                replace_all: None,
                context_lines: Some(1),
            }));
        }

        if task.contains("missing") && task.contains("import") {
            edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                file: PathBuf::from("src/main.rs"),
                search: "".to_string(),
                replace: "use std::io;\n".to_string(),
                replace_all: None,
                context_lines: Some(0),
            }));
        }

        edits
    }

    /// Deterministic syntax repair
    fn deterministic_syntax_repair(
        &self,
        edits: &[EditOperation],
        _failure: &FailureDetails,
    ) -> anyhow::Result<Vec<EditOperation>> {
        let mut repaired = Vec::new();

        for edit in edits {
            match edit {
                EditOperation::SearchReplace(sr) => {
                    let mut new_sr = sr.clone();

                    // Add missing semicolons for statements
                    if !sr.replace.trim_end().ends_with(';')
                        && (sr.replace.contains("let ") || sr.replace.contains("return "))
                    {
                        new_sr.replace.push(';');
                    }

                    // Fix common syntax issues
                    if sr.replace.contains("{{") {
                        new_sr.replace = sr.replace.replace("{{", "{");
                    }
                    if sr.replace.contains("}}") {
                        new_sr.replace = sr.replace.replace("}}", "}");
                    }

                    repaired.push(EditOperation::SearchReplace(new_sr));
                }
                _ => repaired.push(edit.clone()),
            }
        }

        Ok(repaired)
    }

    /// Deterministic context expansion
    fn deterministic_expand_context(
        &self,
        edits: &[EditOperation],
    ) -> anyhow::Result<Vec<EditOperation>> {
        let mut expanded = Vec::new();

        for edit in edits {
            match edit {
                EditOperation::SearchReplace(sr) => {
                    let mut new_sr = sr.clone();
                    // Add 3 more lines of context deterministically
                    new_sr.context_lines = sr.context_lines.map(|c| c.saturating_add(3));
                    expanded.push(EditOperation::SearchReplace(new_sr));
                }
                _ => expanded.push(edit.clone()),
            }
        }

        Ok(expanded)
    }

    /// Deterministic search narrowing
    fn deterministic_narrow_search(
        &self,
        edits: &[EditOperation],
    ) -> anyhow::Result<Vec<EditOperation>> {
        let mut narrowed = Vec::new();

        for edit in edits {
            match edit {
                EditOperation::SearchReplace(sr) => {
                    let mut new_sr = sr.clone();
                    // Add more specific context by including the first line
                    if let Some(first_line) = sr.search.lines().next()
                        && !sr.search.starts_with(first_line)
                    {
                        new_sr.search = format!("{}\n{}", first_line, sr.search);
                    }
                    narrowed.push(EditOperation::SearchReplace(new_sr));
                }
                _ => narrowed.push(edit.clone()),
            }
        }

        Ok(narrowed)
    }

    /// Deterministic import addition
    fn deterministic_add_import(
        &self,
        edits: &[EditOperation],
        _context: &PatchProviderContext,
    ) -> anyhow::Result<Vec<EditOperation>> {
        let mut with_imports = edits.to_vec();

        // Add common imports that might be missing
        let common_imports = vec![
            "use std::collections::HashMap;",
            "use std::error::Error;",
            "use anyhow::Result;",
        ];

        for import in common_imports {
            with_imports.insert(
                0,
                EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from("src/main.rs"),
                    search: "".to_string(),
                    replace: format!("{}\n", import),
                    replace_all: None,
                    context_lines: Some(0),
                }),
            );
        }

        Ok(with_imports)
    }
}

/// Deterministic mock `PatchProvider` used for tests and offline CI.
///
/// This is a *mock implementation of the existing `PatchProvider` trait* (not a
/// second provider abstraction). It emits fixed, controllable edits so the
/// governed workflow can be exercised end to end without any network or model
/// access. Construct it with [`MockProposalProvider::safe`] for the happy path,
/// or [`MockProposalProvider::with_mode`] to exercise a specific negative case.
pub struct MockProposalProvider {
    mode: MockProposalMode,
}

/// Operating modes for [`MockProposalProvider`], covering the happy path and the
/// hostile/negative cases the governed workflow must reject.
#[derive(Debug, Clone, Copy)]
pub enum MockProposalMode {
    /// Create a new in-scope file (`src/generated_patch.rs`).
    Safe,
    /// Create a file outside the allowed scope (`other/x.rs`).
    OutOfScope,
    /// Create a file under a forbidden prefix (`src/secrets/k.rs`).
    Forbidden,
    /// Create a dependency manifest (`Cargo.toml`).
    Dependency,
    /// Create an absolute-path file (`/etc/passwd`).
    Absolute,
    /// Create a file using `..` traversal (`src/../escape.rs`).
    Traversal,
    /// Return a candidate whose patch is a malformed/unsupported diff.
    Malformed,
    /// Return a candidate whose patch is plain text, not a unified diff.
    PlainText,
    /// Create a Windows drive-absolute file (`C:\...`).
    WindowsDrive,
    /// Create a UNC-path file (`\\server\share\...`).
    Unc,
    /// Return a candidate with no edits (empty patch).
    Empty,
    /// Fail generation entirely (no artifact should be created).
    Failing,
}

impl MockProposalProvider {
    /// Create the happy-path mock provider (in-scope file creation).
    pub fn safe() -> Self {
        Self {
            mode: MockProposalMode::Safe,
        }
    }

    /// Create the mock provider in a specific mode.
    pub fn with_mode(mode: MockProposalMode) -> Self {
        Self { mode }
    }
}

fn build_candidate(edits: Vec<EditOperation>, strategy: &str) -> ProviderCandidate {
    ProviderCandidate {
        edits,
        source: "mock".to_string(),
        strategy: strategy.to_string(),
        confidence: 100,
        reasoning: "deterministic mock provider".to_string(),
        estimated_risk: RiskEstimate::Low,
    }
}

#[async_trait]
impl PatchProvider for MockProposalProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn generate(&self, _request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let edits = match self.mode {
            MockProposalMode::Failing => {
                anyhow::bail!("mock provider failure (injected)");
            }
            MockProposalMode::Empty => vec![],
            MockProposalMode::Malformed => vec![EditOperation::UnifiedDiff(UnifiedDiffEdit {
                diff: "--- a/x\n+++ b/x\nGIT binary patch\nliteral 0\nH4sIAAAAAAAA\n".to_string(),
                target_file: None,
            })],
            MockProposalMode::PlainText => vec![EditOperation::UnifiedDiff(UnifiedDiffEdit {
                diff: "this is a note, not a unified diff".to_string(),
                target_file: None,
            })],
            MockProposalMode::WindowsDrive => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("C:\\windows\\system32\\evil.dll"),
                content: "bad\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Unc => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("\\\\server\\share\\evil.dll"),
                content: "bad\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Safe => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("src/generated_patch.rs"),
                content: "pub fn generated() -> u32 {\n    1\n}\n".to_string(),
                executable: None,
            })],
            MockProposalMode::OutOfScope => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("other/x.rs"),
                content: "pub fn x() {}\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Forbidden => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("src/secrets/k.rs"),
                content: "secret\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Dependency => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("Cargo.toml"),
                content: "version = \"0.2\"\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Absolute => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("/etc/passwd"),
                content: "root:x:0:0:root:/root:/bin/sh\n".to_string(),
                executable: None,
            })],
            MockProposalMode::Traversal => vec![EditOperation::CreateFile(CreateFileEdit {
                file: PathBuf::from("src/../escape.rs"),
                content: "pub fn escape() {}\n".to_string(),
                executable: None,
            })],
        };

        let candidates = if edits.is_empty() {
            vec![]
        } else {
            vec![build_candidate(edits, "deterministic")]
        };

        Ok(GenerateResponse {
            candidates,
            generation_time_ms: 0,
            provider_notes: Some(format!("mock provider mode: {:?}", self.mode)),
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec!["create_file".to_string()],
            typical_latency_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::edit_protocol::EditOperation;
    use crate::llm::LlmClient;

    fn strict_provider() -> LlmPatchProvider {
        LlmPatchProvider::new(
            LlmClient::new("http://localhost", "test-model").unwrap(),
            "test-model".to_string(),
        )
    }

    fn fallback_provider() -> LlmPatchProvider {
        LlmPatchProvider::with_fallback_mode(
            LlmClient::new("http://localhost", "test-model").unwrap(),
            "test-model".to_string(),
        )
    }

    const VALID_JSON: &str = r#"{"edits":[{"type":"search_replace","file":"src/foo.rs","search":"a","replace":"b"}],"reasoning":"x","confidence":80}"#;

    const VALID_EDIT_BLOCK: &str = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\nb\n```";

    #[test]
    fn valid_strict_json_parses() {
        let p = strict_provider();
        let edits = p.parse_edits_from_response(VALID_JSON);
        assert_eq!(edits.len(), 1);
        assert!(matches!(edits[0], EditOperation::SearchReplace(_)));
    }

    #[test]
    fn valid_fenced_edit_fallback_parses() {
        let p = fallback_provider();
        let edits = p.parse_edits_from_response(VALID_EDIT_BLOCK);
        assert_eq!(edits.len(), 1);
        assert!(matches!(edits[0], EditOperation::SearchReplace(_)));
    }

    #[test]
    fn prose_only_rejected() {
        let p = fallback_provider();
        let edits = p.parse_edits_from_response(
            "I think we should add a regression test but I cannot produce a patch.",
        );
        assert!(edits.is_empty(), "prose-only must be rejected");
    }

    #[test]
    fn bad_paths_rejected() {
        let p = fallback_provider();
        let dangerous = [
            "/etc/passwd",
            "../../escape.rs",
            "..\\secrets\\x.rs",
            "C:\\windows\\x.rs",
            "\\\\server\\share\\x.rs",
            "http://example.com/x.rs",
        ];
        for path in dangerous {
            let block = format!("```edit\nFILE: {}\nSEARCH:\na\nREPLACE:\nb\n```", path);
            let edits = p.parse_edits_from_response(&block);
            assert!(edits.is_empty(), "unsafe path must be rejected: {}", path);
        }
    }

    #[test]
    fn mixed_json_prose_rejected() {
        let p = fallback_provider();
        let resp = "Here is the patch:\n```json\n{\"edits\":[{\"type\":\"search_replace\",\"file\":\"src/foo.rs\",\"search\":\"a\",\"replace\":\"b\"}]}\n```\nLet me know if you need more.";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "mixed JSON+prose must be rejected");
    }

    #[test]
    fn malformed_edit_block_rejected() {
        let p = fallback_provider();
        let cases = [
            "```edit\nSEARCH:\na\n```",
            "```edit\nFILE: src/foo.rs\nSome prose in the middle\nSEARCH:\na\nREPLACE:\nb\n```",
            "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\nb\n``` extra trailing text",
            "```edit\nFILE: src/foo.rs\nFOO:\na\n```",
        ];
        for c in cases {
            let edits = p.parse_edits_from_response(c);
            assert!(
                edits.is_empty(),
                "malformed block must be rejected: {:?}",
                c
            );
        }
    }

    #[test]
    fn reconstruction_capture_rejected_as_mixed() {
        let p = fallback_provider();
        let response = include_str!(
            "../../docs/research/fixtures/task1-attempt2-reconstruction-ollama-response.txt"
        );
        // The reconstruction is prose followed by a valid ```edit block (mixed),
        // so the strict parser must reject it rather than rummage through prose.
        let edits = p.parse_edits_from_response(response);
        assert!(
            edits.is_empty(),
            "reconstruction (mixed prose+block) must be rejected by the strict parser"
        );
    }

    #[test]
    fn reject_mixed_content_and_search_replace() {
        let p = fallback_provider();
        // FILE/SEARCH then CONTENT is neither grammar: reject.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nCONTENT:\nb\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(
            edits.is_empty(),
            "mixed CONTENT + SEARCH/REPLACE must be rejected"
        );
    }

    #[test]
    fn reject_replace_before_search() {
        let p = fallback_provider();
        // REPLACE before SEARCH violates the exact grammar: reject.
        let resp = "```edit\nFILE: src/foo.rs\nREPLACE:\nb\nSEARCH:\na\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "REPLACE before SEARCH must be rejected");
    }

    #[test]
    fn reject_repeated_search_marker() {
        let p = fallback_provider();
        // A second SEARCH marker is ambiguous: reject.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nSEARCH:\nb\nREPLACE:\nc\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "repeated SEARCH marker must be rejected");
    }

    #[test]
    fn reject_repeated_replace_marker() {
        let p = fallback_provider();
        // A second REPLACE marker is ambiguous: reject.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\nb\nREPLACE:\nc\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "repeated REPLACE marker must be rejected");
    }

    #[test]
    fn reject_inline_text_on_marker_line() {
        let p = fallback_provider();
        // Inline text after a marker colon is not the exact grammar: reject.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH: unexpected inline text\na\nREPLACE:\nb\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(
            edits.is_empty(),
            "inline text on marker line must be rejected"
        );
    }

    #[test]
    fn reject_multiple_edit_blocks() {
        let p = fallback_provider();
        // Two fenced edit blocks in one response: reject (exactly one block allowed).
        let resp = "```edit\nFILE: src/foo.rs\nCONTENT:\nx\n```\n\n```edit\nFILE: src/bar.rs\nCONTENT:\ny\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "multiple edit blocks must be rejected");
    }

    #[test]
    fn rejects_nested_or_multiple_fences() {
        let p = fallback_provider();
        let response = r#"```edit
FILE: src/lib.rs
CONTENT:
fn x() {}

```edit
FILE: src/other.rs
CONTENT:
fn y() {}
```"#;

        assert!(
            p.parse_edits_from_response(response).is_empty(),
            "nested/multiple fences must be rejected"
        );
    }

    #[test]
    fn reject_embedded_fence_in_content() {
        let p = fallback_provider();
        // A triple-backtick sequence inside CONTENT is a nested fence: reject.
        let resp = "```edit\nFILE: src/foo.rs\nCONTENT:\n```rust\nfn x() {}\n```\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(
            edits.is_empty(),
            "embedded fence inside CONTENT must be rejected"
        );
    }

    #[test]
    fn allow_empty_replace_for_deletion() {
        let p = fallback_provider();
        // Empty REPLACE is an intentional deletion and is accepted.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\n```";
        let edits = p.parse_edits_from_response(resp);
        assert_eq!(edits.len(), 1, "empty REPLACE (deletion) must be accepted");
        match &edits[0] {
            EditOperation::SearchReplace(sr) => assert_eq!(sr.replace, ""),
            _ => panic!("expected a SearchReplace edit"),
        }
    }

    #[test]
    fn empty_content_rejected() {
        let p = fallback_provider();
        // Empty CONTENT is ambiguous (use delete_file explicitly): reject.
        let resp = "```edit\nFILE: src/foo.rs\nCONTENT:\n```";
        let edits = p.parse_edits_from_response(resp);
        assert!(edits.is_empty(), "empty CONTENT must be rejected");
    }

    #[test]
    fn diagnostics_record_rejection_reason() {
        let p = fallback_provider();
        // Prose outside the fence is the exact Attempt 3 failure class:
        // the model responded, but the parser recovered zero usable edits and
        // could not explain why. The structured diagnostics must capture that.
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\nb\n```\nHere is the patch.";
        let (edits, diag) = p.parse_edits_with_diagnostics(resp);
        assert!(
            edits.is_empty(),
            "block with trailing prose must be rejected"
        );
        assert!(diag.provider_response_received);
        assert!(diag.edit_fence_detected);
        assert_eq!(diag.usable_edit_count, 0);
        assert_eq!(diag.rejection_reason, "prose_outside_fence");
        assert_eq!(diag.parse_route_attempted, "edit_block_fallback");
        assert!(!diag.response_sha256.is_empty());
        assert!(!diag.raw_response_persisted);
    }

    #[test]
    fn diagnostics_record_success() {
        let p = fallback_provider();
        let resp = "```edit\nFILE: src/foo.rs\nSEARCH:\na\nREPLACE:\nb\n```";
        let (edits, diag) = p.parse_edits_with_diagnostics(resp);
        assert_eq!(edits.len(), 1, "valid block must parse");
        assert_eq!(diag.usable_edit_count, 1);
        assert!(
            diag.rejection_reason.is_empty(),
            "success has no rejection reason"
        );
    }

    #[test]
    fn diagnostics_primary_reason_not_overwritten_by_fallback() {
        let p = fallback_provider();
        // Canonical JSON is recognized but carries no usable edits; the fallback
        // route also fails (no edit fence). The terminal rejection_reason must
        // reflect the richer primary route, not the fallback's edit_fence_missing.
        let resp = r#"{"edits": []}"#;
        let (_edits, diag) = p.parse_edits_with_diagnostics(resp);
        assert!(diag.canonical_json_detected);
        assert!(!diag.edit_fence_detected);
        assert_eq!(diag.primary_route, "canonical_json");
        assert_eq!(diag.primary_rejection_reason, "json_schema_invalid");
        assert_eq!(diag.fallback_route, "edit_block_fallback");
        assert_eq!(diag.fallback_rejection_reason, "edit_fence_missing");
        assert_eq!(diag.rejection_reason, "json_schema_invalid");
        assert_eq!(diag.final_outcome, "no_usable_edits");
        assert_eq!(diag.usable_edit_count, 0);
    }

    #[test]
    fn diagnostics_no_format_detected_falls_through_to_fallback() {
        let p = fallback_provider();
        // Neither JSON nor an edit fence: primary = json_parse_failed, fallback =
        // edit_fence_missing; terminal reason follows fallback (no richer primary).
        let resp = "The model produced only prose with no structured edits.";
        let (_edits, diag) = p.parse_edits_with_diagnostics(resp);
        assert!(!diag.canonical_json_detected);
        assert!(!diag.edit_fence_detected);
        assert_eq!(diag.primary_rejection_reason, "json_parse_failed");
        assert_eq!(diag.fallback_rejection_reason, "edit_fence_missing");
        assert_eq!(diag.rejection_reason, "json_parse_failed");
        assert_eq!(diag.final_outcome, "no_usable_edits");
    }
}
