//! Context Builder - Unified context construction across all nodes
//!
//! This module provides a unified interface for building LLM prompts
//! with automatic token budgeting, memory integration, and context trimming.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::budgeter::{ContextBudgeter, ContextItem, ContextPriority};
use crate::flow::memory::service::MemoryService;
use crate::flow::memory::types::Memory;

/// Context inputs for building prompts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextInputs {
    /// The task or user request
    pub task: String,
    /// Optional plan or strategy
    pub plan: Option<String>,
    /// Memory items to include
    pub memory: Vec<Memory>,
    /// Artifacts or outputs from previous steps
    pub artifacts: Vec<Artifact>,
    /// System prompt (if provided)
    pub system_prompt: Option<String>,
}

/// Artifact reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Built context result
#[derive(Debug, Clone)]
pub struct BuiltContext {
    /// The final prompt text
    pub prompt: String,
    /// Items that were dropped due to budget constraints
    pub dropped_items: Vec<String>,
    /// Total token count
    pub token_count: usize,
    /// Metadata about the context construction
    pub metadata: ContextMetadata,
}

/// Metadata about context construction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetadata {
    /// Number of memory items included
    pub memory_count: usize,
    /// Number of artifacts included
    pub artifact_count: usize,
    /// Whether plan was included
    pub plan_included: bool,
    /// Whether system prompt was included
    pub system_prompt_included: bool,
}

/// Context Builder - unified context construction
#[derive(Clone)]
pub struct ContextBuilder {
    budgeter: ContextBudgeter,
    memory_service: Option<Arc<MemoryService>>,
}

impl ContextBuilder {
    /// Create a new ContextBuilder
    pub fn new(budgeter: ContextBudgeter) -> Self {
        Self {
            budgeter,
            memory_service: None,
        }
    }

    /// Create a new ContextBuilder with memory service
    pub fn with_memory_service(
        budgeter: ContextBudgeter,
        memory_service: Arc<MemoryService>,
    ) -> Self {
        Self {
            budgeter,
            memory_service: Some(memory_service),
        }
    }

    /// Build context from inputs
    pub fn build(&self, inputs: ContextInputs) -> Result<BuiltContext> {
        let mut items = Vec::new();

        // Add system prompt (highest priority)
        let system_prompt_included = inputs.system_prompt.is_some();
        if let Some(system_prompt) = inputs.system_prompt {
            items.push(ContextItem {
                content: system_prompt,
                priority: ContextPriority::System,
                label: "system_prompt".to_string(),
            });
        }

        // Add task (high priority)
        items.push(ContextItem {
            content: inputs.task.clone(),
            priority: ContextPriority::Task,
            label: "task".to_string(),
        });

        // Add plan (high priority)
        let plan_included = inputs.plan.is_some();
        if let Some(plan) = inputs.plan {
            items.push(ContextItem {
                content: plan,
                priority: ContextPriority::Plan,
                label: "plan".to_string(),
            });
        }

        // Add critical memory (high priority)
        let critical_memory: Vec<_> = inputs
            .memory
            .iter()
            .filter(|m| m.importance_score > 0.7)
            .collect();

        for memory in critical_memory {
            items.push(ContextItem {
                content: self.format_memory(memory),
                priority: ContextPriority::CriticalMemory,
                label: format!("memory_{}", memory.id),
            });
        }

        // Add recent artifacts (medium priority)
        let artifact_count = inputs.artifacts.len();
        for artifact in inputs.artifacts.iter().take(5) {
            items.push(ContextItem {
                content: self.format_artifact(artifact),
                priority: ContextPriority::RecentArtifacts,
                label: format!("artifact_{}", artifact.id),
            });
        }

        // Add long-tail memory (low priority)
        let long_tail_memory: Vec<_> = inputs
            .memory
            .iter()
            .filter(|m| m.importance_score <= 0.7)
            .collect();

        for memory in long_tail_memory {
            items.push(ContextItem {
                content: self.format_memory(memory),
                priority: ContextPriority::LongTailMemory,
                label: format!("memory_{}", memory.id),
            });
        }

        // Build context with budgeter
        let trimmed = self.budgeter.build_context(items)?;

        let memory_count = inputs.memory.len();

        Ok(BuiltContext {
            prompt: trimmed.prompt,
            dropped_items: trimmed.dropped_items,
            token_count: trimmed.token_count,
            metadata: ContextMetadata {
                memory_count,
                artifact_count,
                plan_included,
                system_prompt_included,
            },
        })
    }

    /// Format memory for inclusion in context
    fn format_memory(&self, memory: &Memory) -> String {
        let summary = memory.summary.as_deref().unwrap_or(&memory.content);
        format!(
            "[Memory - {:?}]\nImportance: {:.2}\nContent: {}",
            memory.kind, memory.importance_score, summary
        )
    }

