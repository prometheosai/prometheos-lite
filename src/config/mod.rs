//! Runtime configuration loading.

use std::{env, fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

pub const DEFAULT_CONFIG_PATH: &str = "prometheos.config.json";

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_embedding_url")]
    pub embedding_url: String,
    #[serde(default = "default_embedding_dimension")]
    pub embedding_dimension: usize,
    #[serde(default = "default_memory_db_path")]
    pub memory_db_path: String,
    #[serde(default = "default_context_window_size")]
    pub context_window_size: usize,
    #[serde(default = "default_memory_budget")]
    pub memory_budget: MemoryBudget,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBudget {
    pub project_facts: f32,
    pub user_preferences: f32,
    pub recent_episodes: f32,
    pub decisions_constraints: f32,
}

fn default_embedding_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_embedding_dimension() -> usize {
    1536
}

fn default_memory_db_path() -> String {
    "prometheos_memory.db".to_string()
}

fn default_context_window_size() -> usize {
    4096
}

fn default_memory_budget() -> MemoryBudget {
    MemoryBudget {
        project_facts: 0.4,
        user_preferences: 0.25,
        recent_episodes: 0.2,
        decisions_constraints: 0.15,
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        Self::load_from(DEFAULT_CONFIG_PATH)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let mut config: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        if let Ok(base_url) = env::var("PROMETHEOS_BASE_URL") {
            config.base_url = base_url;
        }

        if let Ok(model) = env::var("PROMETHEOS_MODEL") {
            config.model = model;
        }

        Ok(config)
    }
}
