//! Configuration types

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default = "super::defaults::default_embedding_url")]
    pub embedding_url: String,
    #[serde(default = "super::defaults::default_embedding_dimension")]
    pub embedding_dimension: usize,
    #[serde(default = "super::defaults::default_memory_db_path")]
    pub memory_db_path: String,
    #[serde(default = "super::defaults::default_context_window_size")]
    pub context_window_size: usize,
    #[serde(default = "super::defaults::default_memory_budget")]
    pub memory_budget: MemoryBudget,
    #[serde(default = "super::defaults::default_strict_mode")]
    pub strict_mode: StrictMode,
    #[serde(default = "super::defaults::default_repo_path")]
    pub repo_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct StrictMode {
    /// Enforce error on missing inputs instead of silent fallback
    #[serde(default = "super::defaults::default_strict_mode_enforce_missing_inputs")]
    pub enforce_missing_inputs: bool,
    /// Enforce error on missing services instead of silent fallback
    #[serde(default = "super::defaults::default_strict_mode_enforce_missing_services")]
    pub enforce_missing_services: bool,
    /// Enforce error on empty outputs instead of silent fallback
    #[serde(default = "super::defaults::default_strict_mode_enforce_empty_outputs")]
    pub enforce_empty_outputs: bool,
    /// Enforce no unwrap() calls in code (compile-time linting)
    #[serde(default = "super::defaults::default_strict_mode_enforce_no_unwrap")]
    pub enforce_no_unwrap: bool,
    /// Enforce no silent Option::None propagation
    #[serde(default = "super::defaults::default_strict_mode_enforce_no_silent_none")]
    pub enforce_no_silent_none: bool,
    /// Enforce tool idempotency checks
    #[serde(default = "super::defaults::default_strict_mode_enforce_idempotency")]
    pub enforce_idempotency: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryBudget {
    pub project_facts: f32,
    pub user_preferences: f32,
    pub recent_episodes: f32,
    pub decisions_constraints: f32,
}
