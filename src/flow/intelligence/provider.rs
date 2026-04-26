//! LLM Provider abstraction

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::llm::LlmClient;

/// Streaming callback type
pub type StreamCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// LLM Provider trait for provider abstraction
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a completion from the given prompt
    async fn generate(&self, prompt: &str) -> Result<String>;

    /// Generate a completion with streaming support
    async fn generate_stream(&self, prompt: &str, callback: StreamCallback) -> Result<String>;

    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the model name
    fn model(&self) -> &str;
}

/// OpenAI-compatible LLM provider wrapping LlmClient
pub struct OpenAiProvider {
    client: LlmClient,
    name: String,
}

impl OpenAiProvider {
    /// Create a new OpenAiProvider from an LlmClient
    pub fn new(client: LlmClient) -> Self {
        Self {
            name: "openai".to_string(),
            client,
        }
    }

    /// Create a new OpenAiProvider with a custom name
    pub fn with_name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, prompt: &str) -> Result<String> {
        self.client.generate(prompt).await
    }

    async fn generate_stream(&self, prompt: &str, callback: StreamCallback) -> Result<String> {
        self.client
            .generate_stream(prompt, |chunk| callback(chunk))
            .await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        // LlmClient doesn't expose model publicly, so we return a placeholder
        // In a real implementation, we'd add a model() method to LlmClient
        "unknown"
    }
}
