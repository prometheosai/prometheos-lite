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
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::flow::{Action, Input, Node, NodeConfig, Output, SharedState};

/// Memory entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub user_id: Option<String>,
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
    pub kind: MemoryKind,
    pub content: String,
    pub summary: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub importance_score: f32,
    pub confidence_score: f32,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_accessed_at: Option<DateTime<Utc>>,
    pub access_count: i32,
    pub metadata: serde_json::Value,
}

/// Context bundle for structured memory retrieval
#[derive(Debug, Clone, Default)]
pub struct ContextBundle {
    pub project_facts: Vec<Memory>,
    pub user_preferences: Vec<Memory>,
    pub recent_episodes: Vec<Memory>,
    pub decisions_constraints: Vec<Memory>,
}

/// Memory kind classification (more specific than memory_type)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Preference,
    Decision,
    Constraint,
    ProjectFact,
    BehaviorPattern,
}

/// Memory type classification (legacy, kept for compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    Working,
}

/// Memory write task for async processing
#[derive(Debug, Clone)]
pub enum MemoryWriteTask {
    LogEpisode {
        content: String,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        metadata: serde_json::Value,
    },
    CreateSemantic {
        content: String,
        kind: MemoryKind,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        summary: Option<String>,
        importance_score: f32,
        confidence_score: f32,
        metadata: serde_json::Value,
    },
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

        // Updated memories table with new fields
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                user_id TEXT,
                project_id TEXT,
                conversation_id TEXT,
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                summary TEXT,
                embedding BLOB,
                importance_score REAL DEFAULT 0.5,
                confidence_score REAL DEFAULT 0.5,
                source TEXT DEFAULT 'system',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_accessed_at TEXT,
                access_count INTEGER DEFAULT 0,
                metadata TEXT NOT NULL
            )",
            [],
        )
        .context("Failed to create memories table")?;

        // Memory events table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_events (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
            )",
            [],
        )
        .context("Failed to create memory_events table")?;

        // User model table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_model (
                user_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence_score REAL DEFAULT 0.5,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (user_id, key)
            )",
            [],
        )
        .context("Failed to create user_model table")?;

        // Legacy relationships table (kept for compatibility)
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

        // Indexes
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_kind ON memories(kind)",
            [],
        )
        .context("Failed to create memory_kind index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_project ON memories(project_id)",
            [],
        )
        .context("Failed to create memory_project index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_conversation ON memories(conversation_id)",
            [],
        )
        .context("Failed to create memory_conversation index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_importance ON memories(importance_score)",
            [],
        )
        .context("Failed to create memory_importance index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_last_accessed ON memories(last_accessed_at)",
            [],
        )
        .context("Failed to create memory_last_accessed index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_events_memory ON memory_events(memory_id)",
            [],
        )
        .context("Failed to create memory_events_memory index")?;

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

        let kind_str = match memory.kind {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Preference => "preference",
            MemoryKind::Decision => "decision",
            MemoryKind::Constraint => "constraint",
            MemoryKind::ProjectFact => "project_fact",
            MemoryKind::BehaviorPattern => "behavior_pattern",
        };

        conn.execute(
            "INSERT INTO memories (id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                memory.id,
                memory.user_id,
                memory.project_id,
                memory.conversation_id,
                kind_str,
                memory.content,
                memory.summary,
                embedding_blob,
                memory.importance_score,
                memory.confidence_score,
                memory.source,
                memory.created_at.to_rfc3339(),
                memory.updated_at.to_rfc3339(),
                memory.last_accessed_at.map(|t| t.to_rfc3339()),
                memory.access_count,
                metadata_json,
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
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
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

    /// Get memories by kind
    pub fn get_memories_by_kind(&self, kind: MemoryKind) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let kind_str = match kind {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Preference => "preference",
            MemoryKind::Decision => "decision",
            MemoryKind::Constraint => "constraint",
            MemoryKind::ProjectFact => "project_fact",
            MemoryKind::BehaviorPattern => "behavior_pattern",
        };

        let mut stmt = conn
            .prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories WHERE kind = ?1",
            )
            .context("Failed to prepare get_memories_by_kind query")?;

        let rows = stmt
            .query_map(params![kind_str], |row| self.row_to_memory(row))
            .context("Failed to query memories by kind")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Get memories by project
    pub fn get_memories_by_project(&self, project_id: &str) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories WHERE project_id = ?1",
            )
            .context("Failed to prepare get_memories_by_project query")?;

        let rows = stmt
            .query_map(params![project_id], |row| self.row_to_memory(row))
            .context("Failed to query memories by project")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Get memories by conversation
    pub fn get_memories_by_conversation(&self, conversation_id: &str) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories WHERE conversation_id = ?1",
            )
            .context("Failed to prepare get_memories_by_conversation query")?;

        let rows = stmt
            .query_map(params![conversation_id], |row| self.row_to_memory(row))
            .context("Failed to query memories by conversation")?;

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
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories WHERE content LIKE ?1 OR summary LIKE ?1",
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
        let kind_str: String = row.get(4)?;
        let kind = match kind_str.as_str() {
            "episodic" => MemoryKind::Episodic,
            "semantic" => MemoryKind::Semantic,
            "preference" => MemoryKind::Preference,
            "decision" => MemoryKind::Decision,
            "constraint" => MemoryKind::Constraint,
            "project_fact" => MemoryKind::ProjectFact,
            "behavior_pattern" => MemoryKind::BehaviorPattern,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };

        let embedding_blob: Option<Vec<u8>> = row.get(7)?;
        let embedding = embedding_blob.map(|blob| {
            blob.chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect()
        });

        let metadata_json: String = row.get(15)?;
        let metadata = serde_json::from_str(&metadata_json)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let last_accessed_at: Option<String> = row.get(12)?;
        let last_accessed = last_accessed_at.and_then(|s| {
            DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))
        });

        Ok(Memory {
            id: row.get(0)?,
            user_id: row.get(1)?,
            project_id: row.get(2)?,
            conversation_id: row.get(3)?,
            kind,
            content: row.get(5)?,
            summary: row.get(6)?,
            embedding,
            importance_score: row.get(8)?,
            confidence_score: row.get(9)?,
            source: row.get(10)?,
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(11)?)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(13)?)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                .with_timezone(&Utc),
            last_accessed_at: last_accessed,
            access_count: row.get(14)?,
            metadata,
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
    db: Arc<tokio::sync::Mutex<MemoryDb>>,
    embedding_provider: Box<dyn EmbeddingProvider>,
    vector_backend: Arc<tokio::sync::Mutex<Box<dyn VectorSearchBackend>>>,
    write_tx: mpsc::UnboundedSender<MemoryWriteTask>,
}

