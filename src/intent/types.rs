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
    /// Planning requests (PRD, spec, design)
    Planning,
    /// Approval command (to continue with implementation)
    Approval,
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
            Intent::Planning => "planning",
            Intent::Approval => "approval",
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
    /// WorkOrchestrator for persistent work contexts
    WorkOrchestrator,
    /// File editing handler
    FileEdit,
    /// Tool execution handler
    ToolExecution,
    /// Project operations handler
    ProjectAction,
    /// Planning handler (generates plan only, waits for approval)
    Planning,
    /// Approval handler (continues with implementation after plan approval)
    Approval,
}

impl Handler {
    /// Returns the handler type for a given intent
    pub fn from_intent(intent: Intent) -> Self {
        match intent {
            Intent::Conversation | Intent::Question => Handler::DirectLlm,
            Intent::CodingTask => Handler::WorkOrchestrator, // Route to WorkOrchestrator for persistent work
            Intent::FileEdit => Handler::FileEdit,
            Intent::ToolAction => Handler::ToolExecution,
            Intent::ProjectAction => Handler::ProjectAction,
            Intent::Planning => Handler::Planning,
            Intent::Approval => Handler::WorkOrchestrator, // Approval continues with WorkOrchestrator
            Intent::Ambiguous => Handler::DirectLlm,       // Default to direct LLM for ambiguous
        }
    }
}
