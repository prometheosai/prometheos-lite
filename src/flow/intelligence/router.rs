//! Model router for selecting and routing to different LLM providers

use anyhow::Result;
use super::provider::LlmProvider;

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
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;

        for provider_idx in providers_to_try {
            if let Some(provider) = self.providers.get(provider_idx) {
                match provider.generate(prompt).await {
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

    /// Generate a completion with streaming support
    pub async fn generate_stream(&self, prompt: &str, callback: super::provider::StreamCallback) -> Result<String> {
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
