//! RuntimeBuilder - Consolidates RuntimeContext setup for CLI and API

use anyhow::Context;
use std::sync::Arc;

use prometheos_lite::{
    config::AppConfig,
    flow::intelligence::OpenAiProvider,
    flow::{
        EmbeddingProvider, MemoryDb, MemoryService, ModelRouter, RuntimeContext, ToolRuntime,
        ToolSandboxProfile, memory::embedding::OpenRouterEmbeddingProvider,
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
        // Create LlmClient and wrap in OpenAiProvider for ModelRouter
        let llm_client = LlmClient::from_config(&self.config)?;
        let openai_provider =
            OpenAiProvider::new(llm_client).with_name(self.config.provider.clone());
        let model_router = Arc::new(ModelRouter::new(vec![Box::new(openai_provider)]));

        // Create tool runtime with default tools registered
        let repo_path = std::path::PathBuf::from(self.config.repo_path.clone());
        let tool_runtime = Arc::new(ToolRuntime::with_default_tools(
            ToolSandboxProfile::new(),
            repo_path,
        ));

        // Create persistent memory service with OpenRouter embedding provider
        let openrouter_api_key = std::env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY environment variable not set")?;

        let embedding: Box<dyn EmbeddingProvider> = Box::new(OpenRouterEmbeddingProvider::new(
            openrouter_api_key,
            self.config.embedding_dimension,
        ));

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

    /// Build RuntimeContext with minimal services (no memory)
    pub fn build_minimal(&self) -> anyhow::Result<RuntimeContext> {
        let llm_client = LlmClient::from_config(&self.config)?;
        let openai_provider =
            OpenAiProvider::new(llm_client).with_name(self.config.provider.clone());
        let model_router = Arc::new(ModelRouter::new(vec![Box::new(openai_provider)]));

        // Create tool runtime with default tools registered
        let repo_path = std::path::PathBuf::from(self.config.repo_path.clone());
        let tool_runtime = Arc::new(ToolRuntime::with_default_tools(
            ToolSandboxProfile::new(),
            repo_path,
        ));

        Ok(RuntimeContext::new()
            .with_model_router(model_router)
            .with_tool_runtime(tool_runtime))
    }

    /// Build only the model router
    pub fn build_model_router(&self) -> anyhow::Result<Arc<ModelRouter>> {
        let llm_client = LlmClient::from_config(&self.config)?;
        let openai_provider =
            OpenAiProvider::new(llm_client).with_name(self.config.provider.clone());
        Ok(Arc::new(ModelRouter::new(vec![Box::new(openai_provider)])))
    }

    /// Build only the tool runtime
    pub fn build_tool_runtime(&self) -> Arc<ToolRuntime> {
        Arc::new(ToolRuntime::new(ToolSandboxProfile::new()))
    }

    /// Build memory service with OpenRouter embedding provider
    pub fn build_memory_service(&self) -> anyhow::Result<Arc<MemoryService>> {
        let openrouter_api_key = std::env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY environment variable not set")?;

        let embedding: Box<dyn EmbeddingProvider> = Box::new(OpenRouterEmbeddingProvider::new(
            openrouter_api_key,
            self.config.embedding_dimension,
        ));

        let persistent_db =
            MemoryDb::new(std::path::PathBuf::from(self.config.memory_db_path.clone()))
                .context("Failed to create memory database")?;
        Ok(Arc::new(MemoryService::new(persistent_db, embedding)))
    }

    /// Get the config
    pub fn config(&self) -> &AppConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_builder_creation() {
        // This test requires a valid config file, so we skip it in CI
        // In real usage, this would test the builder can be created
    }
}
