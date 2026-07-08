//! Memory types and data structures

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