impl MemoryService {
    pub fn new(db: MemoryDb, embedding_provider: Box<dyn EmbeddingProvider>) -> Self {
        let vector_backend: Box<dyn VectorSearchBackend> = Box::new(BruteForceBackend::new());
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        
        let db_arc = Arc::new(tokio::sync::Mutex::new(db));
        
        let service = Self {
            db: db_arc.clone(),
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
            write_tx,
        };
        
        // Spawn background task processor
        tokio::spawn(async move {
            Self::process_write_tasks(db_arc, write_rx).await;
        });
        
        service
    }

    /// Create a MemoryService with a custom vector search backend
    pub fn with_vector_backend(
        db: MemoryDb,
        embedding_provider: Box<dyn EmbeddingProvider>,
        vector_backend: Box<dyn VectorSearchBackend>,
    ) -> Self {
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        
        let db_arc = Arc::new(tokio::sync::Mutex::new(db));
        
        let service = Self {
            db: db_arc.clone(),
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
            write_tx,
        };
        
        // Spawn background task processor
        tokio::spawn(async move {
            Self::process_write_tasks(db_arc, write_rx).await;
        });
        
        service
    }

    /// Background task processor for memory writes
    async fn process_write_tasks(db: Arc<tokio::sync::Mutex<MemoryDb>>, mut rx: mpsc::UnboundedReceiver<MemoryWriteTask>) {
        while let Some(task) = rx.recv().await {
            match task {
                MemoryWriteTask::LogEpisode {
                    content,
                    user_id,
                    project_id,
                    conversation_id,
                    metadata,
                } => {
                    let memory = Memory {
                        id: Uuid::new_v4().to_string(),
                        user_id,
                        project_id,
                        conversation_id,
                        kind: MemoryKind::Episodic,
                        content,
                        summary: None,
                        embedding: None,
                        importance_score: 0.3,
                        confidence_score: 0.8,
                        source: "conversation".to_string(),
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                        last_accessed_at: None,
                        access_count: 0,
                        metadata,
                    };
                    let db_guard = db.lock().await;
                    let _ = db_guard.create_memory(&memory);
                }
                MemoryWriteTask::CreateSemantic {
                    content,
                    kind,
                    user_id,
                    project_id,
                    conversation_id,
                    summary,
                    importance_score,
                    confidence_score,
                    metadata,
                } => {
                    let memory = Memory {
                        id: Uuid::new_v4().to_string(),
                        user_id,
                        project_id,
                        conversation_id,
                        kind,
                        content,
                        summary,
                        embedding: None, // Will be generated by embedding provider if needed
                        importance_score,
                        confidence_score,
                        source: "extractor".to_string(),
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                        last_accessed_at: None,
                        access_count: 0,
                        metadata,
                    };
                    let db_guard = db.lock().await;
                    let _ = db_guard.create_memory(&memory);
                }
            }
        }
    }

