//! V1.5 Context Budgeter Tests
//!
//! Tests for context budgeting, token estimation, and context trimming.

use prometheos_lite::context::{ContextBudgeter, ContextItem, ContextPriority};

#[test]
fn test_context_budgeter_default() {
    let budgeter = ContextBudgeter::default();
    assert_eq!(budgeter.max_tokens, 128_000);
    assert_eq!(budgeter.reserved_output_tokens, 4_096);
}

#[test]
fn test_available_input_tokens() {
    let budgeter = ContextBudgeter::new(10000, 2000);
    assert_eq!(budgeter.available_input_tokens(), 8000);
}

#[test]
fn test_estimate_tokens() {
    assert_eq!(ContextBudgeter::estimate_tokens("hello world"), 2);
    assert_eq!(ContextBudgeter::estimate_tokens(""), 1); // Empty string counts as 1 token
    assert_eq!(
        ContextBudgeter::estimate_tokens("a".repeat(100).as_str()),
        25
    );
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
    let budgeter = ContextBudgeter::new(10, 2); // Very small budget to force dropping

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
            content: "Long tail memory that should be dropped".to_string(),
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
    // Should either be valid JSON or a placeholder
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

#[test]
fn test_context_overflow_trimming_deterministic() {
    let budgeter = ContextBudgeter::new(50, 10);

    let items = vec![
        ContextItem {
            content: "High priority content".to_string(),
            priority: ContextPriority::System,
            label: "system".to_string(),
        },
        ContextItem {
            content: "Medium priority content".to_string(),
            priority: ContextPriority::Task,
            label: "task".to_string(),
        },
        ContextItem {
            content: "Low priority content".to_string(),
            priority: ContextPriority::LongTailMemory,
            label: "memory".to_string(),
        },
    ];

    let result1 = budgeter.build_context(items.clone()).unwrap();
    let result2 = budgeter.build_context(items).unwrap();

    // Results should be deterministic
    assert_eq!(result1.dropped_items, result2.dropped_items);
    assert_eq!(result1.token_count, result2.token_count);
}
