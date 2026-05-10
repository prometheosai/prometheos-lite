//! Context Budgeter - Token budget management and context trimming
//!
//! This module implements token estimation, priority-based context allocation,
//! and intelligent context trimming to prevent LLM calls from exceeding token limits.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context item with priority for trimming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub content: String,
    pub priority: ContextPriority,
    pub label: String,
}

/// Priority levels for context items
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ContextPriority {
    /// System prompt - never trim
    System = 0,
    /// Task description - highest priority
    Task = 1,
    /// Plan - high priority
    Plan = 2,
    /// Critical memory - high priority
    CriticalMemory = 3,
    /// Recent artifacts - medium priority
    RecentArtifacts = 4,
    /// Long-tail memory - low priority
    LongTailMemory = 5,
}

/// Trimmed context result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimmedContext {
    /// The trimmed prompt text
    pub prompt: String,
    /// Items that were dropped due to budget constraints
    pub dropped_items: Vec<String>,
    /// Total token count of the trimmed context
    pub token_count: usize,
}

/// Context budget configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudgeter {
    /// Maximum total tokens for the context (including system prompt, task, memory, artifacts)
    pub max_tokens: usize,
    /// Reserved tokens for LLM output (not used for input context)
    pub reserved_output_tokens: usize,
}

impl Default for ContextBudgeter {
    fn default() -> Self {
        Self {
            max_tokens: 128_000, // Default for modern LLMs
            reserved_output_tokens: 4_096,
        }
    }
}

impl ContextBudgeter {
    /// Create a new ContextBudgeter with custom limits
    pub fn new(max_tokens: usize, reserved_output_tokens: usize) -> Self {
        Self {
            max_tokens,
            reserved_output_tokens,
        }
    }

