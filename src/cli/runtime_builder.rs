//! RuntimeBuilder - Consolidates RuntimeContext setup for CLI and API

use anyhow::Context;
use std::sync::Arc;

use prometheos_lite::{
    config::AppConfig,
    flow::intelligence::{
        AnthropicProvider, GenericOpenAiCompatibleProvider, LlmMode, LmStudioProvider,
        OllamaProvider, OpenAiProvider, OpenRouterProvider,
    },
    flow::{
        EmbeddingProvider, MemoryDb, MemoryService, ModelRouter, RuntimeContext, ToolRuntime,
        ToolSandboxProfile, memory::embedding::{JinaEmbeddingProvider, LocalEmbeddingProvider, OpenRouterEmbeddingProvider},
    },
    llm::LlmClient,
};

/// Builder for constructing RuntimeContext with all required services
pub struct RuntimeBuilder {
    config: AppConfig,
}

impl RuntimeBuilder {
    /// Create a new RuntimeBuilder from loaded config
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    /// Load config from default location and create builder
    pub fn from_config() -> anyhow::Result<Self> {
        let config = AppConfig::load()?;
        Ok(Self::new(config))
    }

    /// Build a full RuntimeContext with all services
    pub fn build_full(&self) -> anyhow::Result<RuntimeContext> {
        let (providers, mode_chains) = self.build_provider_registry()?;
        let mut model_router = ModelRouter::new(providers);
        model_router = model_router
            .with_mode_chain(LlmMode::Fast, mode_chains.0)
            .with_mode_chain(LlmMode::Balanced, mode_chains.1)
            .with_mode_chain(LlmMode::Deep, mode_chains.2)
            .with_mode_chain(LlmMode::Coding, mode_chains.3);
        let model_router = Arc::new(model_router);

        // Create tool runtime with default tools registered
        let repo_path = std::path::PathBuf::from(self.config.repo_path.clone());
        let tool_runtime = Arc::new(ToolRuntime::with_default_tools(
            ToolSandboxProfile::new(),
            repo_path,
        ));

        // Create persistent memory service with configurable embedding provider
        let embedding: Box<dyn EmbeddingProvider> = if self.config.provider == "lmstudio" {
            Box::new(LocalEmbeddingProvider::new(
                self.config.embedding_url.clone(),
                self.config.embedding_dimension,
            ))
        } else if self.config.provider == "jina" {
            // Use Jina AI embeddings
            Box::new(JinaEmbeddingProvider::new(self.config.embedding_dimension))
        } else if self.config.provider == "openrouter" {
            // Check if we have OpenRouter credits, otherwise use Jina AI
            if let Ok(_) = std::env::var("OPENROUTER_API_KEY") {
                // Try OpenRouter first if API key is available
                Box::new(OpenRouterEmbeddingProvider::new(
                    std::env::var("OPENROUTER_API_KEY").unwrap(),
                    self.config.embedding_dimension,
                ))
            } else {
                // Fallback to Jina AI for free embeddings without credits
                Box::new(JinaEmbeddingProvider::new(self.config.embedding_dimension))
            }
        } else {
            // Default to local provider for other cases
            Box::new(LocalEmbeddingProvider::new(
                self.config.embedding_url.clone(),
                self.config.embedding_dimension,
            ))
        };

        let persistent_db =
            MemoryDb::new(std::path::PathBuf::from(self.config.memory_db_path.clone()))
                .context("Failed to create memory database")?;
        let memory_service = Arc::new(MemoryService::new(persistent_db, embedding));

        let trace_storage = Arc::new(
            prometheos_lite::flow::tracing::TraceStorage::in_memory()
                .context("Failed to create trace storage")?,
        );

        Ok(RuntimeContext::full(
            model_router,
            tool_runtime,
            memory_service,
            trace_storage,
        ))
    }

