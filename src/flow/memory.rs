//! Memory layer - SQLite-based storage with semantic search and embedding support

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::flow::{Action, Input, Node, NodeConfig, Output, SharedState};

/// Memory entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub embedding: Option<Vec<f32>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Memory type classification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    Working,
}

/// Memory relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRelationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relationship_type: String,
    pub strength: f32,
    pub created_at: DateTime<Utc>,
}

/// SQLite database manager for memory storage
pub struct MemoryDb {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryDb {
    /// Create or open a memory database at the given path
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at: {}", db_path.display()))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database for testing
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to create in-memory database")?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                embedding BLOB,
                metadata TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )
        .context("Failed to create memories table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS relationships (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                relationship_type TEXT NOT NULL,
                strength REAL NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (source_id) REFERENCES memories(id),
                FOREIGN KEY (target_id) REFERENCES memories(id)
            )",
            [],
        )
        .context("Failed to create relationships table")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_type ON memories(memory_type)",
            [],
        )
        .context("Failed to create memory_type index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_relationship_source ON relationships(source_id)",
            [],
        )
        .context("Failed to create relationship_source index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_relationship_target ON relationships(target_id)",
            [],
        )
        .context("Failed to create relationship_target index")?;

        Ok(())
    }

    /// Create a new memory entry
    pub fn create_memory(&self, memory: &Memory) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let embedding_blob = memory.embedding.as_ref().map(|e| {
            e.iter()
                .flat_map(|&f| f.to_le_bytes().to_vec())
                .collect::<Vec<u8>>()
        });

        let metadata_json =
            serde_json::to_string(&memory.metadata).context("Failed to serialize metadata")?;

        let memory_type_str = match memory.memory_type {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Procedural => "procedural",
            MemoryType::Working => "working",
        };

        conn.execute(
            "INSERT INTO memories (id, content, memory_type, embedding, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                memory.id,
                memory.content,
                memory_type_str,
                embedding_blob,
                metadata_json,
                memory.created_at.to_rfc3339(),
                memory.updated_at.to_rfc3339(),
            ],
        ).context("Failed to insert memory")?;

        Ok(())
    }

    /// Get a memory by ID
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, content, memory_type, embedding, metadata, created_at, updated_at
             FROM memories WHERE id = ?1",
            )
            .context("Failed to prepare get_memory query")?;

        let mut rows = stmt.query(params![id]).context("Failed to query memory")?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_memory(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get all memories of a specific type
    pub fn get_memories_by_type(&self, memory_type: MemoryType) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let memory_type_str = match memory_type {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Procedural => "procedural",
            MemoryType::Working => "working",
        };

        let mut stmt = conn
            .prepare(
                "SELECT id, content, memory_type, embedding, metadata, created_at, updated_at
             FROM memories WHERE memory_type = ?1",
            )
            .context("Failed to prepare get_memories_by_type query")?;

        let rows = stmt
            .query_map(params![memory_type_str], |row| self.row_to_memory(row))
            .context("Failed to query memories by type")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Create a relationship between memories
    pub fn create_relationship(&self, relationship: &MemoryRelationship) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "INSERT INTO relationships (id, source_id, target_id, relationship_type, strength, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                relationship.id,
                relationship.source_id,
                relationship.target_id,
                relationship.relationship_type,
                relationship.strength,
                relationship.created_at.to_rfc3339(),
            ],
        ).context("Failed to insert relationship")?;

        Ok(())
    }

    /// Get relationships for a memory
    pub fn get_relationships(&self, memory_id: &str) -> Result<Vec<MemoryRelationship>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, source_id, target_id, relationship_type, strength, created_at
             FROM relationships WHERE source_id = ?1 OR target_id = ?1",
            )
            .context("Failed to prepare get_relationships query")?;

        let rows = stmt
            .query_map(params![memory_id], |row| {
                Ok(MemoryRelationship {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    target_id: row.get(2)?,
                    relationship_type: row.get(3)?,
                    strength: row.get(4)?,
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })
            .context("Failed to query relationships")?;

        let mut relationships = Vec::new();
        for row in rows {
            let rel = row.map_err(|e| anyhow::anyhow!(e))?;
            relationships.push(rel);
        }
        Ok(relationships)
    }

    /// Search memories by content (simple text search)
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, content, memory_type, embedding, metadata, created_at, updated_at
             FROM memories WHERE content LIKE ?1",
            )
            .context("Failed to prepare search_memories query")?;

        let search_pattern = format!("%{}%", query);
        let rows = stmt
            .query_map(params![search_pattern], |row| self.row_to_memory(row))
            .context("Failed to search memories")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Delete a memory
    pub fn delete_memory(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "DELETE FROM relationships WHERE source_id = ?1 OR target_id = ?1",
            params![id],
        )
        .context("Failed to delete relationships")?;

        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])
            .context("Failed to delete memory")?;

        Ok(())
    }

    /// Convert a database row to a Memory struct
    fn row_to_memory(&self, row: &rusqlite::Row) -> std::result::Result<Memory, rusqlite::Error> {
        let memory_type_str: String = row.get(2)?;
        let memory_type = match memory_type_str.as_str() {
            "episodic" => MemoryType::Episodic,
            "semantic" => MemoryType::Semantic,
            "procedural" => MemoryType::Procedural,
            "working" => MemoryType::Working,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };

        let embedding_blob: Option<Vec<u8>> = row.get(3)?;
        let embedding = embedding_blob.map(|blob| {
            blob.chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect()
        });

        let metadata_json: String = row.get(4)?;
        let metadata = serde_json::from_str(&metadata_json)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        Ok(Memory {
            id: row.get(0)?,
            content: row.get(1)?,
            memory_type,
            embedding,
            metadata,
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                .with_timezone(&Utc),
        })
    }
}

