//! Memory scoring and ranking for pruning
//!
//! This module provides memory scoring based on relevance, recency, and usage,
//! along with ranking and pruning functions to keep memory size bounded.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::flow::memory::types::Memory;

/// Memory score for ranking and pruning decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryScore {
    /// Relevance score (0.0 to 1.0)
    pub relevance: f32,
    /// Recency score (0.0 to 1.0, higher for more recent)
    pub recency: f32,
    /// Usage score (0.0 to 1.0, higher for frequently accessed)
    pub usage: f32,
    /// Overall composite score
    pub overall: f32,
}

impl MemoryScore {
    /// Calculate score for a memory
    pub fn calculate(memory: &Memory, current_time: chrono::DateTime<Utc>) -> Self {
        // Relevance: based on importance_score
        let relevance = memory.importance_score;

        // Recency: based on time since last access or creation
        let recency = if let Some(last_accessed) = memory.last_accessed_at {
            let days_since_access = (current_time - last_accessed).num_days();
            // Decay over 30 days
            (1.0 - (days_since_access as f32 / 30.0).max(0.0)).max(0.0)
        } else {
            let days_since_creation = (current_time - memory.created_at).num_days();
            (1.0 - (days_since_creation as f32 / 30.0).max(0.0)).max(0.0)
        };

        // Usage: based on access count (logarithmic scaling)
        let usage = (memory.access_count as f32).ln_1p() / 5.0; // Normalize roughly to 0-1

        // Overall: weighted average
        let overall = relevance * 0.5 + recency * 0.3 + usage * 0.2;

        Self {
            relevance,
            recency,
            usage,
            overall: overall.clamp(0.0, 1.0),
        }
    }

    /// Check if memory should be pruned based on score threshold
    pub fn should_prune(&self, threshold: f32) -> bool {
        self.overall < threshold
    }
}

/// Rank memories by score
pub fn rank_memories(memories: Vec<Memory>) -> Vec<(Memory, MemoryScore)> {
    let current_time = Utc::now();

    let mut scored: Vec<_> = memories
        .into_iter()
        .map(|memory| {
            let score = MemoryScore::calculate(&memory, current_time);
            (memory, score)
        })
        .collect();

    // Sort by overall score (descending)
    scored.sort_by(|a, b| {
        b.1.overall
            .partial_cmp(&a.1.overall)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
}

/// Prune memories to a maximum count, keeping highest-scoring memories
pub fn prune(memories: Vec<Memory>, max_count: usize) -> Vec<Memory> {
    if memories.len() <= max_count {
        return memories;
    }

    let ranked = rank_memories(memories);
    ranked
        .into_iter()
        .take(max_count)
        .map(|(memory, _)| memory)
        .collect()
}

/// Prune memories based on score threshold
pub fn prune_by_threshold(memories: Vec<Memory>, threshold: f32) -> Vec<Memory> {
    let current_time = Utc::now();

    memories
        .into_iter()
        .filter(|memory| {
            let score = MemoryScore::calculate(memory, current_time);
            !score.should_prune(threshold)
        })
        .collect()
}

/// Prune memories combining count and threshold
pub fn prune_combined(memories: Vec<Memory>, max_count: usize, threshold: f32) -> Vec<Memory> {
    let current_time = Utc::now();

    let mut scored: Vec<_> = memories
        .into_iter()
        .map(|memory| {
            let score = MemoryScore::calculate(&memory, current_time);
            (memory, score)
        })
        .collect();

    // Filter by threshold first
    scored.retain(|(_, score)| !score.should_prune(threshold));

    // Then limit by count
    if scored.len() > max_count {
        scored.sort_by(|a, b| {
            b.1.overall
                .partial_cmp(&a.1.overall)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(max_count);
    }

    scored.into_iter().map(|(memory, _)| memory).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use chrono::Utc;

    fn create_test_memory(importance: f32, days_old: i64, access_count: u32) -> Memory {
        let created_at = Utc::now() - Duration::days(days_old);
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: crate::flow::memory::types::MemoryKind::Semantic,
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
    fn test_should_prune() {
        let memory = create_test_memory(0.8, 1, 10);
        let score = MemoryScore::calculate(&memory, Utc::now());

        assert!(!score.should_prune(0.5)); // High score should not be pruned
        assert!(score.should_prune(0.95)); // Should be pruned with very high threshold
    }
}
