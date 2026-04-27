//! SQLite database manager for memory storage

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::types::{Memory, MemoryKind, MemoryRelationship};

/// SQLite database manager for memory storage
pub struct MemoryDb {
    pub conn: Arc<Mutex<Connection>>,
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
            .query_map(params![kind_str], |row| {
                let kind_str: String = row.get(4)?;
                let kind = match kind_str.as_str() {
                    "episodic" => MemoryKind::Episodic,
                    "semantic" => MemoryKind::Semantic,
                    "preference" => MemoryKind::Preference,
                    "decision" => MemoryKind::Decision,
                    "constraint" => MemoryKind::Constraint,
                    "project_fact" => MemoryKind::ProjectFact,
                    "behavior_pattern" => MemoryKind::BehaviorPattern,
                    _ => MemoryKind::Semantic,
                };

                let embedding_blob: Option<Vec<u8>> = row.get(7)?;
                let embedding = embedding_blob.map(|blob| {
                    blob.chunks(4)
                        .map(|chunk| {
                            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
                            f32::from_le_bytes(bytes)
                        })
                        .collect()
                });

                let metadata_json: String = row.get(15)?;
                let metadata = serde_json::from_str(&metadata_json).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    )
                })?;

                let created_at_str: String = row.get(11)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let updated_at_str: String = row.get(12)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let last_accessed_at: Option<String> = row.get(13)?;
                let last_accessed_at = last_accessed_at
                    .map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map_err(|e| {
                                rusqlite::Error::ToSqlConversionFailure(
                                    Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                                )
                            })
                            .map(|dt| dt.with_timezone(&Utc))
                    })
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?;

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
                    created_at,
                    updated_at,
                    last_accessed_at,
                    access_count: row.get(14)?,
                    metadata,
                })
            })
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
            .query_map(params![project_id], |row| {
                let kind_str: String = row.get(4)?;
                let kind = match kind_str.as_str() {
                    "episodic" => MemoryKind::Episodic,
                    "semantic" => MemoryKind::Semantic,
                    "preference" => MemoryKind::Preference,
                    "decision" => MemoryKind::Decision,
                    "constraint" => MemoryKind::Constraint,
                    "project_fact" => MemoryKind::ProjectFact,
                    "behavior_pattern" => MemoryKind::BehaviorPattern,
                    _ => MemoryKind::Semantic,
                };

                let embedding_blob: Option<Vec<u8>> = row.get(7)?;
                let embedding = embedding_blob.map(|blob| {
                    blob.chunks(4)
                        .map(|chunk| {
                            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
                            f32::from_le_bytes(bytes)
                        })
                        .collect()
                });

                let metadata_json: String = row.get(15)?;
                let metadata = serde_json::from_str(&metadata_json).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    )
                })?;

                let created_at_str: String = row.get(11)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let updated_at_str: String = row.get(12)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let last_accessed_at: Option<String> = row.get(13)?;
                let last_accessed_at = last_accessed_at
                    .map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map_err(|e| {
                                rusqlite::Error::ToSqlConversionFailure(
                                    Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                                )
                            })
                            .map(|dt| dt.with_timezone(&Utc))
                    })
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?;

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
                    created_at,
                    updated_at,
                    last_accessed_at,
                    access_count: row.get(14)?,
                    metadata,
                })
            })
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
            .query_map(params![conversation_id], |row| {
                let kind_str: String = row.get(4)?;
                let kind = match kind_str.as_str() {
                    "episodic" => MemoryKind::Episodic,
                    "semantic" => MemoryKind::Semantic,
                    "preference" => MemoryKind::Preference,
                    "decision" => MemoryKind::Decision,
                    "constraint" => MemoryKind::Constraint,
                    "project_fact" => MemoryKind::ProjectFact,
                    "behavior_pattern" => MemoryKind::BehaviorPattern,
                    _ => MemoryKind::Semantic,
                };

                let embedding_blob: Option<Vec<u8>> = row.get(7)?;
                let embedding = embedding_blob.map(|blob| {
                    blob.chunks(4)
                        .map(|chunk| {
                            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
                            f32::from_le_bytes(bytes)
                        })
                        .collect()
                });

                let metadata_json: String = row.get(15)?;
                let metadata = serde_json::from_str(&metadata_json).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    )
                })?;

                let created_at_str: String = row.get(11)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let updated_at_str: String = row.get(12)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let last_accessed_at: Option<String> = row.get(13)?;
                let last_accessed_at = last_accessed_at
                    .map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map_err(|e| {
                                rusqlite::Error::ToSqlConversionFailure(
                                    Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                                )
                            })
                            .map(|dt| dt.with_timezone(&Utc))
                    })
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?;

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
                    created_at,
                    updated_at,
                    last_accessed_at,
                    access_count: row.get(14)?,
                    metadata,
                })
            })
            .context("Failed to query memories by conversation")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Update a memory
    pub fn update_memory(&self, memory: &Memory) -> Result<()> {
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
            "UPDATE memories SET user_id = ?2, project_id = ?3, conversation_id = ?4, kind = ?5, content = ?6, summary = ?7, embedding = ?8, importance_score = ?9, confidence_score = ?10, source = ?11, updated_at = ?12, last_accessed_at = ?13, access_count = ?14, metadata = ?15 WHERE id = ?1",
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
                memory.updated_at.to_rfc3339(),
                memory.last_accessed_at.map(|t| t.to_rfc3339()),
                memory.access_count,
                metadata_json,
            ],
        ).context("Failed to update memory")?;

        Ok(())
    }

    /// Delete a memory
    pub fn delete_memory(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])
            .context("Failed to delete memory")?;

        Ok(())
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
        )
        .context("Failed to create relationship")?;

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
                let created_at_str: String = row.get(5)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);
                Ok(MemoryRelationship {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    target_id: row.get(2)?,
                    relationship_type: row.get(3)?,
                    strength: row.get(4)?,
                    created_at,
                })
            })
            .context("Failed to query relationships")?;

        let mut relationships = Vec::new();
        for row in rows {
            relationships.push(row.map_err(|e| anyhow::anyhow!(e))?);
        }
        Ok(relationships)
    }

    /// Search memories by content
    pub fn search_memories(&self, query: &str) -> Result<Vec<Memory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, user_id, project_id, conversation_id, kind, content, summary, embedding, importance_score, confidence_score, source, created_at, updated_at, last_accessed_at, access_count, metadata
                 FROM memories WHERE content LIKE ?1",
            )
            .context("Failed to prepare search_memories query")?;

        let search_pattern = format!("%{}%", query);
        let rows = stmt
            .query_map(params![search_pattern], |row| {
                let kind_str: String = row.get(4)?;
                let kind = match kind_str.as_str() {
                    "episodic" => MemoryKind::Episodic,
                    "semantic" => MemoryKind::Semantic,
                    "preference" => MemoryKind::Preference,
                    "decision" => MemoryKind::Decision,
                    "constraint" => MemoryKind::Constraint,
                    "project_fact" => MemoryKind::ProjectFact,
                    "behavior_pattern" => MemoryKind::BehaviorPattern,
                    _ => MemoryKind::Semantic,
                };

                let embedding_blob: Option<Vec<u8>> = row.get(7)?;
                let embedding = embedding_blob.map(|blob| {
                    blob.chunks(4)
                        .map(|chunk| {
                            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
                            f32::from_le_bytes(bytes)
                        })
                        .collect()
                });

                let metadata_json: String = row.get(15)?;
                let metadata = serde_json::from_str(&metadata_json).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    )
                })?;

                let created_at_str: String = row.get(11)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let updated_at_str: String = row.get(12)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?
                    .with_timezone(&Utc);

                let last_accessed_at: Option<String> = row.get(13)?;
                let last_accessed_at = last_accessed_at
                    .map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map_err(|e| {
                                rusqlite::Error::ToSqlConversionFailure(
                                    Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                                )
                            })
                            .map(|dt| dt.with_timezone(&Utc))
                    })
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                        )
                    })?;

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
                    created_at,
                    updated_at,
                    last_accessed_at,
                    access_count: row.get(14)?,
                    metadata,
                })
            })
            .context("Failed to search memories")?;

        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(|e| anyhow::anyhow!(e))?;
            memories.push(memory);
        }
        Ok(memories)
    }

    /// Update user model entry
    pub fn update_user_model(
        &self,
        user_id: &str,
        key: &str,
        value: serde_json::Value,
        confidence_score: f32,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let value_json = serde_json::to_string(&value).context("Failed to serialize value")?;

        conn.execute(
            "INSERT OR REPLACE INTO user_model (user_id, key, value, confidence_score, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                user_id,
                key,
                value_json,
                confidence_score,
                Utc::now().to_rfc3339(),
            ],
        )
        .context("Failed to update user model")?;

        Ok(())
    }

    /// Get user model entry
    pub fn get_user_model(&self, user_id: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        let mut stmt = conn
            .prepare("SELECT value FROM user_model WHERE user_id = ?1 AND key = ?2")
            .context("Failed to prepare get_user_model query")?;

        let mut rows = stmt
            .query(params![user_id, key])
            .context("Failed to query user model")?;

        if let Some(row) = rows.next()? {
            let value_json: String = row.get(0)?;
            let value = serde_json::from_str(&value_json).context("Failed to deserialize value")?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Update memory importance score
    pub fn update_memory_importance(&self, id: &str, importance_score: f32) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;

        conn.execute(
            "UPDATE memories SET importance_score = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, importance_score, Utc::now().to_rfc3339()],
        )
        .context("Failed to update memory importance")?;

        Ok(())
    }

    /// Helper function to convert a database row to a Memory struct
    pub fn row_to_memory(&self, row: &rusqlite::Row) -> Result<Memory> {
        let kind_str: String = row.get(4)?;
        let kind = match kind_str.as_str() {
            "episodic" => MemoryKind::Episodic,
            "semantic" => MemoryKind::Semantic,
            "preference" => MemoryKind::Preference,
            "decision" => MemoryKind::Decision,
            "constraint" => MemoryKind::Constraint,
            "project_fact" => MemoryKind::ProjectFact,
            "behavior_pattern" => MemoryKind::BehaviorPattern,
            _ => MemoryKind::Semantic, // Default fallback
        };

        let embedding_blob: Option<Vec<u8>> = row.get(7)?;
        let embedding = embedding_blob.map(|blob| {
            blob.chunks(4)
                .map(|chunk| {
                    let bytes: [u8; 4] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
                    f32::from_le_bytes(bytes)
                })
                .collect()
        });

        let metadata_json: String = row.get(15)?;
        let metadata = serde_json::from_str(&metadata_json)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize metadata: {}", e))?;

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
                .map_err(|e| anyhow::anyhow!("Failed to parse created_at: {}", e))?
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(12)?)
                .map_err(|e| anyhow::anyhow!("Failed to parse updated_at: {}", e))?
                .with_timezone(&Utc),
            last_accessed_at: row
                .get::<_, Option<String>>(13)?
                .map(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map_err(|e| anyhow::anyhow!("Failed to parse last_accessed_at: {}", e))
                        .map(|dt| dt.with_timezone(&Utc))
                })
                .transpose()?,
            access_count: row.get(14)?,
            metadata,
        })
    }
}
