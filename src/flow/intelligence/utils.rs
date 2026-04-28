//! LLM Utilities - unified interface for LLM operations

use anyhow::Result;
use std::sync::Arc;

use super::router::{GenerateResult, ModelRouter};

/// LLM Utilities - unified interface for LLM operations
pub struct LlmUtilities {
    router: ModelRouter,
}

impl LlmUtilities {
    pub fn new(router: ModelRouter) -> Self {
        Self { router }
    }

    /// Unified call with automatic retry
    pub async fn call_with_retry(
        &self,
        prompt: &str,
        max_retries: u32,
        initial_delay_ms: u64,
    ) -> Result<String> {
        let mut last_error = None;

        for attempt in 0..=max_retries {
            match self.router.generate(prompt).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries {
                        let delay = initial_delay_ms * 2_u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM call failed")))
    }

    /// Streaming call with automatic retry
    pub async fn call_stream_with_retry<F>(
        &self,
        prompt: &str,
        callback: F,
        max_retries: u32,
        initial_delay_ms: u64,
    ) -> Result<String>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        let mut last_error = None;

        for attempt in 0..=max_retries {
            match self.router.generate_stream(prompt, callback.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries {
                        let delay = initial_delay_ms * 2_u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM call failed")))
    }

    /// Simple call without retry
    pub async fn call(&self, prompt: &str) -> Result<String> {
        self.router.generate(prompt).await
    }

    /// Streaming call without retry
    pub async fn call_stream<F>(&self, prompt: &str, callback: F) -> Result<String>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        self.router.generate_stream(prompt, callback).await
    }

    /// Call with metadata (provider, model, latency, fallback info)
    pub async fn call_with_metadata(&self, prompt: &str) -> Result<GenerateResult> {
        self.router.generate_with_metadata(prompt).await
    }

    /// Streaming call with metadata
    pub async fn call_stream_with_metadata<F>(&self, prompt: &str, callback: F) -> Result<GenerateResult>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        self.router.generate_stream_with_metadata(prompt, callback).await
    }
}