    /// Queue an episodic memory write (async, non-blocking)
    pub fn queue_episode(
        &self,
        content: String,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<()> {
        self.write_tx.send(MemoryWriteTask::LogEpisode {
            content,
            user_id,
            project_id,
            conversation_id,
            metadata,
        }).context("Failed to queue episode write")?;
        Ok(())
    }

    /// Queue a semantic memory write (async, non-blocking)
    pub fn queue_semantic(
        &self,
        content: String,
        kind: MemoryKind,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        summary: Option<String>,
        importance_score: f32,
        confidence_score: f32,
        metadata: serde_json::Value,
    ) -> Result<()> {
        // Check for deduplication before queuing
        if let Ok(similar) = self.find_similar_memory(&content, &kind, project_id.as_deref()) {
            if let Some(existing) = similar {
                // Update existing memory instead of creating new one
                let _ = self.update_memory_importance(&existing.id, importance_score);
                return Ok(());
            }
        }
        
        self.write_tx.send(MemoryWriteTask::CreateSemantic {
            content,
            kind,
            user_id,
            project_id,
            conversation_id,
            summary,
            importance_score,
            confidence_score,
            metadata,
        }).context("Failed to queue semantic write")?;
        Ok(())
    }

    /// Find similar memory for deduplication
    fn find_similar_memory(
        &self,
        content: &str,
        kind: &MemoryKind,
        project_id: Option<&str>,
    ) -> Result<Option<Memory>> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let kind_str = match kind {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Preference => "preference",
            MemoryKind::Decision => "decision",
            MemoryKind::Constraint => "constraint",
            MemoryKind::ProjectFact => "project_fact",
            MemoryKind::BehaviorPattern => "behavior_pattern",
        };

        let mut memories: Vec<Memory> = Vec::new();

        if let Some(pid) = project_id {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = ?1 AND project_id = ?2 
                 ORDER BY created_at DESC LIMIT 5"
            ).context("Failed to prepare deduplication query with project")?;
            let rows = stmt.query_map(params![kind_str, pid], |row| db_guard.row_to_memory(row))
                .context("Failed to query similar memories with project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                if self.content_similarity(content, &memory.content) > 0.7 {
                    return Ok(Some(memory));
                }
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = ?1 
                 ORDER BY created_at DESC LIMIT 5"
            ).context("Failed to prepare deduplication query without project")?;
            let rows = stmt.query_map(params![kind_str], |row| db_guard.row_to_memory(row))
                .context("Failed to query similar memories without project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                if self.content_similarity(content, &memory.content) > 0.7 {
                    return Ok(Some(memory));
                }
            }
        }

        Ok(None)
    }

    /// Simple content similarity for deduplication (word overlap)
    fn content_similarity(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();
        
        let words_a: std::collections::HashSet<&str> = a_lower
            .split_whitespace()
            .collect();
        let words_b: std::collections::HashSet<&str> = b_lower
            .split_whitespace()
            .collect();

        if words_a.is_empty() || words_b.is_empty() {
            return 0.0;
        }

        let intersection = words_a.intersection(&words_b).count();
        let union = words_a.union(&words_b).count();

        if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        }
    }

    /// Update memory importance score
    fn update_memory_importance(&self, memory_id: &str, new_importance: f32) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "UPDATE memories SET importance_score = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_importance, Utc::now().to_rfc3339(), memory_id],
        ).context("Failed to update memory importance")?;

        Ok(())
    }

    /// Calculate final memory score for retrieval ranking
    pub fn calculate_memory_score(
        &self,
        memory: &Memory,
        semantic_similarity: f32,
        project_id: Option<&str>,
    ) -> f32 {
        // Semantic similarity: 45%
        let similarity_score = semantic_similarity;

        // Recency: 20% (more recent = higher score)
        let days_old = (Utc::now() - memory.created_at).num_days() as f32;
        let recency_score = (1.0 / (1.0 + days_old / 30.0)).min(1.0); // Decay over 30 days

        // Importance: 25%
        let importance_score = memory.importance_score;

        // Project match: 10%
        let project_match_score = if let Some(pid) = project_id {
            if memory.project_id.as_deref() == Some(pid) {
                1.0
            } else {
                0.0
            }
        } else {
            0.5 // Neutral if no project context
        };

        // Weighted final score
        similarity_score * 0.45 + recency_score * 0.20 + importance_score * 0.25 + project_match_score * 0.10
    }

    /// Update memory access tracking (last_accessed_at, access_count)
    pub fn track_memory_access(&self, memory_id: &str) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "UPDATE memories SET last_accessed_at = ?1, access_count = access_count + 1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), memory_id],
        ).context("Failed to track memory access")?;

        Ok(())
    }

    /// Apply memory decay to low-value, rarely accessed memories
    pub fn apply_memory_decay(&self, days_threshold: i64) -> Result<usize> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let threshold_date = Utc::now() - chrono::Duration::days(days_threshold);

        // Decay importance_score for memories that haven't been accessed recently
        let rows_affected = conn.execute(
            "UPDATE memories 
             SET importance_score = importance_score * 0.9,
                 updated_at = ?1
             WHERE last_accessed_at < ?2 
             AND importance_score > 0.1
             AND kind != 'episodic'", // Don't decay episodic memories (they're history)
            params![Utc::now().to_rfc3339(), threshold_date.to_rfc3339()],
        ).context("Failed to apply memory decay")?;

        Ok(rows_affected)
    }

    /// Cleanup very low-value memories (importance < 0.1 and not accessed in 90 days)
    pub fn cleanup_stale_memories(&self, days_threshold: i64) -> Result<usize> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let threshold_date = Utc::now() - chrono::Duration::days(days_threshold);

        let rows_affected = conn.execute(
            "DELETE FROM memories 
             WHERE importance_score < 0.1 
             AND (last_accessed_at < ?1 OR last_accessed_at IS NULL)
             AND kind != 'episodic'", // Don't delete episodic memories
            params![threshold_date.to_rfc3339()],
        ).context("Failed to cleanup stale memories")?;

        Ok(rows_affected)
    }

    /// Update user model with a key-value pair
    pub fn update_user_model(
        &self,
        user_id: &str,
        key: &str,
        value: serde_json::Value,
        confidence_score: f32,
    ) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let value_json = serde_json::to_string(&value).context("Failed to serialize value")?;

        conn.execute(
            "INSERT INTO user_model (user_id, key, value, confidence_score, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_id, key) DO UPDATE SET
                 value = ?3,
                 confidence_score = ?4,
                 updated_at = ?5",
            params![
                user_id,
                key,
                value_json,
                confidence_score,
                Utc::now().to_rfc3339(),
            ],
        ).context("Failed to update user model")?;

        Ok(())
    }

    /// Get user model value for a key
    pub fn get_user_model(&self, user_id: &str, key: &str) -> Result<Option<(serde_json::Value, f32)>> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT value, confidence_score FROM user_model WHERE user_id = ?1 AND key = ?2"
        ).context("Failed to prepare get_user_model query")?;

        let mut rows = stmt.query(params![user_id, key]).context("Failed to query user model")?;

        if let Some(row) = rows.next()? {
            let value_json: String = row.get(0)?;
            let confidence_score: f32 = row.get(1)?;
            let value = serde_json::from_str(&value_json)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize user model value: {}", e))?;
            Ok(Some((value, confidence_score)))
        } else {
            Ok(None)
        }
    }

    /// Get all user model entries for a user
    pub fn get_user_model_all(&self, user_id: &str) -> Result<Vec<(String, serde_json::Value, f32)>> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT key, value, confidence_score FROM user_model WHERE user_id = ?1"
        ).context("Failed to prepare get_user_model_all query")?;

        let rows = stmt.query_map(params![user_id], |row| {
            let key: String = row.get(0)?;
            let value_json: String = row.get(1)?;
            let confidence_score: f32 = row.get(2)?;
            let value = serde_json::from_str(&value_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok((key, value, confidence_score))
        }).context("Failed to query user model")?;

        let mut entries = Vec::new();
        for row in rows {
            let entry = row.map_err(|e| anyhow::anyhow!(e))?;
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Promote high-confidence semantic memory to user model
    pub fn promote_to_user_model(
        &self,
        memory: &Memory,
        user_id: Option<&str>,
    ) -> Result<()> {
        if memory.confidence_score < 0.8 {
            return Ok(()); // Only promote high-confidence memories
        }

        let user_id = user_id.unwrap_or("default");

        let key = match memory.kind {
            MemoryKind::Preference => format!("preference_{}", memory.id),
            MemoryKind::Decision => format!("decision_{}", memory.id),
            MemoryKind::Constraint => format!("constraint_{}", memory.id),
            _ => return Ok(()), // Only promote certain types
        };

        self.update_user_model(
            user_id,
            &key,
            serde_json::json!({
                "content": memory.content,
                "summary": memory.summary,
                "source": "memory_promotion",
                "original_memory_id": memory.id,
            }),
            memory.confidence_score,
        )
    }

    /// Retrieve context as category-based bundles
    pub fn retrieve_context_bundles(
        &self,
        query: &str,
        project_id: Option<&str>,
        budget: &crate::config::MemoryBudget,
        total_limit: usize,
    ) -> Result<ContextBundle> {
        let mut bundle = ContextBundle::default();

        // Calculate limits per category based on budget
        let project_facts_limit = (total_limit as f32 * budget.project_facts) as usize;
        let user_preferences_limit = (total_limit as f32 * budget.user_preferences) as usize;
        let recent_episodes_limit = (total_limit as f32 * budget.recent_episodes) as usize;
        let decisions_constraints_limit = (total_limit as f32 * budget.decisions_constraints) as usize;

        // Retrieve project facts
        bundle.project_facts = self.get_memories_by_kind_and_limit(
            MemoryKind::ProjectFact,
            project_id,
            project_facts_limit,
        )?;

        // Retrieve user preferences
        bundle.user_preferences = self.get_memories_by_kind_and_limit(
            MemoryKind::Preference,
            project_id,
            user_preferences_limit,
        )?;

        // Retrieve recent episodes (episodic memories, sorted by recency)
        bundle.recent_episodes = self.get_recent_episodes(project_id, recent_episodes_limit)?;

        // Retrieve decisions and constraints
        let decisions = self.get_memories_by_kind_and_limit(
            MemoryKind::Decision,
            project_id,
            decisions_constraints_limit / 2,
        )?;
        let constraints = self.get_memories_by_kind_and_limit(
            MemoryKind::Constraint,
            project_id,
            decisions_constraints_limit / 2,
        )?;
        bundle.decisions_constraints = decisions.into_iter().chain(constraints).collect();

        Ok(bundle)
    }

    /// Get memories by kind with limit
    fn get_memories_by_kind_and_limit(
        &self,
        kind: MemoryKind,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let kind_str = match kind {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Preference => "preference",
            MemoryKind::Decision => "decision",
            MemoryKind::Constraint => "constraint",
            MemoryKind::ProjectFact => "project_fact",
            MemoryKind::BehaviorPattern => "behavior_pattern",
        };

        let query = if let Some(pid) = project_id {
            "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories 
             WHERE kind = ?1 AND project_id = ?2 
             ORDER BY importance_score DESC, created_at DESC 
             LIMIT ?3"
        } else {
            "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories 
             WHERE kind = ?1 
             ORDER BY importance_score DESC, created_at DESC 
             LIMIT ?2"
        };

        let mut memories = Vec::new();

        if let Some(pid) = project_id {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = ?1 AND project_id = ?2 
                 ORDER BY importance_score DESC, created_at DESC 
                 LIMIT ?3"
            ).context("Failed to prepare kind query with project")?;
            let rows = stmt.query_map(params![kind_str, pid, limit], |row| db_guard.row_to_memory(row))
                .context("Failed to query memories by kind with project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                memories.push(memory);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = ?1 
                 ORDER BY importance_score DESC, created_at DESC 
                 LIMIT ?2"
            ).context("Failed to prepare kind query without project")?;
            let rows = stmt.query_map(params![kind_str, limit], |row| db_guard.row_to_memory(row))
                .context("Failed to query memories by kind without project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                memories.push(memory);
            }
        }

        Ok(memories)
    }

    /// Get recent episodic memories
    fn get_recent_episodes(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        let conn = db_guard.conn.lock().map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let query = if let Some(pid) = project_id {
            "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories 
             WHERE kind = 'episodic' AND project_id = ?1 
             ORDER BY created_at DESC 
             LIMIT ?2"
        } else {
            "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
             FROM memories 
             WHERE kind = 'episodic' 
             ORDER BY created_at DESC 
             LIMIT ?1"
        };

        let mut memories = Vec::new();

        if let Some(pid) = project_id {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = 'episodic' AND project_id = ?1 
                 ORDER BY created_at DESC 
                 LIMIT ?2"
            ).context("Failed to prepare episodes query with project")?;
            let rows = stmt.query_map(params![pid, limit], |row| db_guard.row_to_memory(row))
                .context("Failed to query recent episodes with project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                memories.push(memory);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories 
                 WHERE kind = 'episodic' 
                 ORDER BY created_at DESC 
                 LIMIT ?1"
            ).context("Failed to prepare episodes query without project")?;
            let rows = stmt.query_map(params![limit], |row| db_guard.row_to_memory(row))
                .context("Failed to query recent episodes without project")?;
            for row in rows {
                let memory = row.map_err(|e| anyhow::anyhow!(e))?;
                memories.push(memory);
            }
        }

        Ok(memories)
    }

    /// Format context bundle as string for LLM injection
    pub fn format_context_bundle(&self, bundle: &ContextBundle) -> String {
        let mut parts = Vec::new();

        if !bundle.project_facts.is_empty() {
            parts.push("Project Facts:".to_string());
            for fact in &bundle.project_facts {
                if let Some(summary) = &fact.summary {
                    parts.push(format!("- {}", summary));
                } else {
                    parts.push(format!("- {}", fact.content));
                }
            }
        }

        if !bundle.user_preferences.is_empty() {
            parts.push("User Preferences:".to_string());
            for pref in &bundle.user_preferences {
                if let Some(summary) = &pref.summary {
                    parts.push(format!("- {}", summary));
                } else {
                    parts.push(format!("- {}", pref.content));
                }
            }
        }

        if !bundle.decisions_constraints.is_empty() {
            parts.push("Decisions & Constraints:".to_string());
            for item in &bundle.decisions_constraints {
                if let Some(summary) = &item.summary {
                    parts.push(format!("- {}", summary));
                } else {
                    parts.push(format!("- {}", item.content));
                }
            }
        }

        if !bundle.recent_episodes.is_empty() {
            parts.push("Recent Context:".to_string());
            for episode in &bundle.recent_episodes {
                parts.push(format!("- {}", episode.content));
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("Relevant Memory Context:\n{}", parts.join("\n"))
        }
    }
}

