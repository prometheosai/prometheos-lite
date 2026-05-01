//! Database connection wrapper

use anyhow::Context;
use rusqlite::Connection;

use super::artifacts::ArtifactOperations;
use super::conversations::ConversationOperations;
use super::flow_runs::FlowRunOperations;
use super::interrupts::InterruptOperations;
use super::messages::MessageOperations;
use super::outbox::OutboxOperations;
use super::projects::ProjectOperations;
use super::snapshots::FlowSnapshotOperations;
use super::trait_def::Repository;
use super::trust_policies::TrustPolicyOperations;

/// Database connection wrapper
pub struct Db {
    conn: Connection,
}

// SAFETY: Db is created per-request and not shared across threads.
// The Connection is only used within the request context.
unsafe impl Send for Db {}
unsafe impl Sync for Db {}

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
                work_context_id TEXT,
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

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS trust_policies (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL UNIQUE,
                trust_level TEXT NOT NULL,
                require_approval INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create trust_policies table")?;

        // V1.2 WorkContext tables
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS work_contexts (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                title TEXT NOT NULL,
                domain TEXT NOT NULL,
                domain_profile_id TEXT,
                context_type TEXT NOT NULL,
                project_id TEXT,
                conversation_id TEXT,
                parent_context_id TEXT,
                priority TEXT NOT NULL,
                due_at TEXT,
                goal TEXT NOT NULL,
                requirements TEXT,
                constraints TEXT,
                status TEXT NOT NULL,
                current_phase TEXT NOT NULL,
                blocked_reason TEXT,
                plan TEXT,
                approved_plan TEXT,
                artifacts TEXT,
                memory_refs TEXT,
                decisions TEXT,
                flow_runs TEXT,
                tool_trace TEXT,
                open_questions TEXT,
                autonomy_level TEXT NOT NULL,
                approval_policy TEXT NOT NULL,
                summary TEXT,
                completion_criteria TEXT,
                last_activity_at TEXT NOT NULL,
                metadata TEXT,
                execution_metadata TEXT,
                playbook_id TEXT,
                evaluation_result TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create work_contexts table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS work_context_events (
                id TEXT PRIMARY KEY,
                work_context_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create work_context_events table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS conversation_work_contexts (
                conversation_id TEXT NOT NULL,
                work_context_id TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE,
                PRIMARY KEY (conversation_id, work_context_id)
            )",
                [],
            )
            .context("Failed to create conversation_work_contexts table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS work_artifacts (
                id TEXT PRIMARY KEY,
                work_context_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                content TEXT NOT NULL,
                created_by TEXT NOT NULL,
                storage_type TEXT NOT NULL,
                file_path TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create work_artifacts table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS decisions (
                id TEXT PRIMARY KEY,
                work_context_id TEXT,
                description TEXT NOT NULL,
                chosen_option TEXT NOT NULL,
                alternatives TEXT NOT NULL,
                approved INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create decisions table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS execution_plans (
                work_context_id TEXT PRIMARY KEY,
                steps_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create execution_plans table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS work_domain_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                parent_domain TEXT,
                default_flows TEXT NOT NULL,
                artifact_kinds TEXT NOT NULL,
                approval_defaults TEXT NOT NULL,
                lifecycle_template_json TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create work_domain_profiles table")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS work_context_playbooks (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                domain_profile_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                preferred_flows TEXT NOT NULL,
                preferred_nodes TEXT NOT NULL,
                default_approval_policy TEXT NOT NULL,
                default_research_depth TEXT NOT NULL,
                default_creativity_level TEXT NOT NULL,
                evaluation_rules TEXT NOT NULL,
                success_patterns TEXT NOT NULL,
                failure_patterns TEXT NOT NULL,
                confidence REAL NOT NULL,
                usage_count INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create work_context_playbooks table")?;

        // Migration: Add new columns if they don't exist (for existing databases)
        self.conn
            .execute(
                "ALTER TABLE work_context_playbooks ADD COLUMN preferred_nodes TEXT",
                [],
            )
            .ok(); // Ignore error if column already exists

        self.conn
            .execute(
                "ALTER TABLE work_context_playbooks ADD COLUMN success_patterns TEXT",
                [],
            )
            .ok(); // Ignore error if column already exists

        self.conn
            .execute(
                "ALTER TABLE work_context_playbooks ADD COLUMN failure_patterns TEXT",
                [],
            )
            .ok(); // Ignore error if column already exists

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS playbook_usage_log (
                id TEXT PRIMARY KEY,
                playbook_id TEXT NOT NULL,
                work_context_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                domain TEXT NOT NULL,
                outcome TEXT NOT NULL,
                used_at TEXT NOT NULL,
                FOREIGN KEY (playbook_id) REFERENCES work_context_playbooks(id) ON DELETE CASCADE,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create playbook_usage_log table")?;

        // V1.5.2: Flow performance records table
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS flow_performance_records (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                work_context_id TEXT NOT NULL,
                success_score REAL NOT NULL,
                duration_ms INTEGER NOT NULL,
                token_cost REAL NOT NULL,
                revision_count INTEGER NOT NULL,
                executed_at TEXT NOT NULL,
                FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create flow_performance_records table")?;

        // Create index for efficient queries by work_context_id
        self.conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_flow_perf_work_context 
                 ON flow_performance_records(work_context_id)",
                [],
            )
            .ok(); // Ignore if already exists

        // Create index for efficient queries by flow_id
        self.conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_flow_perf_flow_id 
                 ON flow_performance_records(flow_id)",
                [],
            )
            .ok(); // Ignore if already exists

        // V1.5.2: Skills table for control panel metrics
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                capability_signature TEXT NOT NULL,
                success_rate REAL NOT NULL,
                avg_duration_ms INTEGER NOT NULL,
                usage_count INTEGER NOT NULL DEFAULT 0,
                tags TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
                [],
            )
            .context("Failed to create skills table")?;

        // V1.5.2: Playbook evolutions table for control panel metrics
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS playbook_evolutions (
                id TEXT PRIMARY KEY,
                playbook_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                parent_version INTEGER,
                mutation_strategy TEXT NOT NULL,
                performance_score REAL NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY (playbook_id) REFERENCES work_context_playbooks(id) ON DELETE CASCADE
            )",
                [],
            )
            .context("Failed to create playbook_evolutions table")?;

        // Create index for efficient queries by playbook_id
        self.conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_evolutions_playbook_id 
                 ON playbook_evolutions(playbook_id)",
                [],
            )
            .ok(); // Ignore if already exists

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

// Outbox and interrupt operations are available via trait implementations
// on Db (which implements AsDb)