    /// Get the available tokens for input context
    pub fn available_input_tokens(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserved_output_tokens)
    }

    /// Estimate token count for text (rough approximation: ~4 chars per token)
    pub fn estimate_tokens(text: &str) -> usize {
        // Rough approximation: ~4 characters per token for English text.
        // Use ceil division to avoid systematically undercounting short strings.
        let char_count = text.chars().count();
        char_count.div_ceil(4).max(1)
    }

    /// Estimate token count for JSON value
    pub fn estimate_tokens_json(value: &serde_json::Value) -> usize {
        Self::estimate_tokens(&value.to_string())
    }

    /// Build context with priority-based trimming
    ///
    /// Rules:
    /// - System items are NEVER dropped - they are hard-preserved (error if too large)
    /// - Truncate lowest priority items first (after System)
    /// - Preserve structural integrity (never cut mid-JSON/code block)
    /// - Return dropped items for logging
    pub fn build_context(&self, items: Vec<ContextItem>) -> Result<TrimmedContext> {
        let available_tokens = self.available_input_tokens();

        // Separate System items (hard-preserved) from other items
        let (system_items, other_items): (Vec<_>, Vec<_>) = items
            .into_iter()
            .enumerate()
            .partition(|(_, item)| item.priority == ContextPriority::System);

        // Calculate System tokens (these MUST fit)
        let system_tokens: usize = system_items
            .iter()
            .map(|(_, item)| Self::estimate_tokens(&item.content))
            .sum();

        // HARD PRESERVATION: System items must never be dropped
        // If System items alone exceed budget, this is a configuration error
        if system_tokens > available_tokens {
            anyhow::bail!(
                "System prompt exceeds available token budget: {} tokens required, {} available. \
                 Consider increasing max_tokens or reducing system prompt size.",
                system_tokens,
                available_tokens
            );
        }

        // Calculate remaining budget for non-System items
        let remaining_tokens = available_tokens - system_tokens;

        // Sort non-System items by priority (lower = higher priority)
        let mut sorted_items: Vec<_> = other_items;
        sorted_items.sort_by_key(|(_, item)| item.priority);

        // Calculate non-System tokens
        let other_tokens: usize = sorted_items
            .iter()
            .map(|(_, item)| Self::estimate_tokens(&item.content))
            .sum();

        // If everything fits, return as-is
        if system_tokens + other_tokens <= available_tokens {
            let mut all_items = system_items;
            all_items.extend(sorted_items);
            // Restore original order
            all_items.sort_by_key(|(index, _)| *index);

            let prompt = all_items
                .into_iter()
                .map(|(_, item)| item.content)
                .collect::<Vec<_>>()
                .join("\n\n");

            return Ok(TrimmedContext {
                prompt,
                dropped_items: Vec::new(),
                token_count: system_tokens + other_tokens,
            });
        }

        // Need to trim non-System items - System items are always kept
        let mut kept_items = system_items.clone();
        let mut dropped_items = Vec::new();
        let mut current_tokens = system_tokens;

        // Process non-System items in priority order (highest first)
        for (original_index, item) in sorted_items.into_iter() {
            let item_tokens = Self::estimate_tokens(&item.content);

            if current_tokens + item_tokens <= available_tokens {
                // Keep this item
                kept_items.push((original_index, item));
                current_tokens += item_tokens;
            } else {
                // Budget exceeded - drop this item
                dropped_items.push(item.label);
            }
        }

        // Restore original order
        kept_items.sort_by_key(|(index, _)| *index);

        // Build final prompt (System items guaranteed to be included)
        let prompt = kept_items
            .into_iter()
            .map(|(_, item)| item.content)
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(TrimmedContext {
            prompt,
            dropped_items,
            token_count: current_tokens,
        })
    }

    /// Trim content while preserving structural integrity
    ///
    /// Trim content to fit within token limit, preserving JSON/code blocks
    pub fn trim_content(&self, content: &str, max_tokens: usize) -> Result<String> {
        let max_chars = max_tokens * 4; // Approximate character limit

        if content.len() <= max_chars {
            return Ok(content.to_string());
        }

        // Check if content is JSON - use intelligent truncation
        if serde_json::from_str::<serde_json::Value>(content).is_ok() {
            return self.truncate_json(content, max_chars);
        }

        // Check for code blocks
        if content.contains("```") {
            // Find code block boundaries
            let lines: Vec<&str> = content.lines().collect();
            let mut in_code_block = false;
            let mut result_lines = Vec::new();
            let mut current_chars = 0;

            for line in lines {
                if line.trim().starts_with("```") {
                    in_code_block = !in_code_block;
                    if current_chars + line.len() > max_chars {
                        // Close the code block if we're in one
                        if in_code_block {
                            result_lines.push("```");
                        }
                        break;
                    }
                }

                if current_chars + line.len() > max_chars {
                    if in_code_block {
                        result_lines.push("```");
                    }
                    break;
                }

                result_lines.push(line);
                current_chars += line.len() + 1; // +1 for newline
            }

            return Ok(result_lines.join("\n"));
        }

        // Simple text truncation
        Ok(content[..max_chars.min(content.len())].to_string())
    }

    /// Calculate context budget report
    pub fn budget_report(&self, items: &[ContextItem]) -> HashMap<String, usize> {
        let mut report = HashMap::new();

        let total_tokens: usize = items
            .iter()
            .map(|item| Self::estimate_tokens(&item.content))
            .sum();

        let available = self.available_input_tokens();
        let usage_percentage = if available > 0 {
            (total_tokens * 100) / available
        } else {
            100
        };

        report.insert("total_tokens".to_string(), total_tokens);
        report.insert("available_tokens".to_string(), available);
        report.insert("usage_percentage".to_string(), usage_percentage);
        report.insert("max_tokens".to_string(), self.max_tokens);
        report.insert("reserved_output".to_string(), self.reserved_output_tokens);

        report
    }

    /// Intelligently truncate JSON content while preserving structure
    fn truncate_json(&self, content: &str, max_chars: usize) -> Result<String> {
        // First try simple truncation and re-parse
        let truncated = &content[..max_chars.min(content.len())];
        if serde_json::from_str::<serde_json::Value>(truncated).is_ok() {
            return Ok(truncated.to_string());
        }

        // Try to parse and intelligently truncate
        match serde_json::from_str::<serde_json::Value>(content) {
            Ok(json_value) => {
                let truncated_value = self.truncate_json_value(&json_value, max_chars);
                serde_json::to_string(&truncated_value)
                    .map_err(|e| anyhow::anyhow!("Failed to serialize truncated JSON: {}", e))
            }
            Err(_) => {
                // If we can't parse it, truncate with a note
                let available = max_chars.saturating_sub(50);
                let truncated = &content[..available.min(content.len())];
                Ok(format!("{}... [truncated due to size]", truncated))
            }
        }
    }

    /// Recursively truncate JSON value to fit within character limit
    fn truncate_json_value(
        &self,
        value: &serde_json::Value,
        max_chars: usize,
    ) -> serde_json::Value {
        let json_str = value.to_string();
        if json_str.len() <= max_chars {
            return value.clone();
        }

        match value {
            serde_json::Value::Array(arr) => {
                // Truncate arrays by keeping first N items
                let item_limit = (max_chars / 50).max(3); // Estimate ~50 chars per item, keep at least 3
                let truncated: Vec<serde_json::Value> =
                    arr.iter().take(item_limit).cloned().collect();

                if truncated.len() < arr.len() {
                    let mut result = truncated;
                    result.push(serde_json::Value::String(format!(
                        "... ({} more items)",
                        arr.len() - result.len() + 1
                    )));
                    serde_json::Value::Array(result)
                } else {
                    serde_json::Value::Array(truncated)
                }
            }
            serde_json::Value::Object(obj) => {
                // Truncate objects by keeping most important keys
                let mut result = serde_json::Map::new();
                let key_limit = (max_chars / 100).max(5); // Estimate ~100 chars per key-value pair

                for (i, (key, val)) in obj.iter().enumerate() {
                    if i >= key_limit {
                        result.insert(
                            "_truncated".to_string(),
                            serde_json::Value::String(format!("{} more fields", obj.len() - i)),
                        );
                        break;
                    }
                    // Recursively truncate values
                    let truncated_val = self.truncate_json_value(val, max_chars / key_limit);
                    result.insert(key.clone(), truncated_val);
                }
                serde_json::Value::Object(result)
            }
            serde_json::Value::String(s) => {
                // Truncate long strings
                if s.len() > max_chars {
                    let truncated = &s[..max_chars.saturating_sub(20).min(s.len())];
                    serde_json::Value::String(format!("{}... [truncated]", truncated))
                } else {
                    value.clone()
                }
            }
            _ => value.clone(), // Numbers, booleans, null - keep as-is
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(ContextBudgeter::estimate_tokens("hello world"), 3);
        assert_eq!(ContextBudgeter::estimate_tokens(""), 1);
        assert_eq!(
            ContextBudgeter::estimate_tokens("a".repeat(100).as_str()),
            25
        );
    }

    #[test]
    fn test_available_input_tokens() {
        let budgeter = ContextBudgeter::new(10000, 2000);
        assert_eq!(budgeter.available_input_tokens(), 8000);
    }

    #[test]
    fn test_build_context_within_budget() {
        let budgeter = ContextBudgeter::new(10000, 2000);

        let items = vec![
            ContextItem {
                content: "System prompt".to_string(),
                priority: ContextPriority::System,
                label: "system".to_string(),
            },
            ContextItem {
                content: "Task description".to_string(),
                priority: ContextPriority::Task,
                label: "task".to_string(),
            },
        ];

        let result = budgeter.build_context(items).unwrap();
        assert!(result.dropped_items.is_empty());
        assert!(result.prompt.contains("System prompt"));
        assert!(result.prompt.contains("Task description"));
    }

    #[test]
    fn test_build_context_exceeds_budget() {
        let budgeter = ContextBudgeter::new(10, 5); // Very small budget - only 5 tokens available

        let items = vec![
            ContextItem {
                content: "System prompt".to_string(),
                priority: ContextPriority::System,
                label: "system".to_string(),
            },
            ContextItem {
                content: "Task description".to_string(),
                priority: ContextPriority::Task,
                label: "task".to_string(),
            },
            ContextItem {
                content: "Long tail memory that should be dropped first".to_string(),
                priority: ContextPriority::LongTailMemory,
                label: "memory".to_string(),
            },
        ];

        let result = budgeter.build_context(items).unwrap();
        assert!(!result.dropped_items.is_empty());
        // Low priority items should be dropped first
        assert!(
            result
                .dropped_items
                .iter()
                .any(|item| item.contains("memory"))
        );
        // System and task should be preserved (they're higher priority)
        assert!(
            result.prompt.contains("System prompt") || result.prompt.contains("Task description")
        );
    }

    #[test]
    fn test_priority_ordering() {
        assert!(ContextPriority::System < ContextPriority::Task);
        assert!(ContextPriority::Task < ContextPriority::Plan);
        assert!(ContextPriority::Plan < ContextPriority::CriticalMemory);
        assert!(ContextPriority::CriticalMemory < ContextPriority::RecentArtifacts);
        assert!(ContextPriority::RecentArtifacts < ContextPriority::LongTailMemory);
    }

    #[test]
    fn test_trim_content_json() {
        let budgeter = ContextBudgeter::default();
        let json_content = r#"{"key": "value", "nested": {"data": "test"}}"#;

        let result = budgeter.trim_content(json_content, 5).unwrap();
        // Should either be valid JSON or an explicit truncation marker
        if !result.contains("Content too large") {
            assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
        }
    }

    #[test]
    fn test_trim_content_code_block() {
        let budgeter = ContextBudgeter::default();
        let code_content = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";

        let result = budgeter.trim_content(code_content, 20).unwrap();
        // Should close the code block if truncated
        if result.contains("```") {
            let count = result.matches("```").count();
            assert!(count % 2 == 0, "Code blocks should be balanced");
        }
    }

    #[test]
    fn test_budget_report() {
        let budgeter = ContextBudgeter::new(10000, 2000);

        let items = vec![ContextItem {
            content: "Test content".to_string(),
            priority: ContextPriority::Task,
            label: "task".to_string(),
        }];

        let report = budgeter.budget_report(&items);
        assert!(report.contains_key("total_tokens"));
        assert!(report.contains_key("available_tokens"));
        assert!(report.contains_key("usage_percentage"));
        assert_eq!(report["available_tokens"], 8000);
    }
}