/// Embedding provider trait for generating text embeddings
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts (batch)
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::new();
        for text in texts {
            embeddings.push(self.embed(&text).await?);
        }
        Ok(embeddings)
    }

    /// Get the embedding dimension
    fn dimension(&self) -> usize;
}

/// Local HTTP embedding provider (e.g., local embedding server)
pub struct LocalEmbeddingProvider {
    client: Client,
    base_url: String,
    dimension: usize,
}

impl LocalEmbeddingProvider {
    pub fn new(base_url: String, dimension: usize) -> Self {
        Self {
            client: Client::new(),
            base_url,
            dimension,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/embed", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await
            .context("Failed to send embedding request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Embedding request failed with status: {}",
                response.status()
            );
        }

        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse embedding response")?;

        let embedding = result["embedding"]
            .as_array()
            .context("Missing embedding in response")?
            .iter()
            .map(|v| {
                v.as_f64()
                    .context("Invalid embedding value")
                    .map(|f| f as f32)
            })
            .collect::<Result<Vec<f32>>>()?;

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// External API embedding provider (e.g., OpenAI, Cohere)
pub struct ExternalEmbeddingProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    dimension: usize,
}

impl ExternalEmbeddingProvider {
    pub fn new(api_key: String, base_url: String, model: String, dimension: usize) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
            model,
            dimension,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for ExternalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/embeddings", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": self.model,
                "input": text
            }))
            .send()
            .await
            .context("Failed to send embedding request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Embedding request failed with status: {}",
                response.status()
            );
        }

        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse embedding response")?;

        let embedding = result["data"][0]["embedding"]
            .as_array()
            .context("Missing embedding in response")?
            .iter()
            .map(|v| {
                v.as_f64()
                    .context("Invalid embedding value")
                    .map(|f| f as f32)
            })
            .collect::<Result<Vec<f32>>>()?;

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Fallback embedding provider that tries multiple providers
pub struct FallbackEmbeddingProvider {
    providers: Vec<Box<dyn EmbeddingProvider>>,
}

