//! LLM Provider abstraction

use async_trait::async_trait;
use std::sync::Arc;
use anyhow::Result;

use crate::llm::LlmClient;

/// Streaming callback type
pub type StreamCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// Provider-level error category used by ModelRouter for failover policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Quota,
    RateLimit,
    Transient,
    Fatal,
}

/// A normalized generation error with policy metadata.
#[derive(Debug, Clone)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn quota(msg: impl Into<String>) -> Self {
        Self {
            kind: ProviderErrorKind::Quota,
            message: msg.into(),
        }
    }

    pub fn rate_limit(msg: impl Into<String>) -> Self {
        Self {
            kind: ProviderErrorKind::RateLimit,
            message: msg.into(),
        }
    }

    pub fn transient(msg: impl Into<String>) -> Self {
        Self {
            kind: ProviderErrorKind::Transient,
            message: msg.into(),
        }
    }

    pub fn fatal(msg: impl Into<String>) -> Self {
        Self {
            kind: ProviderErrorKind::Fatal,
            message: msg.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenRouter,
    OpenAi,
    Anthropic,
    Ollama,
    LmStudio,
    GenericOpenAiCompatible,
}

#[derive(Debug, Clone)]
pub struct ProviderMetadata {
    pub kind: ProviderKind,
    pub supports_streaming: bool,
    pub local: bool,
}

/// LLM Provider trait for provider abstraction
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a completion from the given prompt.
    async fn generate(&self, prompt: &str) -> Result<String>;

    /// Generate a completion with streaming support.
    async fn generate_stream(
        &self,
        prompt: &str,
        callback: StreamCallback,
    ) -> Result<String>;

    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the model name
    fn model(&self) -> &str;

    /// Get normalized provider metadata.
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            kind: ProviderKind::GenericOpenAiCompatible,
            supports_streaming: true,
            local: false,
        }
    }

    /// Classify provider errors for router policy decisions.
    fn classify_error(&self, err: &anyhow::Error) -> ProviderErrorKind {
        classify_error(anyhow::anyhow!(err.to_string())).kind
    }
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

pub struct OpenRouterProvider {
    inner: OpenAiProvider,
}

impl OpenRouterProvider {
    pub fn new(client: LlmClient) -> Self {
        Self {
            inner: OpenAiProvider::new(client).with_name("openrouter".to_string()),
        }
    }
}

pub struct AnthropicProvider {
    inner: OpenAiProvider,
}

impl AnthropicProvider {
    pub fn new(client: LlmClient) -> Self {
        Self {
            inner: OpenAiProvider::new(client).with_name("anthropic".to_string()),
        }
    }
}

pub struct OllamaProvider {
    inner: OpenAiProvider,
}

impl OllamaProvider {
    pub fn new(client: LlmClient) -> Self {
        Self {
            inner: OpenAiProvider::new(client).with_name("ollama".to_string()),
        }
    }
}

pub struct LmStudioProvider {
    inner: OpenAiProvider,
}

impl LmStudioProvider {
    pub fn new(client: LlmClient) -> Self {
        Self {
            inner: OpenAiProvider::new(client).with_name("lmstudio".to_string()),
        }
    }
}

pub struct GenericOpenAiCompatibleProvider {
    inner: OpenAiProvider,
}

impl GenericOpenAiCompatibleProvider {
    pub fn new(client: LlmClient, name: String) -> Self {
        Self {
            inner: OpenAiProvider::new(client).with_name(name),
        }
    }
}

fn classify_error(e: anyhow::Error) -> ProviderError {
    let message = e.to_string();
    let lower = message.to_lowercase();
    if lower.contains("quota") || lower.contains("insufficient credits") {
        return ProviderError::quota(message);
    }
    if lower.contains("rate limit") || lower.contains("429") {
        return ProviderError::rate_limit(message);
    }
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("temporar")
        || lower.contains("connection")
        || lower.contains("network")
    {
        return ProviderError::transient(message);
    }
    ProviderError::fatal(message)
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, prompt: &str) -> Result<String> {
        self.client.generate(prompt).await
    }

    async fn generate_stream(
        &self,
        prompt: &str,
        callback: StreamCallback,
    ) -> Result<String> {
        self.client
            .generate_stream(prompt, |chunk| callback(chunk))
            .await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        self.client.model()
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            kind: ProviderKind::OpenAi,
            supports_streaming: true,
            local: false,
        }
    }
}

macro_rules! impl_wrapped_provider {
    ($t:ty, $kind:expr, $local:expr) => {
        #[async_trait]
        impl LlmProvider for $t {
            async fn generate(&self, prompt: &str) -> Result<String> {
                self.inner.generate(prompt).await
            }

            async fn generate_stream(
                &self,
                prompt: &str,
                callback: StreamCallback,
            ) -> Result<String> {
                self.inner.generate_stream(prompt, callback).await
            }

            fn name(&self) -> &str {
                self.inner.name()
            }

            fn model(&self) -> &str {
                self.inner.model()
            }

            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    kind: $kind,
                    supports_streaming: true,
                    local: $local,
                }
            }
        }
    };
}

impl_wrapped_provider!(OpenRouterProvider, ProviderKind::OpenRouter, false);
impl_wrapped_provider!(AnthropicProvider, ProviderKind::Anthropic, false);
impl_wrapped_provider!(OllamaProvider, ProviderKind::Ollama, true);
impl_wrapped_provider!(LmStudioProvider, ProviderKind::LmStudio, true);
impl_wrapped_provider!(
    GenericOpenAiCompatibleProvider,
    ProviderKind::GenericOpenAiCompatible,
    false
);
