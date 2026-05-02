//! Memory-related flow nodes

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;

use super::service::MemoryService;
use super::types::MemoryKind;
use crate::flow::{Action, Input, Node, NodeConfig, Output, SharedState};

#[derive(Debug, Clone)]
struct ExtractedMemory {
    content: String,
    kind: MemoryKind,
    summary: Option<String>,
    importance_score: f32,
    confidence_score: f32,
    metadata: serde_json::Value,
}

/// MemoryExtractorNode - extracts semantic memories from conversation exchanges
pub struct MemoryExtractorNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    user_message_key: String,
    assistant_response_key: String,
    conversation_id_key: String,
}

impl MemoryExtractorNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        user_message_key: String,
        assistant_response_key: String,
        conversation_id_key: String,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            user_message_key,
            assistant_response_key,
            conversation_id_key,
        }
    }
}

#[async_trait]
impl Node for MemoryExtractorNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "memory_extractor"
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let user_message = state
            .get_input(&self.user_message_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let assistant_response = state
            .get_input(&self.assistant_response_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let conversation_id = state
            .get_input(&self.conversation_id_key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(serde_json::json!({
            "user_message": user_message,
            "assistant_response": assistant_response,
            "conversation_id": conversation_id,
        }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let user_message = input["user_message"]
            .as_str()
            .context("Missing user_message")?;
        let assistant_response = input["assistant_response"]
            .as_str()
            .context("Missing assistant_response")?;
        let conversation_id = input["conversation_id"].as_str().map(|s| s.to_string());

        // Simple heuristic extraction (in production, use LLM for better extraction)
        let extracted_memories = self.extract_semantic_memories(user_message, assistant_response);

        let mut failed_queues = 0usize;
        for memory in &extracted_memories {
            if let Err(e) = self.memory_service.queue_semantic(
                memory.content.clone(),
                memory.kind.clone(),
                None, // user_id
                None, // project_id
                conversation_id.clone(),
                memory.summary.clone(),
                memory.importance_score,
                memory.confidence_score,
                memory.metadata.clone(),
            ) {
                tracing::error!("Failed to queue semantic memory: {}", e);
                failed_queues += 1;
            }
        }

        if failed_queues > 0 && failed_queues == extracted_memories.len() {
            anyhow::bail!("All {} memory queue operations failed", failed_queues);
        }

        Ok(serde_json::json!({
            "extracted_count": extracted_memories.len() - failed_queues,
            "failed_count": failed_queues,
        }))
    }

    fn post(&self, _state: &mut SharedState, output: Output) -> Action {
        if let Some(count) = output["extracted_count"].as_u64() {
            // Could emit event about extraction
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

impl MemoryExtractorNode {
    /// Extract semantic memories from conversation (heuristic-based)
    fn extract_semantic_memories(
        &self,
        user_message: &str,
        assistant_response: &str,
    ) -> Vec<ExtractedMemory> {
        let mut memories = Vec::new();
        let combined = format!("{}\n{}", user_message, assistant_response).to_lowercase();

        // Extract preferences
        if combined.contains("prefer") || combined.contains("like") || combined.contains("want") {
            memories.push(ExtractedMemory {
                content: format!("User preference detected in conversation"),
                kind: MemoryKind::Preference,
                summary: Some("User expressed a preference".to_string()),
                importance_score: 0.7,
                confidence_score: 0.6,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "preference_detection",
                }),
            });
        }

        // Extract decisions
        if combined.contains("decided") || combined.contains("choose") || combined.contains("will")
        {
            memories.push(ExtractedMemory {
                content: format!("Decision made in conversation"),
                kind: MemoryKind::Decision,
                summary: Some("A decision was made".to_string()),
                importance_score: 0.8,
                confidence_score: 0.7,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "decision_detection",
                }),
            });
        }

        // Extract constraints
        if combined.contains("must") || combined.contains("should") || combined.contains("require")
        {
            memories.push(ExtractedMemory {
                content: format!("Constraint identified in conversation"),
                kind: MemoryKind::Constraint,
                summary: Some("A constraint was identified".to_string()),
                importance_score: 0.75,
                confidence_score: 0.65,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "constraint_detection",
                }),
            });
        }

        // Extract project facts (heuristic: technical terms, file names, etc.)
        if combined.contains("file") || combined.contains("function") || combined.contains("class")
        {
            memories.push(ExtractedMemory {
                content: format!("Project fact mentioned in conversation"),
                kind: MemoryKind::ProjectFact,
                summary: Some("Project-related information".to_string()),
                importance_score: 0.6,
                confidence_score: 0.5,
                metadata: serde_json::json!({
                    "source": "heuristic",
                    "context": "project_fact_detection",
                }),
            });
        }

        memories
    }
}

/// ContextLoaderNode - loads relevant memories into flow state
pub struct ContextLoaderNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    query_key: String,
    output_key: String,
    limit: usize,
}

impl ContextLoaderNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        query_key: String,
        output_key: String,
        limit: usize,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            query_key,
            output_key,
            limit,
        }
    }
}

#[async_trait]
impl Node for ContextLoaderNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "context_loader"
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let query = state
            .get_input(&self.query_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Ok(serde_json::json!({ "query": query }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let query = input["query"].as_str().context("Missing query in input")?;

        let memories = self
            .memory_service
            .semantic_search(query, self.limit)
            .await?;

        let memories_json: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "content": m.content,
                    "kind": format!("{:?}", m.kind),
                    "metadata": m.metadata,
                })
            })
            .collect();

        Ok(serde_json::json!({ "memories": memories_json }))
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        if let Some(memories) = output["memories"].as_array() {
            state.set_output(self.output_key.clone(), serde_json::json!(memories));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// MemoryWriteNode - writes memories to the memory service
pub struct MemoryWriteNode {
    id: String,
    config: NodeConfig,
    memory_service: Arc<MemoryService>,
    content_key: String,
    kind: MemoryKind,
}

impl MemoryWriteNode {
    pub fn new(
        id: String,
        memory_service: Arc<MemoryService>,
        content_key: String,
        kind: MemoryKind,
    ) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            memory_service,
            content_key,
            kind,
        }
    }
}

#[async_trait]
impl Node for MemoryWriteNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "memory_write"
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let content = state
            .get_input(&self.content_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let metadata = state
            .get_meta("memory_metadata")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        Ok(serde_json::json!({ "content": content, "metadata": metadata }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        let content = input["content"].as_str().context("Missing content")?;

        let metadata = input["metadata"].clone();

        let memory_id = self
            .memory_service
            .create_memory(
                content.to_string(),
                super::types::MemoryType::Semantic,
                metadata,
            )
            .await?;

        Ok(serde_json::json!({ "memory_id": memory_id }))
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        if let Some(memory_id) = output["memory_id"].as_str() {
            state.set_output("memory_id".to_string(), serde_json::json!(memory_id));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}
