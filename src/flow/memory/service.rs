//! Memory service for high-level memory operations

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::db::MemoryDb;
use super::embedding::EmbeddingProvider;
use super::scoring::{prune_combined, rank_memories};
use super::summarizer::MemorySummarizer;
use super::types::{ContextBundle, Memory, MemoryKind, MemoryType, MemoryWriteTask};
use super::vector::{BruteForceBackend, VectorSearchBackend};

/// Memory service for high-level memory operations
pub struct MemoryService {
    db: Arc<tokio::sync::Mutex<MemoryDb>>,
    embedding_provider: Box<dyn EmbeddingProvider>,
    vector_backend: Arc<tokio::sync::Mutex<BruteForceBackend>>,
    write_tx: mpsc::UnboundedSender<MemoryWriteTask>,
    summarizer: MemorySummarizer,
    max_memory_count: usize,
    compression_threshold_tokens: usize,
}

impl MemoryService {
    /// Create a new memory service
    pub fn new(db: MemoryDb, embedding_provider: Box<dyn EmbeddingProvider>) -> Self {
        let vector_backend = BruteForceBackend::new();

        let (write_tx, write_rx) = mpsc::unbounded_channel();

        let db_arc = Arc::new(tokio::sync::Mutex::new(db));

        let service = Self {
            db: db_arc.clone(),
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
            write_tx,
            summarizer: MemorySummarizer::default(),
            max_memory_count: 1000,
            compression_threshold_tokens: 100_000,
        };

        // Spawn background task processor
        tokio::spawn(async move {
            Self::process_write_tasks(db_arc, write_rx).await;
        });

        service
    }

    /// Create a new memory service with custom pruning settings
    pub fn with_pruning_config(
        db: MemoryDb,
        embedding_provider: Box<dyn EmbeddingProvider>,
        max_memory_count: usize,
        compression_threshold_tokens: usize,
    ) -> Self {
        let vector_backend = BruteForceBackend::new();

        let (write_tx, write_rx) = mpsc::unbounded_channel();

        let db_arc = Arc::new(tokio::sync::Mutex::new(db));

        let service = Self {
            db: db_arc.clone(),
            embedding_provider,
            vector_backend: Arc::new(tokio::sync::Mutex::new(vector_backend)),
            write_tx,
            summarizer: MemorySummarizer::default(),
            max_memory_count,
            compression_threshold_tokens,
        };

        // Spawn background task processor
        tokio::spawn(async move {
            Self::process_write_tasks(db_arc, write_rx).await;
        });

        service
    }

    /// Background task processor for memory writes with proper error handling
    async fn process_write_tasks(
        db: Arc<tokio::sync::Mutex<MemoryDb>>,
        mut rx: mpsc::UnboundedReceiver<MemoryWriteTask>,
    ) {
        let mut consecutive_errors = 0u32;
        const MAX_CONSECUTIVE_ERRORS: u32 = 10;
        const ERROR_RESET_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
        let mut last_error_reset = std::time::Instant::now();

        while let Some(task) = rx.recv().await {
            // Reset error counter periodically to allow recovery
            if last_error_reset.elapsed() > ERROR_RESET_INTERVAL {
                consecutive_errors = 0;
                last_error_reset = std::time::Instant::now();
            }

            // Circuit breaker: if too many consecutive errors, log and skip
            if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                tracing::error!(
                    "Memory write processor circuit breaker open: {} consecutive errors. Skipping task.",
                    consecutive_errors
                );
                continue;
            }

            let result = match task {
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
                    db_guard.create_memory(&memory)
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
                        embedding: None,
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
                    db_guard.create_memory(&memory)
                }
            };

