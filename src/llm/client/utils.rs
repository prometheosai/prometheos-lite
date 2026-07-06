//! LLM utility functions

use anyhow::Result;

use super::llm_client::LlmClient;
use crate::config::AppConfig;

/// Generate a response using the configured LLM
pub async fn generate(prompt: &str) -> Result<String> {
    let config = AppConfig::load()?;
    LlmClient::from_config(&config)?.generate(prompt).await
}