impl MemoryService {
    /// Create a memory with automatic embedding generation
    pub async fn create_memory(
        &self,
        content: String,
        kind: MemoryKind,
        metadata: serde_json::Value,
    ) -> Result<String> {
        self.create_memory_with_options(
            content,
            kind,
            None, // user_id
            None, // project_id
            None, // conversation_id
            None, // summary
            0.5,  // importance_score
            0.5,  // confidence_score
            "system".to_string(), // source
            metadata,
        ).await
    }

    /// Create a memory with full options
    pub async fn create_memory_with_options(
        &self,
        content: String,
        kind: MemoryKind,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        summary: Option<String>,
        importance_score: f32,
        confidence_score: f32,
        source: String,
        metadata: serde_json::Value,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // Generate embedding only for semantic memories (not episodic)
        let embedding = match kind {
            MemoryKind::Semantic | MemoryKind::Preference | MemoryKind::Decision | 
            MemoryKind::Constraint | MemoryKind::ProjectFact | MemoryKind::BehaviorPattern => {
                Some(self.embedding_provider.embed(&content).await?)
            }
            MemoryKind::Episodic => None, // No embeddings for episodic (raw history)
        };

        // Add to vector index if it has an embedding
        if let Some(ref emb) = embedding {
            let mut backend = self.vector_backend.lock().await;
            backend.add_vector(id.clone(), emb.clone()).await?;
        }

        let memory = Memory {
            id: id.clone(),
            user_id,
            project_id,
            conversation_id,
            kind,
            content,
            summary,
            embedding,
            importance_score,
            confidence_score,
            source,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
            metadata,
        };

        let db_guard = self.db.blocking_lock();
        db_guard.create_memory(&memory)?;
        Ok(id)
    }

