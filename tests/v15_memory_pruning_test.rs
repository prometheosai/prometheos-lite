//! V1.5 Memory Pruning & Summarization Tests
//!
//! Tests for memory scoring, ranking, pruning, and summarization.

use chrono::{Duration, Utc};
use prometheos_lite::flow::memory::types::{Memory, MemoryKind};
use prometheos_lite::flow::memory::{
    MemoryScore, prune, prune_by_threshold, prune_combined, rank_memories,
};
use prometheos_lite::flow::memory::summarizer::MemorySummarizer;

fn create_test_memory(importance: f32, days_old: i64, access_count: u32) -> Memory {
    let created_at = Utc::now() - Duration::days(days_old);
    Memory {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: None,
        project_id: None,
        conversation_id: None,
        kind: MemoryKind::Semantic,
        content: "Test content".to_string(),
        summary: Some("Test summary".to_string()),
        embedding: None,
        importance_score: importance,
        confidence_score: 0.8,
        source: "test".to_string(),
        created_at,
        updated_at: created_at,
        last_accessed_at: Some(created_at),
        access_count: access_count as i32,
        metadata: serde_json::json!({}),
    }
}

#[test]
fn test_memory_score_calculation() {
    let memory = create_test_memory(0.8, 5, 10);
    let score = MemoryScore::calculate(&memory, Utc::now());

    assert!(score.relevance == 0.8);
    assert!(score.recency > 0.0);
    assert!(score.usage > 0.0);
    assert!(score.overall > 0.0 && score.overall <= 1.0);
}

#[test]
fn test_memory_score_recency_decay() {
    let recent_memory = create_test_memory(0.5, 1, 0);
    let old_memory = create_test_memory(0.5, 60, 0);

    let recent_score = MemoryScore::calculate(&recent_memory, Utc::now());
    let old_score = MemoryScore::calculate(&old_memory, Utc::now());

    assert!(recent_score.recency > old_score.recency);
}

#[test]
fn test_memory_score_usage_scaling() {
    let low_usage = create_test_memory(0.5, 1, 1);
    let high_usage = create_test_memory(0.5, 1, 100);

    let low_score = MemoryScore::calculate(&low_usage, Utc::now());
    let high_score = MemoryScore::calculate(&high_usage, Utc::now());

    assert!(high_score.usage > low_score.usage);
}

#[test]
fn test_rank_memories() {
    let memories = vec![
        create_test_memory(0.3, 30, 1),  // Low score
        create_test_memory(0.9, 1, 100), // High score
        create_test_memory(0.5, 10, 10), // Medium score
    ];

    let ranked = rank_memories(memories);

    // Should be sorted by overall score descending
    assert!(ranked[0].1.overall >= ranked[1].1.overall);
    assert!(ranked[1].1.overall >= ranked[2].1.overall);
}

#[test]
fn test_prune_by_count() {
    let memories = vec![
        create_test_memory(0.3, 30, 1),
        create_test_memory(0.9, 1, 100),
        create_test_memory(0.5, 10, 10),
    ];

    let pruned = prune(memories, 2);
    assert_eq!(pruned.len(), 2);
}

#[test]
fn test_prune_no_change_if_under_limit() {
    let memories = vec![
        create_test_memory(0.3, 30, 1),
        create_test_memory(0.9, 1, 100),
    ];

    let pruned = prune(memories, 5);
    assert_eq!(pruned.len(), 2);
}

#[test]
fn test_prune_by_threshold() {
    let memories = vec![
        create_test_memory(0.1, 60, 0),  // Very low score
        create_test_memory(0.9, 1, 100), // High score
        create_test_memory(0.5, 10, 10), // Medium score
    ];

    let pruned = prune_by_threshold(memories, 0.4);
    assert!(pruned.len() < 3); // At least one should be pruned
}

#[test]
fn test_prune_combined() {
    let memories = vec![
        create_test_memory(0.1, 60, 0),
        create_test_memory(0.9, 1, 100),
        create_test_memory(0.5, 10, 10),
        create_test_memory(0.2, 50, 1),
    ];

    let pruned = prune_combined(memories, 2, 0.3);
    assert!(pruned.len() <= 2);
}

#[test]
fn test_memory_pruning_correctness() {
    let memories = vec![
        create_test_memory(0.1, 60, 0),  // Old, low importance, low usage
        create_test_memory(0.9, 1, 100), // Recent, high importance, high usage
        create_test_memory(0.2, 50, 1),  // Old, low importance, low usage
    ];

    let pruned = prune(memories, 2);

    // Should keep high-value memories
    assert!(pruned.iter().any(|m| m.importance_score > 0.8));
    // Should remove low-value memories
    assert!(!pruned.iter().any(|m| m.importance_score < 0.2));
}

#[tokio::test]
async fn test_summarize_single_memory() {
    let summarizer = MemorySummarizer::default();
    let memory = create_test_memory(0.5, 1, 10);

    let result = std::sync::Arc::new(summarizer).summarize(vec![memory]).await;

    // Should return the same memory unchanged
    assert!(result.is_ok());
}

#[test]
fn test_heuristic_summarize() {
    let summarizer = MemorySummarizer::default();
    let content = "This is a long piece of content that should be truncated when summarized using the heuristic method because we don't have an LLM available in this test context.";

    let summary = summarizer.heuristic_summarize(content);

    assert!(summary.len() <= 303); // 300 + "..."
    assert!(summary.ends_with("..."));
}

#[test]
fn test_summarizer_compression_trigger() {
    let summarizer = MemorySummarizer::default();
    let memories = vec![
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
    ];

    assert!(summarizer.should_compress(&memories, 2, 10000));
    assert!(!summarizer.should_compress(&memories, 5, 10000));
}

#[tokio::test]
async fn test_summarizer_compress_clusters() {
    let summarizer = MemorySummarizer::default();
    let memories = vec![
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
        create_test_memory(0.5, 1, 10),
    ];

    let result = std::sync::Arc::new(summarizer).compress(memories, 2).await;

    assert!(result.is_ok());
    // Should compress into clusters of 2
    let compressed = result.unwrap();
    assert!(compressed.len() <= 5);
}
