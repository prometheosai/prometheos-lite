//! Model router for selecting and routing to different LLM providers

use super::provider::{LlmProvider, ProviderErrorKind};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmMode {
    Fast,
    Balanced,
    Deep,
    Coding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptMetadata {
    pub provider: String,
    pub model: String,
    pub failure_category: Option<String>,
}

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
    pub attempted_path: Vec<AttemptMetadata>,
    pub fallback_count: usize,
    pub quota_rotation_used: bool,
}

#[derive(Debug, Clone)]
struct ProviderHealth {
    cooldown_until: Option<Instant>,
}

impl ProviderHealth {
    fn new() -> Self {
        Self {
            cooldown_until: None,
        }
    }

    fn is_available(&self) -> bool {
        match self.cooldown_until {
            Some(until) => Instant::now() >= until,
            None => true,
        }
    }
}

/// Model router for selecting and routing to different LLM providers
pub struct ModelRouter {
    providers: Vec<Box<dyn LlmProvider>>,
    mode_chains: HashMap<LlmMode, Vec<usize>>,
    cooldown_ttl: Duration,
    provider_health: std::sync::Mutex<HashMap<usize, ProviderHealth>>,
}

impl ModelRouter {
    /// Create a new ModelRouter with a list of providers.
    /// Defaults all modes to all providers in order.
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        let default_chain: Vec<usize> = (0..providers.len()).collect();
        let mut mode_chains = HashMap::new();
        mode_chains.insert(LlmMode::Fast, default_chain.clone());
        mode_chains.insert(LlmMode::Balanced, default_chain.clone());
        mode_chains.insert(LlmMode::Deep, default_chain.clone());
        mode_chains.insert(LlmMode::Coding, default_chain);

        let mut provider_health = HashMap::new();
        for i in 0..providers.len() {
            provider_health.insert(i, ProviderHealth::new());
        }