    /// Get a memory by ID
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_memory(id)
    }

    /// Get memories by kind
    pub fn get_memories_by_kind(&self, kind: MemoryKind) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_memories_by_kind(kind)
    }

    /// Get memories by project
    pub fn get_memories_by_project(&self, project_id: &str) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_memories_by_project(project_id)
    }

    /// Get memories by conversation
    pub fn get_memories_by_conversation(&self, conversation_id: &str) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_memories_by_conversation(conversation_id)
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
            let db_guard = self.db.blocking_lock();
            if let Some(memory) = db_guard.get_memory(&id)? {
                memories.push(memory);
            }
        }

        Ok(memories)
    }

    /// Log an episodic memory (conversation exchange)
    pub async fn log_episode(
        &self,
        content: String,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<String> {
        self.create_memory_with_options(
            content,
            MemoryKind::Episodic,
            user_id,
            project_id,
            conversation_id,
            None, // summary
            0.3,  // importance_score (lower for raw episodes)
            0.8,  // confidence_score (high for actual events)
            "conversation".to_string(), // source
            metadata,
        ).await
    }

    /// Create a relationship between memories
    pub fn create_relationship(&self, relationship: &MemoryRelationship) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        db_guard.create_relationship(relationship)
    }

    /// Get relationships for a memory
    pub fn get_relationships(&self, memory_id: &str) -> Result<Vec<MemoryRelationship>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_relationships(memory_id)
    }

    /// Search memories by content
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.search_memories(query)
    }

    /// Delete a memory
    pub async fn delete_memory(&self, id: &str) -> Result<()> {
        // Remove from vector index
        let mut backend = self.vector_backend.lock().await;
        let _ = backend.remove_vector(id).await;
        drop(backend);

        // Delete from database
        let db_guard = self.db.blocking_lock();
        db_guard.delete_memory(id)
    }

    /// Rebuild the vector index from all semantic memories in the database
    pub async fn rebuild_vector_index(&self) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        let semantic_memories = db_guard.get_memories_by_kind(MemoryKind::Semantic)?;
        drop(db_guard);

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