impl FallbackEmbeddingProvider {
    pub fn new(providers: Vec<Box<dyn EmbeddingProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl EmbeddingProvider for FallbackEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut last_error = None;

        for provider in &self.providers {
            match provider.embed(text).await {
                Ok(embedding) => return Ok(embedding),
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All embedding providers failed")))
    }

    fn dimension(&self) -> usize {
        self.providers.first().map(|p| p.dimension()).unwrap_or(0)
    }
}

/// Memory service combining database and embedding provider
pub struct MemoryService {
    db: MemoryDb,
    embedding_provider: Box<dyn EmbeddingProvider>,
    vector_backend: Arc<tokio::sync::Mutex<Box<dyn VectorSearchBackend>>>,
}

impl MemoryService {
    pub fn new(db: MemoryDb, embedding_provider: Box<dyn EmbeddingProvider>) -> Self {
        let vector_backend: Box<dyn VectorSearchBackend> = Box::new(BruteForceBackend::new());
        Self {
            db,
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
        }
    }

    /// Create a MemoryService with a custom vector search backend
    pub fn with_vector_backend(
        db: MemoryDb,
        embedding_provider: Box<dyn EmbeddingProvider>,
        vector_backend: Box<dyn VectorSearchBackend>,
    ) -> Self {
        Self {
            db,
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
        }
    }

    /// Create a memory with automatic embedding generation
    pub async fn create_memory(
        &self,
        content: String,
        memory_type: MemoryType,
        metadata: serde_json::Value,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // Generate embedding
        let embedding = self.embedding_provider.embed(&content).await?;

        // Add to vector index if it's a semantic memory
        if memory_type == MemoryType::Semantic {
            let mut backend = self.vector_backend.lock().await;
            backend.add_vector(id.clone(), embedding.clone()).await?;
        }

        let memory = Memory {
            id: id.clone(),
            content,
            memory_type,
            embedding: Some(embedding),
            metadata,
            created_at: now,
            updated_at: now,
        };

        self.db.create_memory(&memory)?;
        Ok(id)
    }

    /// Get a memory by ID
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        self.db.get_memory(id)
    }

    /// Get memories by type
    pub fn get_memories_by_type(&self, memory_type: MemoryType) -> Result<Vec<Memory>> {
        self.db.get_memories_by_type(memory_type)
    }

    /// Semantic search using indexed vector backend
    pub async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let query_embedding = self.embedding_provider.embed(query).await?;

        // Use vector backend for similarity search
        let backend = self.vector_backend.lock().await;
        let similar_ids = backend.search(&query_embedding, limit).await?;
        drop(backend);

        // Retrieve actual memories by IDs
        let mut memories = Vec::new();
        for (id, _score) in similar_ids {
            if let Some(memory) = self.db.get_memory(&id)? {
                memories.push(memory);
            }
        }

        Ok(memories)
    }

    /// Log an episodic memory
    pub async fn log_episode(
        &self,
        content: String,
        metadata: serde_json::Value,
    ) -> Result<String> {
        self.create_memory(content, MemoryType::Episodic, metadata)
            .await
    }

    /// Create a relationship between memories
    pub fn create_relationship(&self, relationship: &MemoryRelationship) -> Result<()> {
        self.db.create_relationship(relationship)
    }

    /// Get relationships for a memory
    pub fn get_relationships(&self, memory_id: &str) -> Result<Vec<MemoryRelationship>> {
        self.db.get_relationships(memory_id)
    }

