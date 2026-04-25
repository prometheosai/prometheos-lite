//! Rule-based intent classification

use crate::intent::types::Intent;

/// Rule-based classifier for obvious intent patterns
pub struct RuleClassifier;

impl RuleClassifier {
    /// Classify intent using rule-based patterns
    /// Returns Some(Intent) if a rule matches, None if ambiguous
    pub fn classify(message: &str) -> Option<Intent> {
        let lower = message.to_lowercase();
        let trimmed = lower.trim();

        // Character count heuristic
        let is_short = trimmed.len() < 80;

        // Conversation patterns
        if Self::is_conversation(trimmed, is_short) {
            return Some(Intent::Conversation);
        }

        // Question patterns
        if Self::is_question(trimmed) {
            return Some(Intent::Question);
        }

        // Coding task patterns
        if Self::is_coding_task(trimmed) {
            return Some(Intent::CodingTask);
        }

        // File edit patterns
        if Self::is_file_edit(trimmed) {
            return Some(Intent::FileEdit);
        }

        // Tool action patterns
        if Self::is_tool_action(trimmed) {
            return Some(Intent::ToolAction);
        }

        // Project action patterns
        if Self::is_project_action(trimmed) {
            return Some(Intent::ProjectAction);
        }

        // No rule matched - ambiguous
        None
    }

    /// Check if message is a conversation
    fn is_conversation(message: &str, is_short: bool) -> bool {
        let conversation_patterns = [
            "hi", "hello", "hey", "how are you", "how's it going",
            "thanks", "thank you", "ok", "okay", "sure", "alright",
            "what can you do", "what are you", "who are you",
            "good morning", "good afternoon", "good evening",
            "bye", "goodbye", "see you", "later",
        ];

        // Short messages without action verbs are likely conversation
        if is_short && !Self::has_action_verb(message) {
            return true;
        }

        conversation_patterns.iter().any(|&pattern| message.contains(pattern))
    }

    /// Check if message is a question
    fn is_question(message: &str) -> bool {
        let question_words = [
            "what", "how", "why", "when", "where", "who", "which",
            "can you", "could you", "would you", "should i",
            "is it", "are there", "do you", "does it",
            "explain", "describe", "tell me about",
        ];

        question_words.iter().any(|&word| message.starts_with(word) || message.contains(word))
    }

    /// Check if message is a coding task
    fn is_coding_task(message: &str) -> bool {
        let coding_verbs = [
            "build", "create", "implement", "generate", "make",
            "write code", "code", "program", "develop",
            "fix", "debug", "solve", "resolve",
            "refactor", "optimize", "improve",
            "add feature", "add function", "add method",
            "remove", "delete", "change",
        ];

        let software_nouns = [
            "app", "application", "api", "function", "method",
            "class", "module", "component", "service",
            "endpoint", "route", "handler", "controller",
            "database", "model", "schema", "migration",
            "test", "spec", "test case",
            "rust", "python", "javascript", "typescript",
            "react", "vue", "angular", "svelte",
            "axum", "actix", "rocket", "express",
        ];

        let has_coding_verb = coding_verbs.iter().any(|&verb| message.contains(verb));
        let has_software_noun = software_nouns.iter().any(|&noun| message.contains(noun));

        has_coding_verb || (has_software_noun && Self::has_action_verb(message))
    }

    /// Check if message is a file edit request
    fn is_file_edit(message: &str) -> bool {
        let edit_patterns = [
            "edit", "modify", "change", "update",
            "in file", "in the file", "in src/",
            "line", "replace", "insert", "append",
        ];

        edit_patterns.iter().any(|&pattern| message.contains(pattern))
    }

    /// Check if message is a tool action
    fn is_tool_action(message: &str) -> bool {
        let tool_patterns = [
            "run", "execute", "build", "test", "check",
            "compile", "deploy", "install", "cargo",
            "npm", "yarn", "pip", "gradle",
            "docker", "kubectl", "git",
        ];

        tool_patterns.iter().any(|&pattern| message.contains(pattern))
    }

    /// Check if message is a project action
    fn is_project_action(message: &str) -> bool {
        let project_patterns = [
            "new project", "create project", "init project",
            "project structure", "project setup",
            "add to project", "remove from project",
        ];

        project_patterns.iter().any(|&pattern| message.contains(pattern))
    }

    /// Check if message has an action verb
    fn has_action_verb(message: &str) -> bool {
        let action_verbs = [
            "build", "create", "make", "add", "remove", "delete",
            "fix", "change", "update", "modify", "edit",
            "run", "execute", "implement", "generate",
            "write", "code", "develop", "deploy",
        ];

        action_verbs.iter().any(|&verb| message.contains(verb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_classification() {
        assert_eq!(RuleClassifier::classify("hi"), Some(Intent::Conversation));
        assert_eq!(RuleClassifier::classify("hello"), Some(Intent::Conversation));
        assert_eq!(RuleClassifier::classify("how are you"), Some(Intent::Conversation));
        assert_eq!(RuleClassifier::classify("thanks"), Some(Intent::Conversation));
    }

    #[test]
    fn test_question_classification() {
        assert_eq!(RuleClassifier::classify("what is Rust?"), Some(Intent::Question));
        assert_eq!(RuleClassifier::classify("how do I use axum?"), Some(Intent::Question));
        assert_eq!(RuleClassifier::classify("explain this"), Some(Intent::Question));
    }

    #[test]
    fn test_coding_task_classification() {
        assert_eq!(RuleClassifier::classify("create a REST API"), Some(Intent::CodingTask));
        assert_eq!(RuleClassifier::classify("fix this bug"), Some(Intent::CodingTask));
        assert_eq!(RuleClassifier::classify("implement a function"), Some(Intent::CodingTask));
        assert_eq!(RuleClassifier::classify("add a new feature"), Some(Intent::CodingTask));
    }

    #[test]
    fn test_ambiguous_classification() {
        assert_eq!(RuleClassifier::classify("something random"), None);
        assert_eq!(RuleClassifier::classify("xyz"), None);
    }
}
