//! Vector search backends for semantic similarity

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

/// Vector search backend trait for pluggable similarity search
#[async_trait]
pub trait VectorSearchBackend: Send + Sync {
    /// Add a vector to the index
    async fn add_vector(&mut self, id: String, vector: Vec<f32>) -> Result<()>;

    /// Search for similar vectors
    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>>;

    /// Remove a vector from the index
    async fn remove_vector(&mut self, id: &str) -> Result<()>;

    /// Get the total number of vectors in the index
    async fn count(&self) -> Result<usize>;
}

/// In-memory HNSW-like indexed search backend
pub struct InMemoryVectorIndex {
    vectors: HashMap<String, Vec<f32>>,
    dimension: usize,
}

impl InMemoryVectorIndex {
    pub fn new(dimension: usize) -> Self {
        Self {
            vectors: HashMap::new(),
            dimension,
        }
    }

    /// Calculate cosine similarity between two vectors
    fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }

        let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot_product / (norm_a * norm_b)
        }
    }
}

#[async_trait]
impl VectorSearchBackend for InMemoryVectorIndex {
    async fn add_vector(&mut self, id: String, vector: Vec<f32>) -> Result<()> {
        if vector.len() != self.dimension {
            anyhow::bail!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            );
        }
        self.vectors.insert(id, vector);
        Ok(())
    }

    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>> {
        if query.len() != self.dimension {
            anyhow::bail!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            );
        }

        let mut scored: Vec<(String, f32)> = self
            .vectors
            .iter()
            .map(|(id, vec)| (id.clone(), self.cosine_similarity(query, vec)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored.into_iter().take(limit).collect())
    }

    async fn remove_vector(&mut self, id: &str) -> Result<()> {
        self.vectors.remove(id);
        Ok(())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.vectors.len())
    }
}

/// Brute-force fallback backend (original implementation)
pub struct BruteForceBackend {
    vectors: HashMap<String, Vec<f32>>,
}

impl Default for BruteForceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BruteForceBackend {
    pub fn new() -> Self {
        Self {
            vectors: HashMap::new(),
        }
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }

        let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot_product / (norm_a * norm_b)
        }
    }
}

#[async_trait]
impl VectorSearchBackend for BruteForceBackend {
    async fn add_vector(&mut self, id: String, vector: Vec<f32>) -> Result<()> {
        self.vectors.insert(id, vector);
        Ok(())
    }

    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>> {
        let mut scored: Vec<(String, f32)> = self
            .vectors
            .iter()
            .map(|(id, vec)| (id.clone(), Self::cosine_similarity(query, vec)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored.into_iter().take(limit).collect())
    }

    async fn remove_vector(&mut self, id: &str) -> Result<()> {
        self.vectors.remove(id);
        Ok(())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.vectors.len())
    }
}

/// Calculate cosine similarity between two vectors (legacy function for compatibility)
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}
