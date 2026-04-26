//! LLM utility functions

use anyhow::Result;

use crate::config::AppConfig;
use super::client::LlmClient;

/// Generate a response using the configured LLM
pub async fn generate(prompt: &str) -> Result<String> {
    let config = AppConfig::load()?;
    LlmClient::from_config(&config)?.generate(prompt).await
}
