//! Repository trait for database operations

use crate::db::models::{Artifact, Conversation, CreateConversation, CreateMessage, CreateProject, FlowRun, Message, Project};

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