    /// Search memories by content
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        self.db.search_memories(query)
    }

    /// Delete a memory
    pub async fn delete_memory(&self, id: &str) -> Result<()> {
        // Remove from vector index
        let mut backend = self.vector_backend.lock().await;
        let _ = backend.remove_vector(id).await;
        drop(backend);

        // Delete from database
        self.db.delete_memory(id)
    }

    /// Rebuild the vector index from all semantic memories in the database
    pub async fn rebuild_vector_index(&self) -> Result<()> {
        let semantic_memories = self.db.get_memories_by_type(MemoryType::Semantic)?;

        let mut backend = self.vector_backend.lock().await;

        for memory in semantic_memories {
            if let Some(embedding) = memory.embedding {
                backend.add_vector(memory.id.clone(), embedding).await?;
            }
        }

        Ok(())
    }
}

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
            anyhow::bail!("Vector dimension mismatch: expected {}, got {}", self.dimension, vector.len());
        }
        self.vectors.insert(id, vector);
        Ok(())
    }

    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>> {
        if query.len() != self.dimension {
            anyhow::bail!("Query dimension mismatch: expected {}, got {}", self.dimension, query.len());
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

/// ContextLoaderNode - loads relevant memories into flow state
pub struct ContextLoaderNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    query_key: String,
    output_key: String,
    limit: usize,
}

impl ContextLoaderNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        query_key: String,
        output_key: String,
        limit: usize,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            query_key,
            output_key,
            limit,
        }
    }
}

#[async_trait]
impl Node for ContextLoaderNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let query = state
            .get_input(&self.query_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Ok(serde_json::json!({ "query": query }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let query = input["query"].as_str().context("Missing query in input")?;

        let memories = self
            .memory_service
            .semantic_search(query, self.limit)
            .await?;

        let memories_json: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "content": m.content,
                    "memory_type": format!("{:?}", m.memory_type),
                    "metadata": m.metadata,
                })
            })
            .collect();

        Ok(serde_json::json!({ "memories": memories_json }))
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        if let Some(memories) = output["memories"].as_array() {
            state.set_output(self.output_key.clone(), serde_json::json!(memories));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// MemoryWriteNode - writes flow state to memory
pub struct MemoryWriteNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    content_key: String,
    memory_type: MemoryType,
}

impl MemoryWriteNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        content_key: String,
        memory_type: MemoryType,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            content_key,
            memory_type,
        }
    }
}

