//! Database repository for UI state
//!
//! This module provides the database connection and repository methods
//! for CRUD operations on projects, conversations, and messages.

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::models::{
    Artifact, Conversation, Conversation as ConversationModel, CreateConversation,
    CreateMessage, CreateProject, FlowRun, Message, Message as MessageModel, Project,
    Project as ProjectModel,
};

/// Database connection wrapper
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Create a new database connection and initialize schema
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)
            .context("Failed to open database connection")?;
        
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database for testing
    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;
        
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> anyhow::Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        ).context("Failed to create projects table")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        ).context("Failed to create conversations table")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            )",
            [],
        ).context("Failed to create messages table")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS flow_runs (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            )",
            [],
        ).context("Failed to create flow_runs table")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                file_path TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE CASCADE
            )",
            [],
        ).context("Failed to create artifacts table")?;

        Ok(())
    }
}

/// Repository trait for database operations
pub trait Repository {
    // Project operations
    fn create_project(&self, input: CreateProject) -> anyhow::Result<Project>;
    fn get_projects(&self) -> anyhow::Result<Vec<Project>>;
    fn get_project(&self, id: &str) -> anyhow::Result<Option<Project>>;

    // Conversation operations
    fn create_conversation(&self, input: CreateConversation) -> anyhow::Result<Conversation>;
    fn get_conversations(&self, project_id: &str) -> anyhow::Result<Vec<Conversation>>;
    fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>>;

    // Message operations
    fn create_message(&self, input: CreateMessage) -> anyhow::Result<Message>;
    fn get_messages(&self, conversation_id: &str) -> anyhow::Result<Vec<Message>>;

    // FlowRun operations
    fn create_flow_run(&self, conversation_id: &str) -> anyhow::Result<FlowRun>;
    fn update_flow_run_status(&self, id: &str, status: &str) -> anyhow::Result<()>;

    // Artifact operations
    fn create_artifact(&self, run_id: &str, file_path: &str, content: &str) -> anyhow::Result<Artifact>;
}

impl Repository for Db {
    fn create_project(&self, input: CreateProject) -> anyhow::Result<Project> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        self.conn.execute(
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![&id, &input.name, &now.to_rfc3339(), &now.to_rfc3339()],
        ).context("Failed to insert project")?;

        Ok(Project {
            id,
            name: input.name,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_projects(&self) -> anyhow::Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, created_at, updated_at FROM projects ORDER BY created_at DESC"
        ).context("Failed to prepare projects query")?;

        let projects = stmt.query_map([], |row| {
            let created_str: String = row.get(2)?;
            let updated_str: String = row.get(3)?;
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).context("Failed to query projects")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to collect projects")?;

        Ok(projects)
    }

    fn get_project(&self, id: &str) -> anyhow::Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, created_at, updated_at FROM projects WHERE id = ?1"
        ).context("Failed to prepare project query")?;

        let project = stmt.query_row(params![id], |row| {
            let created_str: String = row.get(2)?;
            let updated_str: String = row.get(3)?;
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).optional().context("Failed to query project")?;

        Ok(project)
    }

    fn create_conversation(&self, input: CreateConversation) -> anyhow::Result<Conversation> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        self.conn.execute(
            "INSERT INTO conversations (id, project_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, &input.project_id, &input.title, &now.to_rfc3339(), &now.to_rfc3339()],
        ).context("Failed to insert conversation")?;

        Ok(Conversation {
            id,
            project_id: input.project_id,
            title: input.title,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_conversations(&self, project_id: &str) -> anyhow::Result<Vec<Conversation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, created_at, updated_at FROM conversations WHERE project_id = ?1 ORDER BY created_at DESC"
        ).context("Failed to prepare conversations query")?;

        let conversations = stmt.query_map(params![project_id], |row| {
            let created_str: String = row.get(3)?;
            let updated_str: String = row.get(4)?;
            Ok(Conversation {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).context("Failed to query conversations")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to collect conversations")?;

        Ok(conversations)
    }

    fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, created_at, updated_at FROM conversations WHERE id = ?1"
        ).context("Failed to prepare conversation query")?;

        let conversation = stmt.query_row(params![id], |row| {
            let created_str: String = row.get(3)?;
            let updated_str: String = row.get(4)?;
            Ok(Conversation {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).optional().context("Failed to query conversation")?;

        Ok(conversation)
    }

    fn create_message(&self, input: CreateMessage) -> anyhow::Result<Message> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, &input.conversation_id, &input.role, &input.content, &now.to_rfc3339()],
        ).context("Failed to insert message")?;

        Ok(Message {
            id,
            conversation_id: input.conversation_id,
            role: input.role,
            content: input.content,
            created_at: now,
        })
    }

    fn get_messages(&self, conversation_id: &str) -> anyhow::Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, conversation_id, role, content, created_at FROM messages WHERE conversation_id = ?1 ORDER BY created_at ASC"
        ).context("Failed to prepare messages query")?;

        let messages = stmt.query_map(params![conversation_id], |row| {
            let created_str: String = row.get(4)?;
            Ok(Message {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).context("Failed to query messages")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to collect messages")?;

        Ok(messages)
    }

    // FlowRun operations
    fn create_flow_run(&self, conversation_id: &str) -> anyhow::Result<FlowRun> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        self.conn.execute(
            "INSERT INTO flow_runs (id, conversation_id, status, started_at, completed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, conversation_id, "running", &now.to_rfc3339(), None::<String>],
        ).context("Failed to insert flow run")?;

        Ok(FlowRun {
            id,
            conversation_id: conversation_id.to_string(),
            status: "running".to_string(),
            started_at: now,
            completed_at: None,
        })
    }

    fn update_flow_run_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        let completed_at = if status == "completed" || status == "failed" {
            Some(Utc::now().to_rfc3339())
        } else {
            None
        };

        if let Some(at) = completed_at {
            self.conn.execute(
                "UPDATE flow_runs SET status = ?1, completed_at = ?2 WHERE id = ?3",
                params![status, at, id],
            ).context("Failed to update flow run")?;
        } else {
            self.conn.execute(
                "UPDATE flow_runs SET status = ?1 WHERE id = ?2",
                params![status, id],
            ).context("Failed to update flow run")?;
        }

        Ok(())
    }

    // Artifact operations
    fn create_artifact(&self, run_id: &str, file_path: &str, content: &str) -> anyhow::Result<Artifact> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        self.conn.execute(
            "INSERT INTO artifacts (id, run_id, file_path, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, run_id, file_path, content, &now.to_rfc3339()],
        ).context("Failed to insert artifact")?;

        Ok(Artifact {
            id,
            run_id: run_id.to_string(),
            file_path: file_path.to_string(),
            content: content.to_string(),
            created_at: now,
        })
    }
}