        Self {
            providers,
            mode_chains,
            cooldown_ttl: Duration::from_secs(60),
            provider_health: std::sync::Mutex::new(provider_health),
        }
    }

    pub fn with_mode_chain(mut self, mode: LlmMode, chain: Vec<usize>) -> Self {
        self.mode_chains.insert(mode, chain);
        self
    }

    pub fn with_cooldown_ttl(mut self, ttl: Duration) -> Self {
        self.cooldown_ttl = ttl;
        self
    }

    /// Backward-compatible wrapper: defaults to `balanced` mode.
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        self.generate_for_mode(LlmMode::Balanced, prompt).await
    }

    /// Backward-compatible wrapper: defaults to `balanced` mode.
    pub async fn generate_with_metadata(&self, prompt: &str) -> Result<GenerateResult> {
        self.generate_for_mode_with_metadata(LlmMode::Balanced, prompt)
            .await
    }

    pub async fn generate_for_mode(&self, mode: LlmMode, prompt: &str) -> Result<String> {
        let result = self.generate_for_mode_with_metadata(mode, prompt).await?;
        Ok(result.content)
    }

    pub async fn generate_for_mode_with_metadata(
        &self,
        mode: LlmMode,
        prompt: &str,
    ) -> Result<GenerateResult> {
        let chain = self
            .mode_chains
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| (0..self.providers.len()).collect());

        let mut last_error = None;
        let mut attempted_path = Vec::new();
        let mut fallback_from = None;
        let mut fallback_count = 0usize;
        let mut quota_rotation_used = false;

        for (i, provider_idx) in chain.iter().enumerate() {
            if !self.provider_is_available(*provider_idx) {
                continue;
            }

            let Some(provider) = self.providers.get(*provider_idx) else {
                continue;
            };

            let start = Instant::now();
            match provider.generate(prompt).await {
                Ok(result) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    attempted_path.push(AttemptMetadata {
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        failure_category: None,
                    });
                    return Ok(GenerateResult {
                        content: result,
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        latency_ms,
                        fallback_used: i > 0,
                        fallback_from,
                        tokens_used: None,
                        attempted_path,
                        fallback_count,
                        quota_rotation_used,
                    });
                }
                Err(err) => {
                    fallback_count += 1;
                    if fallback_from.is_none() {
                        fallback_from = Some(provider.name().to_string());
                    }
                    let err_kind = provider.classify_error(&err);
                    let category = match err_kind {
                        ProviderErrorKind::Quota => {
                            self.mark_cooldown(*provider_idx);
                            quota_rotation_used = true;
                            "quota"
                        }
                        ProviderErrorKind::RateLimit => {
                            self.mark_cooldown(*provider_idx);
                            quota_rotation_used = true;
                            "rate_limit"
                        }
                        ProviderErrorKind::Transient => "transient",
                        ProviderErrorKind::Fatal => "fatal",
                    }
                    .to_string();

                    attempted_path.push(AttemptMetadata {
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        failure_category: Some(category),
                    });

                    last_error = Some(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Backward-compatible wrapper: defaults to `balanced` mode.
    pub async fn generate_stream(
        &self,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<String> {
        self.generate_stream_for_mode(LlmMode::Balanced, prompt, callback)
            .await
    }

    pub async fn generate_stream_for_mode(
        &self,
        mode: LlmMode,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<String> {
        let chain = self
            .mode_chains
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| (0..self.providers.len()).collect());

        let mut last_error = None;

        for provider_idx in chain {
            if !self.provider_is_available(provider_idx) {
                continue;
            }

            if let Some(provider) = self.providers.get(provider_idx) {
                match provider.generate_stream(prompt, callback.clone()).await {
                    Ok(result) => return Ok(result),
                    Err(err) => {
                        let err_kind = provider.classify_error(&err);
                        if matches!(err_kind, ProviderErrorKind::Quota | ProviderErrorKind::RateLimit)
                        {
                            self.mark_cooldown(provider_idx);
                        }
                        last_error = Some(err);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    pub async fn generate_stream_with_metadata(
        &self,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<GenerateResult> {
        self.generate_stream_for_mode_with_metadata(LlmMode::Balanced, prompt, callback)
            .await
    }

    pub async fn generate_stream_for_mode_with_metadata(
        &self,
        mode: LlmMode,
        prompt: &str,
        callback: super::provider::StreamCallback,
    ) -> Result<GenerateResult> {
        let chain = self
            .mode_chains
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| (0..self.providers.len()).collect());

        let mut last_error = None;
        let mut attempted_path = Vec::new();
        let mut fallback_from = None;
        let mut fallback_count = 0usize;
        let mut quota_rotation_used = false;

        for (i, provider_idx) in chain.iter().enumerate() {
            if !self.provider_is_available(*provider_idx) {
                continue;
            }

            let Some(provider) = self.providers.get(*provider_idx) else {
                continue;
            };

            let start = Instant::now();
            match provider.generate_stream(prompt, callback.clone()).await {
                Ok(content) => {
                    attempted_path.push(AttemptMetadata {
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        failure_category: None,
                    });
                    return Ok(GenerateResult {
                        content,
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        latency_ms: start.elapsed().as_millis() as u64,
                        fallback_used: i > 0,
                        fallback_from,
                        tokens_used: None,
                        attempted_path,
                        fallback_count,
                        quota_rotation_used,
                    });
                }
                Err(err) => {
                    fallback_count += 1;
                    if fallback_from.is_none() {
                        fallback_from = Some(provider.name().to_string());
                    }

                    let err_kind = provider.classify_error(&err);
                    let category = match err_kind {
                        ProviderErrorKind::Quota => {
                            self.mark_cooldown(*provider_idx);
                            quota_rotation_used = true;
                            "quota"
                        }
                        ProviderErrorKind::RateLimit => {
                            self.mark_cooldown(*provider_idx);
                            quota_rotation_used = true;
                            "rate_limit"
                        }
                        ProviderErrorKind::Transient => "transient",
                        ProviderErrorKind::Fatal => "fatal",
                    }
                    .to_string();

                    attempted_path.push(AttemptMetadata {
                        provider: provider.name().to_string(),
                        model: provider.model().to_string(),
                        failure_category: Some(category),
                    });
                    last_error = Some(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    fn provider_is_available(&self, idx: usize) -> bool {
        let Ok(health) = self.provider_health.lock() else {
            return true;
        };
        health.get(&idx).map(|h| h.is_available()).unwrap_or(true)
    }

    fn mark_cooldown(&self, idx: usize) {
        if let Ok(mut health) = self.provider_health.lock() {
            let entry = health.entry(idx).or_insert_with(ProviderHealth::new);
            entry.cooldown_until = Some(Instant::now() + self.cooldown_ttl);
        }
    }
}
