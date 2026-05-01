//! Model router for selecting and routing to different LLM providers

use super::provider::LlmProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Result of a generation with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub latency_ms: u64,
    pub fallback_used: bool,
    pub fallback_from: Option<String>,
    pub tokens_used: Option<u32>,
}

/// Model router for selecting and routing to different LLM providers
pub struct ModelRouter {
    providers: Vec<Box<dyn LlmProvider>>,
    fallback_chain: Vec<usize>,
    current_provider: usize,
}

impl ModelRouter {
    /// Create a new ModelRouter with a list of providers
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self {
            providers,
            fallback_chain: Vec::new(),
            current_provider: 0,
        }
    }

    /// Set the fallback chain (indices of providers to try in order)
    pub fn with_fallback_chain(mut self, chain: Vec<usize>) -> Self {
        self.fallback_chain = chain;
        self
    }

    /// Generate a completion using the current provider with fallback
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let result = self.generate_with_metadata(prompt).await?;
        Ok(result.content)
    }

    /// Generate a completion with metadata (provider, model, latency, fallback info)
    pub async fn generate_with_metadata(&self, prompt: &str) -> Result<GenerateResult> {
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;
        let mut fallback_from = None;

        for (i, provider_idx) in providers_to_try.iter().enumerate() {
            if let Some(provider) = self.providers.get(*provider_idx) {
                let start = Instant::now();
                match provider.generate(prompt).await {
                    Ok(result) => {
                        let latency_ms = start.elapsed().as_millis() as u64;
                        return Ok(GenerateResult {
                            content: result,
                            provider: provider.name().to_string(),
                            model: provider.model().to_string(),
                            latency_ms,
                            fallback_used: i > 0,
                            fallback_from,
                            tokens_used: None, // Provider should report this
                        });
                    }
                    Err(e) => {
                        if i > 0 {
                            fallback_from = Some(
                                self.providers
                                    .get(*provider_idx)
                                    .map(|p| p.name().to_string())
                                    .unwrap_or_else(|| "unknown".to_string()),
                            );
                        }
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Generate a completion with streaming support
    pub async fn generate_stream(
        &self,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<String> {
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;

        for provider_idx in providers_to_try {
            if let Some(provider) = self.providers.get(provider_idx) {
                match provider.generate_stream(prompt, callback.clone()).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Generate a completion with streaming support and metadata
    pub async fn generate_stream_with_metadata(
        &self,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<GenerateResult> {
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;
        let mut fallback_from = None;

        for (i, provider_idx) in providers_to_try.iter().enumerate() {
            if let Some(provider) = self.providers.get(*provider_idx) {
                let start = Instant::now();
                match provider.generate_stream(prompt, callback.clone()).await {
                    Ok(result) => {
                        let latency_ms = start.elapsed().as_millis() as u64;
                        return Ok(GenerateResult {
                            content: result,
                            provider: provider.name().to_string(),
                            model: provider.model().to_string(),
                            latency_ms,
                            fallback_used: i > 0,
                            fallback_from,
                            tokens_used: None,
                        });
                    }
                    Err(e) => {
                        if i > 0 {
                            fallback_from = Some(
                                self.providers
                                    .get(*provider_idx)
                                    .map(|p| p.name().to_string())
                                    .unwrap_or_else(|| "unknown".to_string()),
                            );
                        }
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Get the current provider
    pub fn current_provider(&self) -> Option<&dyn LlmProvider> {
        self.providers
            .get(self.current_provider)
            .map(|p| p.as_ref())
    }

    /// Set the current provider index
    pub fn set_current_provider(&mut self, idx: usize) -> Result<()> {
        if idx >= self.providers.len() {
            anyhow::bail!("Provider index out of bounds: {}", idx);
        }
        self.current_provider = idx;
        Ok(())
    }
}
