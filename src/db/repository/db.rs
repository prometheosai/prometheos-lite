//! Database connection wrapper

use anyhow::Context;
use rusqlite::Connection;

use super::artifacts::ArtifactOperations;
use super::conversations::ConversationOperations;
use super::flow_runs::FlowRunOperations;
use super::messages::MessageOperations;
use super::projects::ProjectOperations;
use super::trait_def::Repository;

/// Database connection wrapper
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Create a new database connection and initialize schema
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path).context("Failed to open database connection")?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database for testing
    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> anyhow::Result<()> {
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create projects table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create conversations table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create messages table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS flow_runs (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create flow_runs table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                file_path TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create artifacts table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS interrupts (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                trace_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                expected_schema TEXT NOT NULL,
                status TEXT NOT NULL,
                decision TEXT,
                expires_at TEXT,
                created_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create interrupts table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS flow_snapshots (
                id TEXT PRIMARY KEY,
                flow_name TEXT NOT NULL,
                flow_version TEXT NOT NULL,
                source_hash TEXT NOT NULL,
                source_text TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create flow_snapshots table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS tool_outbox (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                trace_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                input_hash TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                completed_at TEXT,
                result_json TEXT
            )",
                [],
            )
            .context("Failed to create tool_outbox table")?;

        Ok(())
    }

    /// Get the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

// Implement Repository trait by delegating to operation traits
impl Repository for Db {
    fn create_project(
        &self,
        input: crate::db::models::CreateProject,
    ) -> anyhow::Result<crate::db::models::Project> {
        ProjectOperations::create_project(self, input)
    }

    fn get_projects(&self) -> anyhow::Result<Vec<crate::db::models::Project>> {
        ProjectOperations::get_projects(self)
    }

    fn get_project(&self, id: &str) -> anyhow::Result<Option<crate::db::models::Project>> {
        ProjectOperations::get_project(self, id)
    }

    fn create_conversation(
        &self,
        input: crate::db::models::CreateConversation,
    ) -> anyhow::Result<crate::db::models::Conversation> {
        ConversationOperations::create_conversation(self, input)
    }

    fn get_conversations(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Vec<crate::db::models::Conversation>> {
        ConversationOperations::get_conversations(self, project_id)
    }

    fn get_conversation(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<crate::db::models::Conversation>> {
        ConversationOperations::get_conversation(self, id)
    }

    fn create_message(
        &self,
        input: crate::db::models::CreateMessage,
    ) -> anyhow::Result<crate::db::models::Message> {
        MessageOperations::create_message(self, input)
    }

    fn get_messages(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Vec<crate::db::models::Message>> {
        MessageOperations::get_messages(self, conversation_id)
    }

    fn create_flow_run(&self, conversation_id: &str) -> anyhow::Result<crate::db::models::FlowRun> {
        FlowRunOperations::create_flow_run(self, conversation_id)
    }

    fn update_flow_run_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        FlowRunOperations::update_flow_run_status(self, id, status)
    }

    fn create_artifact(
        &self,
        run_id: &str,
        file_path: &str,
        content: &str,
    ) -> anyhow::Result<crate::db::models::Artifact> {
        ArtifactOperations::create_artifact(self, run_id, file_path, content)
    }
}