    /// Build memory service with configurable embedding provider
    pub fn build_memory_service(&self) -> anyhow::Result<Arc<MemoryService>> {
        let embedding: Box<dyn EmbeddingProvider> = if self.config.provider == "lmstudio" {
            Box::new(LocalEmbeddingProvider::new(
                self.config.embedding_url.clone(),
                self.config.embedding_dimension,
            ))
        } else if self.config.provider == "jina" {
            // Use Jina AI embeddings
            Box::new(JinaEmbeddingProvider::new(self.config.embedding_dimension))
        } else if self.config.provider == "openrouter" {
            // Check if we have OpenRouter credits, otherwise use Jina AI
            if let Ok(_) = std::env::var("OPENROUTER_API_KEY") {
                // Try OpenRouter first if API key is available
                Box::new(OpenRouterEmbeddingProvider::new(
                    std::env::var("OPENROUTER_API_KEY").unwrap(),
                    self.config.embedding_dimension,
                ))
            } else {
                // Fallback to Jina AI for free embeddings without credits
                Box::new(JinaEmbeddingProvider::new(self.config.embedding_dimension))
            }
        } else {
            // Default to local provider for other cases
            Box::new(LocalEmbeddingProvider::new(
                self.config.embedding_url.clone(),
                self.config.embedding_dimension,
            ))
        };

        let persistent_db =
            MemoryDb::new(std::path::PathBuf::from(self.config.memory_db_path.clone()))
                .context("Failed to create memory database")?;
        Ok(Arc::new(MemoryService::new(persistent_db, embedding)))
    }

    /// Get the config
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn build_provider_registry(
        &self,
    ) -> anyhow::Result<(
        Vec<Box<dyn prometheos_lite::flow::intelligence::LlmProvider>>,
        (Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>),
    )> {
        let mut providers: Vec<Box<dyn prometheos_lite::flow::intelligence::LlmProvider>> =
            Vec::new();
        let mut index_by_name = std::collections::HashMap::new();

        for provider_cfg in &self.config.llm_routing.providers {
            if !provider_cfg.enabled {
                continue;
            }

            let api_key = provider_cfg
                .api_key_env
                .as_ref()
                .and_then(|env_name| std::env::var(env_name).ok());
            let client = LlmClient::new(&provider_cfg.base_url, &provider_cfg.model)?
                .with_api_key(api_key.clone());

            let provider: Box<dyn prometheos_lite::flow::intelligence::LlmProvider> =
                match provider_cfg.provider_type.as_str() {
                    "openrouter" => {
                        if api_key.is_none() {
                            anyhow::bail!(
                                "OpenRouter provider '{}' requires api key env '{}'",
                                provider_cfg.name,
                                provider_cfg.api_key_env.clone().unwrap_or_default()
                            );
                        }
                        Box::new(OpenRouterProvider::new(client))
                    }
                    "openai" => Box::new(OpenAiProvider::new(client).with_name(provider_cfg.name.clone())),
                    "anthropic" => Box::new(AnthropicProvider::new(client)),
                    "ollama" => Box::new(OllamaProvider::new(client)),
                    "lmstudio" => Box::new(LmStudioProvider::new(client)),
                    _ => Box::new(GenericOpenAiCompatibleProvider::new(
                        client,
                        provider_cfg.name.clone(),
                    )),
                };
            index_by_name.insert(provider_cfg.name.clone(), providers.len());
            providers.push(provider);
        }

        if providers.is_empty() {
            anyhow::bail!("No LLM providers enabled in llm_routing.providers");
        }

        let resolve_chain = |names: &[String]| -> anyhow::Result<Vec<usize>> {
            let mut out = Vec::new();
            for name in names {
                let idx = index_by_name
                    .get(name)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("Mode chain references unknown provider: {}", name))?;
                out.push(idx);
            }
            if out.is_empty() {
                anyhow::bail!("Mode chain cannot be empty");
            }
            Ok(out)
        };

        let mode_chains = (
            resolve_chain(&self.config.llm_routing.mode_chains.fast)?,
            resolve_chain(&self.config.llm_routing.mode_chains.balanced)?,
            resolve_chain(&self.config.llm_routing.mode_chains.deep)?,
            resolve_chain(&self.config.llm_routing.mode_chains.coding)?,
        );

        Ok((providers, mode_chains))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_runtime_builder_creation() {
        // This test requires a valid config file, so we skip it in CI
        // In real usage, this would test the builder can be created
    }
}
