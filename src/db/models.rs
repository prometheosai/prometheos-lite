//! Database models for UI state
//!
//! This module defines the data models for projects, conversations, messages,
//! flow runs, and artifacts stored in SQLite.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Project - a container for conversations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Conversation - a chat session within a project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Message - a single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// FlowRun - tracks execution of a flow for a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRun {
    pub id: String,
    pub conversation_id: String,
    pub status: String, // "running", "completed", "failed"
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Artifact - a file generated during flow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub run_id: String,
    pub file_path: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Input types for creating new records
#[derive(Debug, Deserialize)]
pub struct CreateProject {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateConversation {
    pub project_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessage {
    pub conversation_id: String,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RunFlow {
    pub message: String,
}