            match result {
                Ok(_) => {
                    consecutive_errors = 0; // Reset on success
                }
                Err(e) => {
                    consecutive_errors += 1;
                    tracing::error!(
                        "Memory write failed (error {}/{}): {}",
                        consecutive_errors,
                        MAX_CONSECUTIVE_ERRORS,
                        e
                    );
                }
            }
        }

        tracing::info!("Memory write task processor shutting down");
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
        self.write_tx
            .send(MemoryWriteTask::LogEpisode {
                content,
                user_id,
                project_id,
                conversation_id,
                metadata,
            })
            .context("Failed to queue episode write")?;
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
                if let Err(e) = self.update_memory_importance(&existing.id, importance_score) {
                    tracing::warn!("Failed to update memory importance for {}: {}", existing.id, e);
                    // Continue with creating new memory instead of failing silently
                } else {
                    return Ok(()); // Successfully updated existing memory
                }
            }
        }

        self.write_tx
            .send(MemoryWriteTask::CreateSemantic {
                content,
                kind,
                user_id,
                project_id,
                conversation_id,
                summary,
                importance_score,
                confidence_score,
                metadata,
            })
            .context("Failed to queue semantic write")?;
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

        if let Some(pid) = project_id {
            let results = db_guard.get_memories_by_project(pid)?;
            let recent: Vec<Memory> = results
                .into_iter()
                .filter(|m| m.kind == *kind)
                .take(5)
                .collect();
            for memory in recent {
                if self.content_similarity(content, &memory.content) > 0.7 {
                    return Ok(Some(memory));
                }
            }
        } else {
            let results = db_guard.get_memories_by_kind(kind.clone())?;
            let recent: Vec<Memory> = results.into_iter().take(5).collect();
            for memory in recent {
                if self.content_similarity(content, &memory.content) > 0.7 {
                    return Ok(Some(memory));
                }
            }
        }

        Ok(None)
    }

    /// Content similarity heuristic for deduplication
    fn content_similarity(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        let a_words: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
        let b_words: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();

        if a_words.is_empty() || b_words.is_empty() {
            return 0.0;
        }

        let intersection = a_words.intersection(&b_words).count();
        let union = a_words.union(&b_words).count();

        if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        }
    }

    /// Update memory importance score
    fn update_memory_importance(&self, id: &str, importance_score: f32) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        db_guard.update_memory_importance(id, importance_score)
    }

    /// Create a memory with embedding
    pub async fn create_memory(
        &self,
        content: String,
        kind: MemoryType,
        metadata: serde_json::Value,
    ) -> Result<String> {
        let kind = match kind {
            MemoryType::Episodic => MemoryKind::Episodic,
            MemoryType::Semantic => MemoryKind::Semantic,
            MemoryType::Procedural => MemoryKind::Semantic,
            MemoryType::Working => MemoryKind::Episodic,
        };

        let embedding = self.embedding_provider.embed(&content).await?;

        let memory = Memory {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind,
            content,
            summary: None,
            embedding: Some(embedding),
            importance_score: 0.5,
            confidence_score: 0.5,
            source: "api".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata,
        };

        let db_guard = self.db.lock().await;
        db_guard.create_memory(&memory)?;

        // Add to vector index
        if let Some(embedding) = memory.embedding {
            let mut backend = self.vector_backend.lock().await;
            let _ = backend.add_vector(memory.id.clone(), embedding).await;
        }

        Ok(memory.id)
    }

    /// Log an episode (episodic memory)
    pub async fn log_episode(
        &self,
        content: String,
        user_id: Option<String>,
        project_id: Option<String>,
        conversation_id: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<String> {
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

        let db_guard = self.db.lock().await;
        db_guard.create_memory(&memory)?;

        Ok(memory.id)
    }

    /// Get a memory by ID
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let db_guard = self.db.blocking_lock();
        db_guard.get_memory(id)
    }

    /// Semantic search for memories
    pub async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let query_embedding = self.embedding_provider.embed(query).await?;

        let backend = self.vector_backend.lock().await;
        let results = backend.search(&query_embedding, limit).await?;
        drop(backend);

        let db_guard = self.db.lock().await;
        let mut memories = Vec::new();

        for (id, _score) in results {
            if let Some(memory) = db_guard.get_memory(&id)? {
                memories.push(memory);
            }
        }

        Ok(memories)
    }

    /// Create a relationship between memories
    pub fn create_relationship(
        &self,
        relationship: &super::types::MemoryRelationship,
    ) -> Result<()> {
        let db_guard = self.db.blocking_lock();
        db_guard.create_relationship(relationship)
    }

    /// Get relationships for a memory
    pub fn get_relationships(
        &self,
        memory_id: &str,
    ) -> Result<Vec<super::types::MemoryRelationship>> {
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

    /// Prune memories based on configured thresholds
    pub async fn prune_memories(&self) -> Result<usize> {
        let db_guard = self.db.lock().await;
        // Get all memories by kind (semantic is the main one we care about)
        let all_memories =
            db_guard.get_memories_by_kind(crate::flow::memory::types::MemoryKind::Semantic)?;
        drop(db_guard);

        if all_memories.len() <= self.max_memory_count {
            return Ok(0);
        }

        // Prune using combined strategy
        let pruned = prune_combined(all_memories, self.max_memory_count, 0.3);
        let pruned_count = pruned.len();

        // Delete pruned memories from database with proper error handling
        let db_guard = self.db.lock().await;
        let mut failed_deletions = 0usize;
        for memory in pruned {
            if let Err(e) = db_guard.delete_memory(&memory.id) {
                tracing::error!("Failed to delete memory {} during pruning: {}", memory.id, e);
                failed_deletions += 1;
            }
        }

        if failed_deletions > 0 {
            tracing::warn!("Pruning completed with {} failed deletions out of {}", failed_deletions, pruned_count);
        }

        Ok(pruned_count - failed_deletions)
    }

    /// Compress memories if threshold exceeded
    pub async fn compress_if_needed(&self) -> Result<bool> {
        let db_guard = self.db.lock().await;
        let all_memories =
            db_guard.get_memories_by_kind(crate::flow::memory::types::MemoryKind::Semantic)?;

        // Track original IDs before compression
        let original_ids: Vec<String> = all_memories.iter().map(|m| m.id.clone()).collect();
        drop(db_guard);

        let summarizer = std::sync::Arc::new(self.summarizer.clone());

        if summarizer.should_compress(
            &all_memories,
            self.max_memory_count,
            self.compression_threshold_tokens,
        ) {
            let compressed = summarizer.compress(all_memories, 10).await?;

            // Delete original memories by their original IDs with proper error handling
            let db_guard = self.db.lock().await;
            let mut failed_deletions = 0usize;
            for id in &original_ids {
                if let Err(e) = db_guard.delete_memory(id) {
                    tracing::error!("Failed to delete original memory {} during compression: {}", id, e);
                    failed_deletions += 1;
                }
            }

            if failed_deletions > 0 {
                tracing::warn!("Compression: {} out of {} original memories failed to delete", failed_deletions, original_ids.len());
            }

            // Add compressed memories with their new IDs
            let mut failed_insertions = 0usize;
            for memory in compressed {
                if let Err(e) = db_guard.create_memory(&memory) {
                    tracing::error!("Failed to create compressed memory {}: {}", memory.id, e);
                    failed_insertions += 1;
                }
            }

            if failed_insertions > 0 {
                tracing::warn!("Compression: {} compressed memories failed to insert", failed_insertions);
            }

            // Return true if at least some compression happened
            Ok(failed_insertions < original_ids.len())
        } else {
            Ok(false)
        }
    }

    /// Get memory statistics
    pub async fn get_memory_stats(&self) -> Result<MemoryStats> {
        let db_guard = self.db.lock().await;
        let all_memories =
            db_guard.get_memories_by_kind(crate::flow::memory::types::MemoryKind::Semantic)?;
        drop(db_guard);

        let total_count = all_memories.len();
        let total_tokens: usize = all_memories
            .iter()
            .map(|m| crate::context::ContextBudgeter::estimate_tokens(&m.content))
            .sum();

        let ranked = rank_memories(all_memories);
        let avg_importance = if ranked.is_empty() {
            0.5
        } else {
            ranked.iter().map(|(_, s)| s.overall).sum::<f32>() / ranked.len() as f32
        };

        Ok(MemoryStats {
            total_count,
            total_tokens,
            avg_importance,
            max_count: self.max_memory_count,
            compression_threshold: self.compression_threshold_tokens,
        })
    }
}

/// Memory statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStats {
    pub total_count: usize,
    pub total_tokens: usize,
    pub avg_importance: f32,
    pub max_count: usize,
    pub compression_threshold: usize,
}