#[async_trait]
impl Node for MemoryWriteNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let content = state
            .get_input(&self.content_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let metadata = state
            .get_meta("memory_metadata")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        Ok(serde_json::json!({
            "content": content,
            "metadata": metadata
        }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let content = input["content"]
            .as_str()
            .context("Missing content in input")?
            .to_string();

        let metadata = input["metadata"].clone();

        let memory_id = self
            .memory_service
            .create_memory(content, self.memory_type.clone(), metadata)
            .await?;

        Ok(serde_json::json!({ "memory_id": memory_id }))
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        if let Some(memory_id) = output["memory_id"].as_str() {
            state.set_output("memory_id".to_string(), serde_json::json!(memory_id));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_db_init() {
        let _db = MemoryDb::in_memory().unwrap();
        // Schema should be created without errors
    }

    #[test]
    fn test_create_and_get_memory() {
        let db = MemoryDb::in_memory().unwrap();

        let memory = Memory {
            id: Uuid::new_v4().to_string(),
            content: "Test memory content".to_string(),
            memory_type: MemoryType::Episodic,
            embedding: None,
            metadata: serde_json::json!({ "key": "value" }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.create_memory(&memory).unwrap();

        let retrieved = db.get_memory(&memory.id).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.content, memory.content);
        assert_eq!(retrieved.memory_type, memory.memory_type);
    }

    #[test]
    fn test_memories_by_type() {
        let db = MemoryDb::in_memory().unwrap();

        let episodic = Memory {
            id: Uuid::new_v4().to_string(),
            content: "Episodic memory".to_string(),
            memory_type: MemoryType::Episodic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let semantic = Memory {
            id: Uuid::new_v4().to_string(),
            content: "Semantic memory".to_string(),
            memory_type: MemoryType::Semantic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.create_memory(&episodic).unwrap();
        db.create_memory(&semantic).unwrap();

        let episodic_memories = db.get_memories_by_type(MemoryType::Episodic).unwrap();
        assert_eq!(episodic_memories.len(), 1);
        assert_eq!(episodic_memories[0].id, episodic.id);

        let semantic_memories = db.get_memories_by_type(MemoryType::Semantic).unwrap();
        assert_eq!(semantic_memories.len(), 1);
        assert_eq!(semantic_memories[0].id, semantic.id);
    }

    #[test]
    fn test_create_relationship() {
        let db = MemoryDb::in_memory().unwrap();

        let memory1 = Memory {
            id: Uuid::new_v4().to_string(),
            content: "Memory 1".to_string(),
            memory_type: MemoryType::Semantic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let memory2 = Memory {
            id: Uuid::new_v4().to_string(),
            content: "Memory 2".to_string(),
            memory_type: MemoryType::Semantic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.create_memory(&memory1).unwrap();
        db.create_memory(&memory2).unwrap();

        let relationship = MemoryRelationship {
            id: Uuid::new_v4().to_string(),
            source_id: memory1.id.clone(),
            target_id: memory2.id.clone(),
            relationship_type: "related".to_string(),
            strength: 0.8,
            created_at: Utc::now(),
        };

        db.create_relationship(&relationship).unwrap();

        let relationships = db.get_relationships(&memory1.id).unwrap();
        assert_eq!(relationships.len(), 1);
        assert_eq!(relationships[0].relationship_type, "related");
    }

    #[test]
    fn test_search_memories() {
        let db = MemoryDb::in_memory().unwrap();

        let memory = Memory {
            id: Uuid::new_v4().to_string(),
            content: "The quick brown fox jumps over the lazy dog".to_string(),
            memory_type: MemoryType::Episodic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.create_memory(&memory).unwrap();

        let results = db.search_memories("quick").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("quick"));

        let no_results = db.search_memories("elephant").unwrap();
        assert_eq!(no_results.len(), 0);
    }

    #[test]
    fn test_delete_memory() {
        let db = MemoryDb::in_memory().unwrap();

        let memory = Memory {
            id: Uuid::new_v4().to_string(),
            content: "To be deleted".to_string(),
            memory_type: MemoryType::Episodic,
            embedding: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.create_memory(&memory).unwrap();

        let retrieved = db.get_memory(&memory.id).unwrap();
        assert!(retrieved.is_some());

        db.delete_memory(&memory.id).unwrap();

        let retrieved = db.get_memory(&memory.id).unwrap();
        assert!(retrieved.is_none());
    }

    // Mock embedding provider for testing
    struct MockEmbeddingProvider {
        dimension: usize,
    }

    impl MockEmbeddingProvider {
        fn new(dimension: usize) -> Self {
            Self { dimension }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0; self.dimension])
        }

        fn dimension(&self) -> usize {
            self.dimension
        }
    }

    #[tokio::test]
    async fn test_mock_embedding_provider() {
        let provider = MockEmbeddingProvider::new(128);

        let embedding = provider.embed("test text").await.unwrap();
        assert_eq!(embedding.len(), 128);
    }

    #[tokio::test]
    async fn test_in_memory_vector_index() {
        let mut index = InMemoryVectorIndex::new(128);

        let vec1 = vec![0.0; 128];
        let vec2 = vec![1.0; 128];

        index.add_vector("id1".to_string(), vec1.clone()).await.unwrap();
        index.add_vector("id2".to_string(), vec2.clone()).await.unwrap();

        assert_eq!(index.count().await.unwrap(), 2);

        let results = index.search(&vec1, 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "id1"); // Should be most similar to itself
    }

    #[tokio::test]
    async fn test_brute_force_backend() {
        let mut backend = BruteForceBackend::new();

        let vec1 = vec![0.0; 128];
        let vec2 = vec![1.0; 128];

        backend.add_vector("id1".to_string(), vec1.clone()).await.unwrap();
        backend.add_vector("id2".to_string(), vec2.clone()).await.unwrap();

        let results = backend.search(&vec1, 10).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_vector_backend_remove() {
        let mut backend = BruteForceBackend::new();

        backend.add_vector("id1".to_string(), vec![0.0; 128]).await.unwrap();
        assert_eq!(backend.count().await.unwrap(), 1);

        backend.remove_vector("id1").await.unwrap();
        assert_eq!(backend.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_fallback_embedding_provider() {
        let provider1 = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let provider2 = Box::new(MockEmbeddingProvider::new(256)) as Box<dyn EmbeddingProvider>;

        let fallback = FallbackEmbeddingProvider::new(vec![provider1, provider2]);

        let embedding = fallback.embed("test").await.unwrap();
        assert_eq!(embedding.len(), 128);
        assert_eq!(fallback.dimension(), 128);
    }

    #[tokio::test]
    async fn test_memory_service_create() {
        let db = MemoryDb::in_memory().unwrap();
        let provider = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let service = MemoryService::new(db, provider);

        let id = service
            .create_memory(
                "Test memory content".to_string(),
                MemoryType::Semantic,
                serde_json::json!({ "key": "value" }),
            )
            .await
            .unwrap();

        let memory = service.get_memory(&id).unwrap();
        assert!(memory.is_some());
        let memory = memory.unwrap();
        assert_eq!(memory.content, "Test memory content");
        assert!(memory.embedding.is_some());
    }

    #[tokio::test]
    async fn test_memory_service_log_episode() {
        let db = MemoryDb::in_memory().unwrap();
        let provider = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let service = MemoryService::new(db, provider);

        let id = service
            .log_episode(
                "Episode content".to_string(),
                serde_json::json!({ "type": "conversation" }),
            )
            .await
            .unwrap();

        let memory = service.get_memory(&id).unwrap();
        assert!(memory.is_some());
        let memory = memory.unwrap();
        assert_eq!(memory.memory_type, MemoryType::Episodic);
    }

    #[tokio::test]
    async fn test_memory_service_semantic_search() {
        let db = MemoryDb::in_memory().unwrap();
        let provider = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let service = MemoryService::new(db, provider);

        service
            .create_memory(
                "First semantic memory".to_string(),
                MemoryType::Semantic,
                serde_json::json!({}),
            )
            .await
            .unwrap();

        service
            .create_memory(
                "Second semantic memory".to_string(),
                MemoryType::Semantic,
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let results = service.semantic_search("semantic", 10).await.unwrap();
        assert!(results.len() > 0);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.001);

        let c = vec![1.0, 0.0, 0.0];
        let d = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&c, &d);
        assert!((sim - 0.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_context_loader_node() {
        let db = MemoryDb::in_memory().unwrap();
        let provider = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let service = Arc::new(MemoryService::new(db, provider));

        // Create some semantic memories
        service
            .create_memory(
                "Test memory about coding".to_string(),
                MemoryType::Semantic,
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let node = ContextLoaderNode::new(
            "context_loader".to_string(),
            service.clone(),
            "query".to_string(),
            "context".to_string(),
            5,
        );

        let mut state = SharedState::new();
        state.set_input("query".to_string(), serde_json::json!("coding"));

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        let action = node.post(&mut state, output);

        assert_eq!(action, "continue");
        assert!(state.get_output("context").is_some());
    }

    #[tokio::test]
    async fn test_memory_write_node() {
        let db = MemoryDb::in_memory().unwrap();
        let provider = Box::new(MockEmbeddingProvider::new(128)) as Box<dyn EmbeddingProvider>;
        let service = Arc::new(MemoryService::new(db, provider));

        let node = MemoryWriteNode::new(
            "memory_writer".to_string(),
            service.clone(),
            "content".to_string(),
            MemoryType::Episodic,
        );

        let mut state = SharedState::new();
        state.set_input(
            "content".to_string(),
            serde_json::json!("This is an episode"),
        );
        state.set_meta(
            "memory_metadata".to_string(),
            serde_json::json!({ "type": "test" }),
        );

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        let action = node.post(&mut state, output);

        assert_eq!(action, "continue");
        assert!(state.get_output("memory_id").is_some());
    }
}
