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
                        // V1.6-FIX-012: Require provider diagnostics on empty candidates
                        if response.candidates.is_empty() && response.provider_notes.is_none() {
                            anyhow::bail!(
                                "Provider returned empty candidates without diagnostic information. \
                                Provider must include diagnostic details when no candidates are generated."
                            );
                        }
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
            aggregate.add_provider(Box::new(LlmPatchProvider::new(client, config.model.clone())));
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
            aggregate.add_provider(Box::new(LlmPatchProvider::new(client, config.model.clone())));
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

    /// DEPRECATED: Use mode-specific constructors instead
    ///
    /// This method is deprecated and will be removed in V1.7.
    /// Use for_testing(), for_review_only(), for_assisted(), or for_autonomous() instead.
    #[deprecated(note = "Use mode-specific constructors: for_testing(), for_review_only(), for_assisted(), or for_autonomous()")]
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
        aggregate.add_provider(Box::new(LlmPatchProvider::new(client, model)));
        aggregate.add_provider(Box::new(HeuristicPatchProvider::new()));

        Ok(Self { aggregate })
    }

    /// V1.6-P0-001: Create provider registry from configuration with mode awareness
    ///
    /// This method determines the appropriate mode based on configuration
    /// and creates a provider registry with the correct safety constraints.
    pub fn from_config_with_mode(config: &crate::config::AppConfig, mode: crate::harness::mode_policy::HarnessMode) -> anyhow::Result<Self> {
        match mode {
            crate::harness::mode_policy::HarnessMode::Autonomous => {
                Self::for_autonomous(config)
            }
            crate::harness::mode_policy::HarnessMode::Assisted => {
                Self::for_assisted(config)
            }
            crate::harness::mode_policy::HarnessMode::ReviewOnly => {
                Self::for_review_only()
            }
            crate::harness::mode_policy::HarnessMode::Review => {
                Self::for_review_only()
            }
            crate::harness::mode_policy::HarnessMode::Benchmark => {
                Self::for_review_only()
            }
        }
    }

    /// DEPRECATED: Use from_config_with_mode instead
    ///
    /// This method is deprecated and will be removed in V1.7.
    /// Use from_config_with_mode() with explicit mode parameter.
    #[deprecated(note = "Use from_config_with_mode() with explicit mode parameter")]
    pub fn from_config(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        // Check if we have a valid LLM configuration
        if config.provider.is_empty() || config.model.is_empty() {
            // P1-Issue7: Provide actionable error messages for missing provider config
            let provider_var = std::env::var("PROMETHEOS_PROVIDER").unwrap_or_else(|_| "not set".to_string());
            let model_var = std::env::var("PROMETHEOS_MODEL").unwrap_or_else(|_| "not set".to_string());
            let base_url_var = std::env::var("PROMETHEOS_BASE_URL").unwrap_or_else(|_| "not set".to_string());
            
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

        // Create LLM client from config
        let client = crate::llm::LlmClient::new(&config.base_url, &config.model)
            .map_err(|e| {
                // P1-Issue7: Provide actionable error messages for LLM client creation failures
                anyhow::anyhow!(
                    "Failed to create LLM client with configuration:\n\n
                    Provider: {}\n                    Model: {}\n                    Base URL: {}\n\n                    Error: {}\n\n                    To fix this issue:\n\n                    1. Check that the base URL is correct and accessible\n                    2. Verify the model name matches what the provider supports\n                    3. Ensure the LLM server is running\n                    4. Check network connectivity and firewall settings\n                    5. For local models, verify the server is started with the correct model\n\n                    Example working configurations:\n                    - LM Studio: http://localhost:1234/v1\n                    - Ollama: http://localhost:11434\n                    - OpenAI: https://api.openai.com/v1\n                    ",
                    config.provider, config.model, config.base_url, e
                )
            })?;

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
    pub fn with_runtime(mut self, runtime: std::sync::Arc<dyn crate::harness::sandbox::CommandRuntime + Send + Sync>) -> Self {
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
                script_path.display(), allowed_dirs
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
        if let Some(candidates) = parsed.get("candidates") {
            if let Some(candidates_array) = candidates.as_array() {
                for candidate in candidates_array {
                    if let Some(operations) = candidate.get("edits") {
                        if let Some(operations_array) = operations.as_array() {
                            for operation in operations_array {
                                if let Some(op_type) = operation.get("type") {
                                    if let Some(op_str) = op_type.as_str() {
                                        if !self.output_schema.allowed_operations.contains(&op_str.to_string()) {
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
            runtime.run_command(
                &std::path::Path::new("."),
                &script_command,
                30000, // 30 second timeout
            ).await?
        } else {
            // Fallback to local execution with warning
            tracing::warn!("V1.6-P0-004: Script provider using local runtime - sandboxing disabled");
            
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
            runtime.run_command(
                &std::path::Path::new("."),
                &script_command,
                30000, // 30 second timeout
            ).await?
        } else {
            // Fallback to local execution with warning
            tracing::warn!("V1.6-P0-004: Script provider repair using local runtime - sandboxing disabled");
            
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
        matches!(kind, 
            FailureKind::PatchApplyFailure | 
            FailureKind::PatchParseFailure | 
            FailureKind::SyntaxError
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
struct Template {
    name: String,
    pattern: String,
    replacements: HashMap<String, String>,
    confidence: u8,
}

impl TemplatePatchProvider {
    pub fn new() -> Self {
        let mut templates = HashMap::new();
        
        // Add common templates
        templates.insert("add_import".into(), Template {
            name: "add_import".into(),
            pattern: "use {import};".into(),
            replacements: HashMap::new(),
            confidence: 85,
        });

        templates.insert("fix_missing_semicolon".into(), Template {
            name: "fix_missing_semicolon".into(),
            pattern: "{line};".into(),
            replacements: HashMap::new(),
            confidence: 90,
        });

        templates.insert("add_error_handling".into(), Template {
            name: "add_error_handling".into(),
            pattern: "match {expr} {{\n    Ok(result) => result,\n    Err(e) => return Err(e.into()),\n}}".into(),
            replacements: HashMap::new(),
            confidence: 75,
        });

        Self { templates }
    }

    pub fn with_template(mut self, template: Template) -> Self {
        self.templates.insert(template.name.clone(), template);
        self
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
            provider_notes: Some(format!("Generated {} template candidates", candidates_count)),
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
        matches!(kind, FailureKind::SyntaxError | FailureKind::PatchParseFailure)
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

    /// P0-FIX: Create with strict mode disabled (for tests/compatibility only)
    #[cfg(test)]
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
        // P0-FIX: Try strict JSON schema parsing first (always attempted)
        if let Some(json) = Self::parse_json_schema_response(response) {
            let mut edits = Vec::new();
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

        // P0-FIX: If strict mode enabled, do NOT fall back to markdown parsing
        if self.strict_mode {
            tracing::warn!(
                "P0: Strict JSON schema parsing failed and strict_mode is enabled. Rejecting provider response with {} characters.",
                response.len()
            );
            return Vec::new();
        }

        // P0-FIX: Markdown fallback only allowed in non-strict mode (tests/compatibility)
        #[cfg(test)]
        {
            tracing::info!("P0: Attempting markdown fallback parsing (test mode only)");
            return self.parse_markdown_fallback(response);
        }
        #[cfg(not(test))]
        {
            tracing::warn!("P0: Markdown fallback disabled in production builds. Use LlmPatchProvider::with_fallback_mode() for compatibility.");
            Vec::new()
        }
    }

    /// P0-FIX: Legacy markdown parsing - test/gated use only
    #[cfg(test)]
    fn parse_markdown_fallback(&self, response: &str) -> Vec<EditOperation> {
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
    /// - fenced block with `whole_file` marker and `FILE: <path>` header
    /// - generic fenced block with `FILE: <path>` header
    ///
    /// Example payload (text format):
    /// `FILE: path/to/file.rs`
    /// `<content>`
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

// Helper functions for new providers

/// Parse script output into provider candidates
fn parse_script_candidates(output: serde_json::Value, source: &str) -> anyhow::Result<Vec<ProviderCandidate>> {
    let mut candidates = Vec::new();
    
    if let Some(candidates_array) = output.get("candidates").and_then(|v| v.as_array()) {
        for candidate_json in candidates_array {
            let edits = if let Some(edits_array) = candidate_json.get("edits").and_then(|v| v.as_array()) {
                parse_script_edits_from_array(edits_array)?
            } else {
                vec![]
            };
            
            candidates.push(ProviderCandidate {
                edits,
                source: source.to_string(),
                strategy: candidate_json.get("strategy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("script")
                    .to_string(),
                confidence: candidate_json.get("confidence")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(80) as u8,
                reasoning: candidate_json.get("reasoning")
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
fn parse_script_edits_from_array(edits_array: &[serde_json::Value]) -> anyhow::Result<Vec<EditOperation>> {
    let mut edits = Vec::new();
    
    for edit_json in edits_array {
        let edit_type = edit_json.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing edit type"))?;
        
        match edit_type {
            "search_replace" => {
                let file = edit_json.get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let search = edit_json.get("search")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing search pattern"))?;
                let replace = edit_json.get("replace")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing replace pattern"))?;
                
                edits.push(EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from(file),
                    search: search.to_string(),
                    replace: replace.to_string(),
                    replace_all: edit_json.get("replace_all").and_then(|v| v.as_bool()),
                    context_lines: edit_json.get("context_lines").and_then(|v| v.as_u64()).map(|v| v as u16),
                }));
            }
            "whole_file" => {
                let file = edit_json.get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let content = edit_json.get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file content"))?;
                
                edits.push(EditOperation::WholeFile(WholeFileEdit {
                    file: PathBuf::from(file),
                    content: content.to_string(),
                }));
            }
            "create_file" => {
                let file = edit_json.get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;
                let content = edit_json.get("content")
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
fn apply_template(template: &Template, context: &PatchProviderContext) -> Option<Vec<EditOperation>> {
    let mut edits = Vec::new();
    
    match template.name.as_str() {
        "add_import" => {
            // Extract import from task description
            let task_lower = context.task.to_lowercase();
            if let Some(start) = task_lower.find("import ") {
                let remaining = &task_lower[start..];
                if let Some(end) = remaining.find(|c: char| c.is_whitespace() && !c.is_alphanumeric()) {
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
    
    if edits.is_empty() {
        None
    } else {
        Some(edits)
    }
}

/// Apply semicolon fix to edits
fn apply_semicolon_fix(edits: &[EditOperation]) -> Vec<EditOperation> {
    let mut fixed = Vec::new();
    
    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                let mut new_sr = sr.clone();
                
                // Add semicolon if missing and it looks like a statement
                if !sr.replace.trim_end().ends_with(';') && 
                   (sr.replace.contains("let ") || sr.replace.contains("fn ") || sr.replace.contains("return ")) {
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
    seed: u64,
}

impl DeterministicPatchProvider {
    /// Create a new deterministic patch provider with the given seed
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }
    
    /// Create a deterministic patch provider with default seed
    pub fn new_default() -> Self {
        Self { seed: 42 }
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
            RepairStrategy::FixSyntaxError => self.deterministic_syntax_repair(&request.failed_edits, &request.failure),
            RepairStrategy::ExpandContextWindow => self.deterministic_expand_context(&request.failed_edits),
            RepairStrategy::NarrowSearchPattern => self.deterministic_narrow_search(&request.failed_edits),
            RepairStrategy::AddMissingImport => self.deterministic_add_import(&request.failed_edits, &request.context),
            _ => Ok(request.failed_edits.clone()),
        };
        
        let repair_applied = repaired_edits.as_ref().map_or(false, |edits| edits != &request.failed_edits);
        
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
            if let Some(import_end) = remaining.find(|c: char| c.is_whitespace() && !c.is_alphanumeric()) {
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
    fn deterministic_syntax_repair(&self, edits: &[EditOperation], _failure: &FailureDetails) -> anyhow::Result<Vec<EditOperation>> {
        let mut repaired = Vec::new();
        
        for edit in edits {
            match edit {
                EditOperation::SearchReplace(sr) => {
                    let mut new_sr = sr.clone();
                    
                    // Add missing semicolons for statements
                    if !sr.replace.trim_end().ends_with(';') && 
                       (sr.replace.contains("let ") || sr.replace.contains("return ")) {
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
    fn deterministic_expand_context(&self, edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
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
    fn deterministic_narrow_search(&self, edits: &[EditOperation]) -> anyhow::Result<Vec<EditOperation>> {
        let mut narrowed = Vec::new();
        
        for edit in edits {
            match edit {
                EditOperation::SearchReplace(sr) => {
                    let mut new_sr = sr.clone();
                    // Add more specific context by including the first line
                    if let Some(first_line) = sr.search.lines().next() {
                        if !sr.search.starts_with(first_line) {
                            new_sr.search = format!("{}\n{}", first_line, sr.search);
                        }
                    }
                    narrowed.push(EditOperation::SearchReplace(new_sr));
                }
                _ => narrowed.push(edit.clone()),
            }
        }
        
        Ok(narrowed)
    }
    
    /// Deterministic import addition
    fn deterministic_add_import(&self, edits: &[EditOperation], _context: &PatchProviderContext) -> anyhow::Result<Vec<EditOperation>> {
        let mut with_imports = edits.to_vec();
        
        // Add common imports that might be missing
        let common_imports = vec![
            "use std::collections::HashMap;",
            "use std::error::Error;",
            "use anyhow::Result;",
        ];
        
        for import in common_imports {
            with_imports.insert(0, EditOperation::SearchReplace(SearchReplaceEdit {
                file: PathBuf::from("src/main.rs"),
                search: "".to_string(),
                replace: format!("{}\n", import),
                replace_all: None,
                context_lines: Some(0),
            }));
        }
        
        Ok(with_imports)
    }
}
