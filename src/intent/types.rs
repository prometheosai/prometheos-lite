//! Intent type definitions

use serde::{Deserialize, Serialize};

/// User intent classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Casual conversation, greetings
    Conversation,
    /// Information queries
    Question,
    /// Code generation, implementation tasks
    CodingTask,
    /// File modification requests
    FileEdit,
    /// Tool execution commands
    ToolAction,
    /// Project-level operations
    ProjectAction,
    /// Ambiguous - needs LLM classification
    Ambiguous,
}

impl Intent {
    /// Returns the display name for the intent
    pub fn display_name(&self) -> &'static str {
        match self {
            Intent::Conversation => "conversation",
            Intent::Question => "question",
            Intent::CodingTask => "coding_task",
            Intent::FileEdit => "file_edit",
            Intent::ToolAction => "tool_action",
            Intent::ProjectAction => "project_action",
            Intent::Ambiguous => "ambiguous",
        }
    }

    /// Parse intent from override command
    pub fn from_override(command: &str) -> Option<Self> {
        match command.to_lowercase().as_str() {
            "/run" | "/run flow" | "/flow" => Some(Intent::CodingTask),
            "/ask" | "/question" => Some(Intent::Question),
            "/chat" | "/conversation" => Some(Intent::Conversation),
            _ => None,
        }
    }
}

/// Result of intent classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentClassificationResult {
    /// The classified intent
    pub intent: Intent,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Reasoning for the classification
    pub reason: String,
    /// Whether this was an override
    pub override_: bool,
}

/// Handler type for routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// Direct LLM response
    DirectLlm,
    /// Full code generation flow
    CodeGenFlow,
    /// File editing handler
    FileEdit,
    /// Tool execution handler
    ToolExecution,
    /// Project operations handler
    ProjectAction,
}

impl Handler {
    /// Returns the handler type for a given intent
    pub fn from_intent(intent: Intent) -> Self {
        match intent {
            Intent::Conversation | Intent::Question => Handler::DirectLlm,
            Intent::CodingTask => Handler::CodeGenFlow,
            Intent::FileEdit => Handler::FileEdit,
            Intent::ToolAction => Handler::ToolExecution,
            Intent::ProjectAction => Handler::ProjectAction,
            Intent::Ambiguous => Handler::DirectLlm, // Default to direct LLM for ambiguous
        }
    }
}
