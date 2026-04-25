//! Intent routing to appropriate handlers

use crate::intent::types::{Intent, Handler};

/// Intent router for routing intents to handlers
pub struct IntentRouter;

impl IntentRouter {
    /// Route an intent to the appropriate handler
    pub fn route(intent: Intent) -> Handler {
        Handler::from_intent(intent)
    }

    /// Check if intent requires direct LLM response
    pub fn is_direct_llm(intent: Intent) -> bool {
        matches!(intent, Intent::Conversation | Intent::Question | Intent::Ambiguous)
    }

    /// Check if intent requires code generation flow
    pub fn is_codegen_flow(intent: Intent) -> bool {
        matches!(intent, Intent::CodingTask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_routing() {
        assert_eq!(IntentRouter::route(Intent::Conversation), Handler::DirectLlm);
        assert!(IntentRouter::is_direct_llm(Intent::Conversation));
    }

    #[test]
    fn test_question_routing() {
        assert_eq!(IntentRouter::route(Intent::Question), Handler::DirectLlm);
        assert!(IntentRouter::is_direct_llm(Intent::Question));
    }

    #[test]
    fn test_coding_task_routing() {
        assert_eq!(IntentRouter::route(Intent::CodingTask), Handler::CodeGenFlow);
        assert!(IntentRouter::is_codegen_flow(Intent::CodingTask));
    }

    #[test]
    fn test_ambiguous_routing() {
        assert_eq!(IntentRouter::route(Intent::Ambiguous), Handler::DirectLlm);
        assert!(IntentRouter::is_direct_llm(Intent::Ambiguous));
    }
}
