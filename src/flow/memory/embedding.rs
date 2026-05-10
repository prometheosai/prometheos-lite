//! Embedding providers for semantic search

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

/// Trait for embedding providers
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding for the given text
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Get the dimension of the embedding vectors
    fn dimension(&self) -> usize;
}

/// Local embedding provider using a local embedding server
pub struct LocalEmbeddingProvider {
    client: Client,
    url: String,
    dimension: usize,
}

impl LocalEmbeddingProvider {
    /// Create a new local embedding provider
    pub fn new(url: String, dimension: usize) -> Self {
        Self {
            client: Client::new(),
            url,
            dimension,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let json_body = serde_json::json!({ "text": text });
        let response = self
            .client
            .post(&self.url)
            .json(&json_body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send embedding request: {}", e))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Embedding request failed with status: {}",
                response.status()
            );
        }

        let output: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse embedding response: {}", e))?;

        let embedding = output["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing embedding in response"))?
            .iter()
            .map(|v| {
                v.as_f64()
                    .ok_or_else(|| anyhow::anyhow!("Invalid embedding value"))
                    .map(|f| f as f32)
            })
            .collect::<Result<Vec<f32>>>()?;

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Fallback embedding provider that tries multiple providers in order
pub struct FallbackEmbeddingProvider {
    providers: Vec<Box<dyn EmbeddingProvider>>,
}

impl FallbackEmbeddingProvider {
    /// Create a new fallback embedding provider
    pub fn new(providers: Vec<Box<dyn EmbeddingProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl EmbeddingProvider for FallbackEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        for provider in &self.providers {
            match provider.embed(text).await {
                Ok(embedding) => return Ok(embedding),
                Err(_) => continue,
            }
        }
        anyhow::bail!("All embedding providers failed")
    }

    fn dimension(&self) -> usize {
        self.providers.first().map(|p| p.dimension()).unwrap_or(0)
    }
}

/// OpenRouter embedding provider with free tier fallback
pub struct OpenRouterEmbeddingProvider {
    client: Client,
    api_key: String,
    models: Vec<String>,
    current_model_index: usize,
    dimension: usize,
}

impl OpenRouterEmbeddingProvider {
    /// Create a new OpenRouter embedding provider with fallback models
    pub fn new(api_key: String, dimension: usize) -> Self {
        // List of embedding models in order of preference
        // Start with paid models, fallback to free tier
        let models = vec![
            "openai/text-embedding-3-large".to_string(),
            "openai/text-embedding-3-small".to_string(),
            "openai/text-embedding-ada-002".to_string(),
            "cohere/embed-english-v3.0".to_string(),
            "cohere/embed-multilingual-v3.0".to_string(),
        ];

        Self {
            client: Client::new(),
            api_key,
            models,
            current_model_index: 0,
            dimension,
        }
    }

    /// Try the next model in the fallback list
    fn try_next_model(&mut self) -> bool {
        if self.current_model_index + 1 < self.models.len() {
            self.current_model_index += 1;
            true
        } else {
            false
        }
    }

    /// Reset to the first model (for retrying)
    fn reset_model(&mut self) {
        self.current_model_index = 0;
    }
}

#[async_trait]
impl EmbeddingProvider for OpenRouterEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut provider = self.clone();
        provider.reset_model();

        loop {
            let current_model = &provider.models[provider.current_model_index];

            match provider.embed_with_model(text, current_model).await {
                Ok(embedding) => return Ok(embedding),
                Err(e) => {
                    tracing::warn!("Failed to embed with model {}: {}", current_model, e);

                    if !provider.try_next_model() {
                        anyhow::bail!("All embedding models failed. Last error: {}", e);
                    }

                    tracing::info!(
                        "Falling back to next model: {}",
                        provider.models[provider.current_model_index]
                    );
                }
            }
        }
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

impl OpenRouterEmbeddingProvider {
    async fn embed_with_model(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let json_body = serde_json::json!({
            "model": model,
            "input": text
        });

        let response = self
            .client
            .post("https://openrouter.ai/api/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://prometheos.ai")
            .header("X-Title", "PrometheOS Lite")
            .json(&json_body)
            .send()
            .await
            .map_err(|e| {
                anyhow::anyhow!("Failed to send embedding request to OpenRouter: {}", e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenRouter embedding request failed with status {}: {}",
                status,
                error_text
            );
        }

        let output: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse OpenRouter embedding response: {}", e))?;

        // OpenRouter returns embeddings in a different format than OpenAI
        let embeddings = output["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing data array in OpenRouter response"))?;

        if embeddings.is_empty() {
            anyhow::bail!("No embeddings returned from OpenRouter");
        }

        let embedding = embeddings[0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing embedding in OpenRouter response"))?
            .iter()
            .map(|v| {
                v.as_f64()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Invalid embedding value in OpenRouter response")
                    })
                    .map(|f| f as f32)
            })
            .collect::<Result<Vec<f32>>>()?;

        Ok(embedding)
    }
}

impl Clone for OpenRouterEmbeddingProvider {
    fn clone(&self) -> Self {
        Self {
            client: Client::new(),
            api_key: self.api_key.clone(),
            models: self.models.clone(),
            current_model_index: self.current_model_index,
            dimension: self.dimension,
        }
    }
}
