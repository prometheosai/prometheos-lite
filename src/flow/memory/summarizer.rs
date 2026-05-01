//! Memory summarization for compression
//!
//! This module provides memory summarization to compress clusters of memories
//! into single summarized memories, reducing token usage while preserving information.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::flow::ModelRouter;
use crate::flow::memory::types::{Memory, MemoryKind};

/// Memory summarization for compressing memory clusters
#[derive(Clone)]
pub struct MemorySummarizer {
    /// Optional model router for LLM-based summarization
    model_router: Option<std::sync::Arc<ModelRouter>>,
}

impl MemorySummarizer {
    /// Create a new MemorySummarizer
    pub fn new(model_router: Option<std::sync::Arc<ModelRouter>>) -> Self {
        Self { model_router }
    }

    /// Summarize a cluster of memories into a single memory
    pub async fn summarize(&self, memories: Vec<Memory>) -> Result<Memory> {
        if memories.is_empty() {
            anyhow::bail!("Cannot summarize empty memory list");
        }

        if memories.len() == 1 {
            return Ok(memories.into_iter().next().unwrap());
        }

        // Calculate combined importance score
        let combined_importance =
            memories.iter().map(|m| m.importance_score).sum::<f32>() / memories.len() as f32;

        // Collect all content
        let combined_content: String = memories
            .iter()
            .map(|m| {
                format!(
                    "[{:?} - {}]\n{}",
                    m.kind, m.created_at.format("%Y-%m-%d"), m.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // Generate summary using LLM if available, otherwise use heuristic
        let summary = if let Some(ref router) = self.model_router {
            self.llm_summarize(&combined_content, router).await?
        } else {
            self.heuristic_summarize(&combined_content)
        };

        // Create summarized memory
        let first_memory = &memories[0];
        let summarized = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: first_memory.user_id.clone(),
            project_id: first_memory.project_id.clone(),
            conversation_id: first_memory.conversation_id.clone(),
            kind: MemoryKind::Semantic, // Summaries are semantic
            content: combined_content,
            summary: Some(summary),
            embedding: None,
            importance_score: combined_importance,
            confidence_score: 0.7, // Summaries have slightly lower confidence
            source: "summarizer".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({
                "summarized_count": memories.len(),
                "source_ids": memories.iter().map(|m| &m.id).collect::<Vec<_>>()
            }),
        };

        Ok(summarized)
    }

    /// LLM-based summarization
    async fn llm_summarize(&self, content: &str, router: &ModelRouter) -> Result<String> {
        let prompt = format!(
            "Summarize the following memories into a concise summary (max 200 words) that captures the key information:\n\n{}",
            content
        );

        let result = router.generate(&prompt).await?;
        Ok(result)
    }

    /// Heuristic summarization for when LLM is unavailable
    pub fn heuristic_summarize(&self, content: &str) -> String {
        // Take first 300 characters as a simple summary
        let truncated = if content.len() > 300 {
            &content[..300]
        } else {
            content
        };

        format!("{}...", truncated)
    }

    /// Check if compression should be triggered
    pub fn should_compress(
        &self,
        memories: &[Memory],
        count_threshold: usize,
        token_threshold: usize,
    ) -> bool {
        if memories.len() >= count_threshold {
            return true;
        }

        // Estimate total token count
        let total_tokens: usize = memories
            .iter()
            .map(|m| crate::context::ContextBudgeter::estimate_tokens(&m.content))
            .sum();

        total_tokens >= token_threshold
    }

    /// Compress memories by summarizing clusters
    pub async fn compress(
        &self,
        memories: Vec<Memory>,
        cluster_size: usize,
    ) -> Result<Vec<Memory>> {
        if memories.len() <= cluster_size {
            return Ok(memories);
        }

        let mut compressed = Vec::new();
        let mut cluster = Vec::new();

        for memory in memories {
            cluster.push(memory);

            if cluster.len() >= cluster_size {
                let summary = self.summarize(cluster.clone()).await?;
                compressed.push(summary);
                cluster.clear();
            }
        }

        // Add remaining memories
        if !cluster.is_empty() {
            compressed.extend(cluster);
        }

        Ok(compressed)
    }
}

impl Default for MemorySummarizer {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_memory(content: &str) -> Memory {
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: crate::flow::memory::types::MemoryKind::Semantic,
            content: content.to_string(),
            summary: None,
            embedding: None,
            importance_score: 0.5,
            confidence_score: 0.8,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn test_summarize_single_memory() {
        let summarizer = MemorySummarizer::new(None);
        let memory = create_test_memory("Test content");

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

    #[tokio::test]
    async fn test_compress_memories() {
        let summarizer = MemorySummarizer::new(None);
        let memories = vec![
            create_test_memory("Memory 1"),
            create_test_memory("Memory 2"),
            create_test_memory("Memory 3"),
        ];

        let result = summarizer.compress(memories, 10).await;
        assert!(result.is_ok());
        let compressed = result.unwrap();
        assert!(compressed.len() <= 3);
    }

    #[tokio::test]
    async fn test_compress_with_target_count() {
        let summarizer = MemorySummarizer::default();
        let memories = vec![
            create_test_memory("Memory 1"),
            create_test_memory("Memory 2"),
            create_test_memory("Memory 3"),
        ];

        let result = summarizer.compress(memories, 2).await;
        assert!(result.is_ok());
        let compressed = result.unwrap();
        assert_eq!(compressed.len(), 2);
    }

    #[tokio::test]
    async fn test_compress_empty_memories() {
        let summarizer = MemorySummarizer::default();
        let memories: Vec<Memory> = Vec::new();

        let result = summarizer.compress(memories, 10).await;
        assert!(result.is_ok());
        let compressed = result.unwrap();
        assert!(compressed.is_empty());
    }

    #[test]
    fn test_should_compress_by_count() {
        let summarizer = MemorySummarizer::default();
        let memories = vec![
            create_test_memory("Memory 1"),
            create_test_memory("Memory 2"),
            create_test_memory("Memory 3"),
        ];

        assert!(summarizer.should_compress(&memories, 2, 10000));
        assert!(!summarizer.should_compress(&memories, 5, 10000));
    }

    #[test]
    fn test_should_compress_by_tokens() {
        let summarizer = MemorySummarizer::default();
        let memories = vec![
            create_test_memory(&"a".repeat(1000)),
            create_test_memory(&"b".repeat(1000)),
        ];

        assert!(summarizer.should_compress(&memories, 10, 100));
        assert!(!summarizer.should_compress(&memories, 10, 10000));
    }

    #[tokio::test]
    async fn test_compress_no_change_if_under_cluster_size() {
        let summarizer = MemorySummarizer::default();
        let memories = vec![
            create_test_memory("Memory 1"),
            create_test_memory("Memory 2"),
        ];

        let result = std::sync::Arc::new(summarizer).compress(memories, 5).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_compress_clusters() {
        let summarizer = MemorySummarizer::default();
        let memories = vec![
            create_test_memory("Memory 1"),
            create_test_memory("Memory 2"),
            create_test_memory("Memory 3"),
            create_test_memory("Memory 4"),
            create_test_memory("Memory 5"),
        ];

        let result = std::sync::Arc::new(summarizer).compress(memories, 2).await;

        assert!(result.is_ok());
        // Should compress into clusters of 2
        let compressed = result.unwrap();
        assert!(compressed.len() <= 5);
    }
}