/// MemoryExtractorNode - extracts semantic memories from conversation exchanges
pub struct MemoryExtractorNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    user_message_key: String,
    assistant_response_key: String,
    conversation_id_key: String,
}

impl MemoryExtractorNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        user_message_key: String,
        assistant_response_key: String,
        conversation_id_key: String,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            user_message_key,
            assistant_response_key,
            conversation_id_key,
        }
    }
}

#[async_trait]
impl Node for MemoryExtractorNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let user_message = state
            .get_input(&self.user_message_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        let assistant_response = state
            .get_input(&self.assistant_response_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        let conversation_id = state
            .get_input(&self.conversation_id_key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(serde_json::json!({
            "user_message": user_message,
            "assistant_response": assistant_response,
            "conversation_id": conversation_id,
        }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let user_message = input["user_message"].as_str().context("Missing user_message")?;
        let assistant_response = input["assistant_response"].as_str().context("Missing assistant_response")?;
        let conversation_id = input["conversation_id"].as_str().map(|s| s.to_string());

        // Simple heuristic extraction (in production, use LLM for better extraction)
        let extracted_memories = self.extract_semantic_memories(user_message, assistant_response);

        for memory in &extracted_memories {
            let _ = self.memory_service.queue_semantic(
                memory.content.clone(),
                memory.kind.clone(),
                None, // user_id
                None, // project_id
                conversation_id.clone(),
                memory.summary.clone(),
                memory.importance_score,
                memory.confidence_score,
                memory.metadata.clone(),
            );
        }

        Ok(serde_json::json!({
            "extracted_count": extracted_memories.len(),
        }))
    }

    fn post(&self, _state: &mut SharedState, output: Output) -> Action {
        if let Some(count) = output["extracted_count"].as_u64() {
            // Could emit event about extraction
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

#[derive(Debug, Clone)]
struct ExtractedMemory {
    content: String,
    kind: MemoryKind,
    summary: Option<String>,
    importance_score: f32,
    confidence_score: f32,
    metadata: serde_json::Value,
}

impl MemoryExtractorNode {
    /// Extract semantic memories from conversation (heuristic-based)
    fn extract_semantic_memories(&self, user_message: &str, assistant_response: &str) -> Vec<ExtractedMemory> {
        let mut memories = Vec::new();
        let combined = format!("{}\n{}", user_message, assistant_response).to_lowercase();

        // Extract preferences
        if combined.contains("prefer") || combined.contains("like") || combined.contains("want") {
            memories.push(ExtractedMemory {
                content: format!("User preference detected in conversation"),
                kind: MemoryKind::Preference,
                summary: Some("User expressed a preference".to_string()),
                importance_score: 0.7,
                confidence_score: 0.6,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "preference_detection",
                }),
            });
        }

        // Extract decisions
        if combined.contains("decided") || combined.contains("choose") || combined.contains("will") {
            memories.push(ExtractedMemory {
                content: format!("Decision made in conversation"),
                kind: MemoryKind::Decision,
                summary: Some("A decision was made".to_string()),
                importance_score: 0.8,
                confidence_score: 0.7,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "decision_detection",
                }),
            });
        }

        // Extract constraints
        if combined.contains("must") || combined.contains("should") || combined.contains("require") {
            memories.push(ExtractedMemory {
                content: format!("Constraint identified in conversation"),
                kind: MemoryKind::Constraint,
                summary: Some("A constraint was identified".to_string()),
                importance_score: 0.75,
                confidence_score: 0.65,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "constraint_detection",
                }),
            });
        }

        // Extract project facts (heuristic: technical terms, file names, etc.)
        if combined.contains("file") || combined.contains("function") || combined.contains("class") {
            memories.push(ExtractedMemory {
                content: format!("Project fact mentioned in conversation"),
                kind: MemoryKind::ProjectFact,
                summary: Some("Project-related information".to_string()),
                importance_score: 0.6,
                confidence_score: 0.5,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "project_fact_detection",
                }),
            });
        }

        memories
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
                    "kind": format!("{:?}", m.kind),
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

