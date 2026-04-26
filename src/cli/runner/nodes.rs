//! Node implementations for CLI runner

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;

use prometheos_lite::flow::{Node, NodeConfig, SharedState};

/// IdWrapper - wraps a node to override its id
pub struct IdWrapper {
    id: String,
    inner: Arc<dyn Node>,
}

impl IdWrapper {
    pub fn new(id: String, inner: Arc<dyn Node>) -> Self {
        Self { id, inner }
    }
}

#[async_trait::async_trait]
impl Node for IdWrapper {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        self.inner.prep(state)
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        self.inner.exec(input).await
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        self.inner.post(state, output)
    }

    fn config(&self) -> NodeConfig {
        self.inner.config()
    }
}

/// Planner Node - creates structured plans
pub struct PlannerNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl PlannerNode {
    pub fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
        Self { config, model_router }
    }
}

#[async_trait::async_trait]
impl Node for PlannerNode {
    fn id(&self) -> String {
        "planner".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(serde_json::json!({ "task": task }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let task = input["task"]
            .as_str()
            .context("Missing task in planner node input")?;

        if let Some(router) = &self.model_router {
            // Use ModelRouter for actual LLM call
            let prompt = format!(
                "You are a planning assistant. Create a structured plan for the following task:\n\nTask: {}\n\nProvide a step-by-step plan as a JSON array of strings.",
                task
            );
            let response = router.generate(&prompt).await?;
            Ok(serde_json::json!({ "plan": response }))
        } else {
            // Fallback to placeholder if no ModelRouter
            Ok(serde_json::json!({
                "plan": ["Step 1: Analyze requirements", "Step 2: Design solution", "Step 3: Implement"]
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(plan) = output["plan"].as_str() {
            state.set_working("plan".to_string(), serde_json::json!(plan));
        } else if let Some(plan) = output["plan"].as_array() {
            state.set_working("plan".to_string(), serde_json::json!(plan));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Coder Node - generates code
pub struct CoderNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl CoderNode {
    pub fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
        Self { config, model_router }
    }
}

#[async_trait::async_trait]
impl Node for CoderNode {
    fn id(&self) -> String {
        "coder".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let plan = state.get_working("plan").cloned().unwrap_or(serde_json::json!(null));
        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(serde_json::json!({ "task": task, "plan": plan }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let task = input["task"]
            .as_str()
            .context("Missing task in coder node input")?;
        let plan = &input["plan"];

        if let Some(router) = &self.model_router {
            // Use ModelRouter for actual LLM call
            let prompt = format!(
                "You are a coding assistant. Generate code for the following task based on the provided plan:\n\nTask: {}\n\nPlan: {}\n\nProvide the generated code only, without explanations.",
                task,
                serde_json::to_string(plan).unwrap_or_default()
            );
            let response = router.generate(&prompt).await?;
            Ok(serde_json::json!({ "generated_code": response }))
        } else {
            // Fallback to placeholder if no ModelRouter
            Ok(serde_json::json!({
                "generated_code": "// Generated code placeholder"
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(code) = output["generated_code"].as_str() {
            state.set_output("generated".to_string(), serde_json::json!(code));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Reviewer Node - reviews generated output
pub struct ReviewerNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl ReviewerNode {
    pub fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
        Self { config, model_router }
    }
}

#[async_trait::async_trait]
impl Node for ReviewerNode {
    fn id(&self) -> String {
        "reviewer".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let generated = state.get_output("generated").cloned().unwrap_or(serde_json::json!(null));
        Ok(serde_json::json!({ "generated": generated }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let generated = &input["generated"];

        if let Some(router) = &self.model_router {
            // Use ModelRouter for actual LLM call
            let prompt = format!(
                "You are a code reviewer. Review the following generated code:\n\nCode:\n{}\n\nProvide a brief review with feedback on quality, correctness, and potential improvements.",
                serde_json::to_string(generated).unwrap_or_default()
            );
            let response = router.generate(&prompt).await?;
            Ok(serde_json::json!({ "review": response }))
        } else {
            // Fallback to placeholder if no ModelRouter
            Ok(serde_json::json!({
                "review": "Code looks good - placeholder review"
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(review) = output["review"].as_str() {
            state.set_output("review".to_string(), serde_json::json!(review));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// LLM Node - executes an LLM call with configurable prompt template
pub struct LlmNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
    prompt_template: Option<String>,
}

impl LlmNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
        node_config: Option<serde_json::Value>,
    ) -> Self {
        let prompt_template = node_config
            .as_ref()
            .and_then(|cfg| cfg["prompt_template"].as_str())
            .map(|s| s.to_string());
        Self {
            config,
            model_router,
            prompt_template,
        }
    }
}

#[async_trait::async_trait]
impl Node for LlmNode {
    fn id(&self) -> String {
        "llm".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let prompt = state
            .get_input("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(serde_json::json!({ "prompt": prompt }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let prompt = input["prompt"]
            .as_str()
            .context("Missing prompt in LLM node input")?;

        if let Some(router) = &self.model_router {
            // Use ModelRouter for actual LLM call
            let final_prompt = if let Some(template) = &self.prompt_template {
                // Use configured prompt template
                template.replace("{{prompt}}", prompt)
            } else {
                // Use prompt directly
                prompt.to_string()
            };
            let response = router.generate(&final_prompt).await?;
            Ok(serde_json::json!({ "response": response }))
        } else {
            // Fallback to placeholder if no ModelRouter
            Ok(serde_json::json!({
                "response": "LLM response placeholder - integrate with ModelRouter"
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(response) = output["response"].as_str() {
            state.set_output("llm_response".to_string(), serde_json::json!(response));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Tool Node - executes a tool
pub struct ToolNode {
    config: NodeConfig,
    tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>,
}

impl ToolNode {
    pub fn new(config: NodeConfig, tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>) -> Self {
        Self { config, tool_runtime }
    }
}

#[async_trait::async_trait]
impl Node for ToolNode {
    fn id(&self) -> String {
        "tool".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let tool_name = state
            .get_input("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tool_args = state.get_input("tool_args").cloned().unwrap_or(serde_json::json!({}));
        Ok(serde_json::json!({
            "tool_name": tool_name,
            "tool_args": tool_args
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let tool_name = input["tool_name"]
            .as_str()
            .context("Missing tool_name in tool node input")?;
        let tool_args = &input["tool_args"];

        if let Some(runtime) = &self.tool_runtime {
            // Parse tool_args as a command and arguments
            let args: Vec<String> = if let Some(arr) = tool_args.as_array() {
                arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
            } else {
                vec![]
            };

            let result = runtime.execute_command(tool_name, args).await?;
            Ok(serde_json::json!({ "result": result }))
        } else {
            // Fallback to placeholder if no ToolRuntime
            Ok(serde_json::json!({
                "result": "Tool execution placeholder - integrate with ToolRuntime"
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(result) = output["result"].as_str() {
            state.set_output("tool_result".to_string(), serde_json::json!(result));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// FileWriter Node - writes files to disk
pub struct FileWriterNode {
    config: NodeConfig,
}

impl FileWriterNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for FileWriterNode {
    fn id(&self) -> String {
        "file_writer".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let content = state.get_output("generated").cloned().unwrap_or(serde_json::json!(null));
        let file_path = state
            .get_input("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("output.txt")
            .to_string();
        
        // Ensure prometheos-output directory exists
        std::fs::create_dir_all("prometheos-output")
            .context("Failed to create prometheos-output directory")?;
        
        // Prepend prometheos-output/ to the file path if not already absolute
        let full_path = if file_path.starts_with("/") || file_path.contains(":") {
            file_path
        } else {
            format!("prometheos-output/{}", file_path)
        };
        
        Ok(serde_json::json!({ "content": content, "file_path": full_path }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let content = input["content"]
            .as_str()
            .context("Missing content in file writer node input")?;
        let file_path = input["file_path"]
            .as_str()
            .context("Missing file_path in file writer node input")?;

        // Write the file to disk
        std::fs::write(file_path, content)
            .with_context(|| format!("Failed to write file: {}", file_path))?;

        Ok(serde_json::json!({
            "success": true,
            "file_path": file_path,
            "bytes_written": content.len()
        }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(file_path) = output["file_path"].as_str() {
            state.set_output("written_file".to_string(), serde_json::json!(file_path));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// ContextLoader Node - loads context from memory
pub struct ContextLoaderNode {
    config: NodeConfig,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl ContextLoaderNode {
    pub fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>) -> Self {
        Self { config, memory_service }
    }
}

#[async_trait::async_trait]
impl Node for ContextLoaderNode {
    fn id(&self) -> String {
        "context_loader".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let query = state
            .get_input("context_query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(serde_json::json!({ "query": query }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let query = input["query"]
            .as_str()
            .context("Missing query in context loader node input")?;

        if let Some(service) = &self.memory_service {
            // Use MemoryService for actual retrieval
            let memories = service.semantic_search(query, 5).await?;
            Ok(serde_json::json!({ "context": memories }))
        } else {
            // Fallback to placeholder if no MemoryService
            Ok(serde_json::json!({
                "context": {}
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(context) = output.get("context") {
            state.set_context("loaded_context".to_string(), context.clone());
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Memory Write Node - writes data to memory
pub struct MemoryWriteNode {
    config: NodeConfig,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl MemoryWriteNode {
    pub fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>) -> Self {
        Self { config, memory_service }
    }
}

#[async_trait::async_trait]
impl Node for MemoryWriteNode {
    fn id(&self) -> String {
        "memory_write".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let content = state
            .get_input("memory_content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(serde_json::json!({ "content": content }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let content = input["content"]
            .as_str()
            .context("Missing content in memory write node input")?;

        if let Some(service) = &self.memory_service {
            // Use MemoryService for actual write, but handle embedding server failures gracefully
            match service.create_memory(content.to_string(), prometheos_lite::flow::MemoryType::Semantic, serde_json::json!({})).await {
                Ok(memory_id) => Ok(serde_json::json!({ "memory_id": memory_id, "status": "success" })),
                Err(e) => {
                    // Log the error but don't fail the flow - embedding server might be unavailable
                    eprintln!("Memory write failed (embedding server unavailable?): {}", e);
                    Ok(serde_json::json!({ 
                        "memory_id": "skipped", 
                        "status": "skipped",
                        "reason": "embedding server unavailable"
                    }))
                }
            }
        } else {
            // Fallback to placeholder if no MemoryService
            Ok(serde_json::json!({
                "memory_id": "placeholder_id",
                "status": "placeholder"
            }))
        }
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(memory_id) = output["memory_id"].as_str() {
            state.set_output("memory_id".to_string(), serde_json::json!(memory_id));
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Conditional Node - evaluates a condition and returns different actions
pub struct ConditionalNode {
    config: NodeConfig,
}

impl ConditionalNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for ConditionalNode {
    fn id(&self) -> String {
        "conditional".to_string()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let condition = state
            .get_input("condition")
            .and_then(|v| v.as_str())
            .unwrap_or("true")
            .to_string();
        Ok(serde_json::json!({ "condition": condition }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let condition = input["condition"]
            .as_str()
            .context("Missing condition in conditional node input")?;

        // Simple condition evaluation
        let result = match condition {
            "true" => true,
            "false" => false,
            _ => {
                // Try to parse as boolean
                condition.parse::<bool>().unwrap_or(true)
            }
        };

        Ok(serde_json::json!({ "result": result }))
    }

    fn post(&self, _state: &mut SharedState, output: serde_json::Value) -> String {
        let result = output["result"].as_bool().unwrap_or(true);
        if result {
            "true".to_string()
        } else {
            "false".to_string()
        }
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Passthrough Node - passes through state without modification
pub struct PassthroughNode {
    config: NodeConfig,
}

impl PassthroughNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for PassthroughNode {
    fn id(&self) -> String {
        "passthrough".to_string()
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({ "passthrough": true }))
    }

    fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}