    /// Format artifact for inclusion in context
    fn format_artifact(&self, artifact: &Artifact) -> String {
        format!(
            "[Artifact - {}]\nKind: {}\nContent: {}",
            artifact.id, artifact.kind, artifact.content
        )
    }

    /// Build context with automatic memory retrieval
    ///
    /// # Errors
    ///
    /// Returns an error if the memory service is configured but memory retrieval fails.
    /// This ensures explicit failure rather than silent degradation.
    pub async fn build_with_memory_retrieval(
        &self,
        task: String,
        project_id: Option<String>,
        limit: usize,
    ) -> Result<BuiltContext> {
        let memory = if let Some(ref service) = self.memory_service {
            // Use semantic_search with the provided limit
            // Note: project_id filtering would need to be implemented in MemoryService
            // For now, we pass the query and limit
            let memories = service
                .semantic_search(&task, limit)
                .await
                .context("Memory retrieval failed during context building")?;

            // Filter by project_id if provided
            if let Some(ref pid) = project_id {
                memories
                    .into_iter()
                    .filter(|m| m.project_id.as_deref() == Some(pid))
                    .collect()
            } else {
                memories
            }
        } else {
            // No memory service configured - proceed without memory
            Vec::new()
        };

        let inputs = ContextInputs {
            task,
            plan: None,
            memory,
            artifacts: Vec::new(),
            system_prompt: None,
        };

        self.build(inputs)
    }

    /// Get the budgeter
    pub fn budgeter(&self) -> &ContextBudgeter {
        &self.budgeter
    }

    /// Get the memory service
    pub fn memory_service(&self) -> Option<&Arc<MemoryService>> {
        self.memory_service.as_ref()
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new(ContextBudgeter::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_memory(importance: f32) -> Memory {
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            project_id: None,
            conversation_id: None,
            kind: crate::flow::memory::types::MemoryKind::Semantic,
            content: "Test memory content".to_string(),
            summary: Some("Test summary".to_string()),
            embedding: None,
            importance_score: importance,
            confidence_score: 0.8,
            source: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            metadata: serde_json::json!({}),
        }
    }

    fn create_test_artifact() -> Artifact {
        Artifact {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "code".to_string(),
            content: "Test artifact content".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_context_builder_basic() {
        let builder = ContextBuilder::default();

        let inputs = ContextInputs {
            task: "Test task".to_string(),
            plan: Some("Test plan".to_string()),
            memory: vec![create_test_memory(0.8)],
            artifacts: vec![create_test_artifact()],
            system_prompt: Some("You are a helpful assistant".to_string()),
        };

        let result = builder.build(inputs).unwrap();
        assert!(!result.prompt.is_empty());
        assert!(result.prompt.contains("Test task"));
        assert!(result.metadata.system_prompt_included);
        assert!(result.metadata.plan_included);
        assert_eq!(result.metadata.memory_count, 1);
        assert_eq!(result.metadata.artifact_count, 1);
    }

    #[tokio::test]
    async fn test_context_builder_no_memory_service() {
        let budgeter = ContextBudgeter::default();
        let builder = ContextBuilder::new(budgeter);

        let context_inputs = ContextInputs {
            task: "Test task".to_string(),
            plan: None,
            memory: Vec::new(),
            artifacts: Vec::new(),
            system_prompt: None,
        };

        let result = builder.build(context_inputs).unwrap();
        assert!(result.prompt.contains("Test task"));
    }

    #[test]
    fn test_context_builder_memory_priority() {
        let builder = ContextBuilder::new(ContextBudgeter::new(100, 20));

        let inputs = ContextInputs {
            task: "Test task".to_string(),
            plan: None,
            memory: vec![
                create_test_memory(0.9), // Critical
                create_test_memory(0.5), // Long-tail
            ],
            artifacts: vec![],
            system_prompt: None,
        };

        let result = builder.build(inputs).unwrap();
        // With small budget, long-tail memory should be dropped first
        if !result.dropped_items.is_empty() {
            assert!(result.prompt.contains("Test task")); // Task should always be kept
        }
    }

    #[test]
    fn test_format_memory() {
        let builder = ContextBuilder::default();
        let memory = create_test_memory(0.8);

        let formatted = builder.format_memory(&memory);
        assert!(formatted.contains("Memory"));
        assert!(formatted.contains("Importance: 0.80"));
        assert!(formatted.contains("Test summary"));
    }

    #[test]
    fn test_format_artifact() {
        let builder = ContextBuilder::default();
        let artifact = create_test_artifact();

        let formatted = builder.format_artifact(&artifact);
        assert!(formatted.contains("Artifact"));
        assert!(formatted.contains("code"));
        assert!(formatted.contains("Test artifact content"));
    }
}
