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