/// MemoryWriteNode - writes memories to the memory service
pub struct MemoryWriteNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    content_key: String,
    kind: MemoryKind,
}

impl MemoryWriteNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        content_key: String,
        kind: MemoryKind,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            content_key,
            kind,
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

        Ok(serde_json::json!({ "content": content, "metadata": metadata }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let content = input["content"].as_str().context("Missing content")?;

        let metadata = input["metadata"].clone();

        let memory_id = self
            .memory_service
            .create_memory(content.to_string(), self.kind.clone(), metadata)
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
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Episodic,
            content: "Test memory".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({ "key": "value" }),
        };

        db.create_memory(&memory).unwrap();

        let retrieved = db.get_memory(&memory.id).unwrap().unwrap();
        assert_eq!(retrieved.content, memory.content);
        assert_eq!(retrieved.kind, memory.kind);
        assert_eq!(retrieved.content, memory.content);
        assert_eq!(retrieved.kind, memory.kind);
    }

    #[test]
    fn test_memories_by_kind() {
        let db = MemoryDb::in_memory().unwrap();

        let episodic = Memory {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Episodic,
            content: "Episodic memory".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
        };

        let semantic = Memory {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Semantic,
            content: "Semantic memory".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
        };

        db.create_memory(&episodic).unwrap();
        db.create_memory(&semantic).unwrap();

        let episodic_memories = db.get_memories_by_kind(MemoryKind::Episodic).unwrap();
        assert_eq!(episodic_memories.len(), 1);
        assert_eq!(episodic_memories[0].kind, MemoryKind::Episodic);

        let semantic_memories = db.get_memories_by_kind(MemoryKind::Semantic).unwrap();
        assert_eq!(semantic_memories.len(), 1);
        assert_eq!(semantic_memories[0].kind, MemoryKind::Semantic);
    }

    #[test]
    fn test_create_relationship() {
        let db = MemoryDb::in_memory().unwrap();

        let memory1 = Memory {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Semantic,
            content: "Memory 1".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
        };

        let memory2 = Memory {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Semantic,
            content: "Memory 2".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
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
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: MemoryKind::Episodic,
            content: "To be deleted".to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
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
                None, // user_id
                None, // project_id
                None, // conversation_id
                serde_json::json!({ "type": "conversation" }),
            )
            .await
            .unwrap();

        let memory = service.get_memory(&id).unwrap();
        assert!(memory.is_some());
        let memory = memory.unwrap();
        assert_eq!(memory.kind, MemoryKind::Episodic);
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
