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

        // Planning patterns (PRD, spec, design)
        if Self::is_planning_request(trimmed) {
            return Some(Intent::Planning);
        }

        // Approval commands (to continue with implementation after plan review)
        if Self::is_approval_command(trimmed) {
            return Some(Intent::Approval);
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
        let single_word_patterns = [
            "hi", "hello", "hey", "thanks", "ok", "okay", "sure", "alright", "bye", "goodbye",
            "later",
        ];
        let phrase_patterns = [
            "how are you",
            "how's it going",
            "thank you",
            "what can you do",
            "what are you",
            "who are you",
            "good morning",
            "good afternoon",
            "good evening",
            "see you",
        ];

        let words: Vec<&str> = message
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .collect();

        let has_single_word_match = single_word_patterns
            .iter()
            .any(|pattern| words.iter().any(|word| word == pattern));
        let has_phrase_match = phrase_patterns
            .iter()
            .any(|pattern| message.contains(pattern));

        // Only treat short non-action messages as conversation when they match
        // explicit conversational patterns.
        (is_short && !Self::has_action_verb(message)) && (has_single_word_match || has_phrase_match)
    }

    /// Check if message is a question
    fn is_question(message: &str) -> bool {
        let question_words = [
            "what",
            "how",
            "why",
            "when",
            "where",
            "who",
            "which",
            "can you",
            "could you",
            "would you",
            "should i",
            "is it",
            "are there",
            "do you",
            "does it",
            "explain",
            "describe",
            "tell me about",
        ];

        question_words
            .iter()
            .any(|&word| message.starts_with(word) || message.contains(word))
    }

    /// Check if message is a coding task
    fn is_coding_task(message: &str) -> bool {
        // Tool-oriented command requests should route to ToolAction, not CodingTask.
        if Self::is_tool_action(message) {
            return false;
        }

        // First check if it's a planning request (PRD, design, spec)
        let planning_patterns = [
            "prd",
            "product requirement",
            "specification",
            "spec",
            "design document",
            "architecture",
            "plan",
            "planning",
        ];
        if planning_patterns
            .iter()
            .any(|&pattern| message.to_lowercase().contains(pattern))
        {
            return false;
        }

        let coding_verbs = [
            "build",
            "create",
            "add",
            "implement",
            "generate",
            "make",
            "write code",
            "code",
            "program",
            "develop",
            "fix",
            "debug",
            "solve",
            "resolve",
            "refactor",
            "optimize",
            "improve",
            "add feature",
            "add function",
            "add method",
            "remove",
            "delete",
            "change",
        ];

        let software_nouns = [
            "app",
            "application",
            "api",
            "function",
            "method",
            "class",
            "module",
            "component",
            "service",
            "endpoint",
            "route",
            "handler",
            "controller",
            "database",
            "model",
            "schema",
            "migration",
            "feature",
            "bug",
            "test",
            "spec",
            "test case",
            "rust",
            "python",
            "javascript",
            "typescript",
            "react",
            "vue",
            "angular",
            "svelte",
            "axum",
            "actix",
            "rocket",
            "express",
        ];

        let has_coding_verb = coding_verbs.iter().any(|&verb| message.contains(verb));
        let has_software_noun = software_nouns.iter().any(|&noun| message.contains(noun));

        has_coding_verb || (has_software_noun && Self::has_action_verb(message))
    }

    /// Check if message is a file edit request
    fn is_file_edit(message: &str) -> bool {
        let edit_patterns = [
            "edit",
            "modify",
            "change",
            "update",
            "in file",
            "in the file",
            "in src/",
            "line",
            "replace",
            "insert",
            "append",
        ];

        edit_patterns
            .iter()
            .any(|&pattern| message.contains(pattern))
    }

    /// Check if message is a tool action
    fn is_tool_action(message: &str) -> bool {
        let words: Vec<&str> = message
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .filter(|w| !w.is_empty())
            .collect();

        if words.is_empty() {
            return false;
        }

        let command_verbs = [
            "run", "execute", "build", "test", "check", "compile", "deploy", "install",
        ];
        let tool_nouns = [
            "cargo", "npm", "yarn", "pnpm", "pip", "gradle", "docker", "kubectl", "git", "mvn",
            "make", "cmake", "pytest", "jest", "vitest", "go", "rustc",
        ];
        let explicit_tool_phrases = [
            "run tests",
            "run the tests",
            "execute tests",
            "execute command",
            "run command",
            "build project",
            "compile project",
            "cargo test",
            "cargo build",
            "cargo check",
            "npm test",
            "npm run",
            "pip install",
            "docker build",
            "git status",
        ];

        if explicit_tool_phrases
            .iter()
            .any(|phrase| message.contains(phrase))
        {
            return true;
        }

        let starts_with_command = words
            .first()
            .map(|word| command_verbs.contains(word))
            .unwrap_or(false);
        let has_tool_noun = words.iter().any(|word| tool_nouns.contains(word));

        starts_with_command && has_tool_noun
    }

    /// Check if message is a project action
    fn is_project_action(message: &str) -> bool {
        let project_patterns = [
            "create a new project",
            "initialize a project",
            "set up a new project",
            "start a new project",
        ];

        project_patterns
            .iter()
            .any(|&pattern| message.contains(pattern))
    }

    /// Check if message is a planning request
    fn is_planning_request(message: &str) -> bool {
        let planning_patterns = [
            "prd",
            "product requirement",
            "specification",
            "spec",
            "design document",
            "architecture",
            "plan",
            "planning",
            "create a prd",
            "write a prd",
            "generate a prd",
            "create a spec",
            "write a spec",
        ];

        planning_patterns
            .iter()
            .any(|&pattern| message.to_lowercase().contains(pattern))
    }

    /// Check if message is an approval command (to continue with implementation)
    fn is_approval_command(message: &str) -> bool {
        let approval_patterns = [
            "implement this plan",
            "implement the plan",
            "continue",
            "proceed",
            "go ahead",
            "yes implement",
            "yes continue",
            "approve",
            "approved",
        ];

        approval_patterns
            .iter()
            .any(|&pattern| message.to_lowercase().contains(pattern))
    }

    /// Check if message has an action verb
    fn has_action_verb(message: &str) -> bool {
        let action_verbs = [
            "build",
            "create",
            "make",
            "add",
            "remove",
            "delete",
            "fix",
            "change",
            "update",
            "modify",
            "edit",
            "run",
            "execute",
            "implement",
            "generate",
            "write",
            "code",
            "develop",
            "deploy",
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
        assert_eq!(
            RuleClassifier::classify("hello"),
            Some(Intent::Conversation)
        );
        assert_eq!(
            RuleClassifier::classify("how are you"),
            Some(Intent::Conversation)
        );
        assert_eq!(
            RuleClassifier::classify("thanks"),
            Some(Intent::Conversation)
        );
    }

    #[test]
    fn test_question_classification() {
        assert_eq!(
            RuleClassifier::classify("what is Rust?"),
            Some(Intent::Question)
        );
        assert_eq!(
            RuleClassifier::classify("how do I use axum?"),
            Some(Intent::Question)
        );
        assert_eq!(
            RuleClassifier::classify("explain this"),
            Some(Intent::Question)
        );
    }

    #[test]
    fn test_coding_task_classification() {
        assert_eq!(
            RuleClassifier::classify("create a REST API"),
            Some(Intent::CodingTask)
        );
        assert_eq!(
            RuleClassifier::classify("fix this bug"),
            Some(Intent::CodingTask)
        );
        assert_eq!(
            RuleClassifier::classify("implement a function"),
            Some(Intent::CodingTask)
        );
        assert_eq!(
            RuleClassifier::classify("add a new feature"),
            Some(Intent::CodingTask)
        );
    }

    #[test]
    fn test_ambiguous_classification() {
        assert_eq!(RuleClassifier::classify("something random"), None);
        assert_eq!(RuleClassifier::classify("xyz"), None);
        assert_eq!(RuleClassifier::classify("test message"), None);
    }

    #[test]
    fn test_tool_action_classification() {
        assert_eq!(
            RuleClassifier::classify("run cargo test"),
            Some(Intent::ToolAction)
        );
        assert_eq!(
            RuleClassifier::classify("execute command git status"),
            Some(Intent::ToolAction)
        );
    }
}
