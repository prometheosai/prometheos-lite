//! Configuration types

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBudget {
    pub project_facts: f32,
    pub user_preferences: f32,
    pub recent_episodes: f32,
    pub decisions_constraints: f32,
}
