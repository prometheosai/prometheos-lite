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
