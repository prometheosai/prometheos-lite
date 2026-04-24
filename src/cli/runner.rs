//! CLI Runner for flow execution and flow file loading

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use prometheos_lite::flow::{Flow, FlowBuilder, Node, NodeConfig, SharedState};

/// Flow file format for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowFile {
    pub name: String,
    pub description: Option<String>,
    pub start_node: String,
    pub nodes: Vec<NodeDefinition>,
    pub transitions: Vec<TransitionDefinition>,
}

impl FlowFile {
    /// Validate the flow file structure according to Flow JSON Schema v1
    pub fn validate(&self) -> Result<()> {
        // Validate name is not empty
        if self.name.is_empty() {
            anyhow::bail!("Flow name cannot be empty");
        }

        // Validate start_node is not empty
        if self.start_node.is_empty() {
            anyhow::bail!("Start node cannot be empty");
        }

        // Validate nodes is not empty
        if self.nodes.is_empty() {
            anyhow::bail!("Flow must have at least one node");
        }

        // Validate each node definition
        for node in &self.nodes {
            node.validate()?;
        }

        // Validate transitions
        for transition in &self.transitions {
            transition.validate()?;
        }

        // Validate start_node exists in nodes
        let node_ids: std::collections::HashSet<_> = self.nodes.iter().map(|n| &n.id).collect();
        if !node_ids.contains(&self.start_node) {
            anyhow::bail!("Start node '{}' not found in nodes", self.start_node);
        }

        // Validate all transition sources and targets exist
        for transition in &self.transitions {
            if !node_ids.contains(&transition.from) {
                anyhow::bail!("Transition source node '{}' not found", transition.from);
            }
            if !node_ids.contains(&transition.to) {
                anyhow::bail!("Transition target node '{}' not found", transition.to);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub id: String,
    pub node_type: String,
    pub config: Option<serde_json::Value>,
}

impl NodeDefinition {
    /// Validate node definition according to Flow JSON Schema v1
    pub fn validate(&self) -> Result<()> {
        // Validate id is not empty
        if self.id.is_empty() {
            anyhow::bail!("Node id cannot be empty");
        }

        // Validate node_type is not empty
        if self.node_type.is_empty() {
            anyhow::bail!("Node type cannot be empty");
        }

        // Validate node_type is one of the known types
        let valid_types = [
            "planner", "coder", "reviewer", "llm", "tool",
            "file_writer", "context_loader", "memory_write", "conditional"
        ];

        if !valid_types.contains(&self.node_type.as_str()) {
            // Warn but don't fail - will default to passthrough
            eprintln!("Warning: Unknown node type '{}', will use passthrough", self.node_type);
        }

        // Validate config if present
        if let Some(config) = &self.config {
            if !config.is_object() {
                anyhow::bail!("Node config must be a JSON object");
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionDefinition {
    pub from: String,
    pub action: String,
    pub to: String,
}

impl TransitionDefinition {
    /// Validate transition definition
    pub fn validate(&self) -> Result<()> {
        if self.from.is_empty() {
            anyhow::bail!("Transition 'from' cannot be empty");
        }
        if self.action.is_empty() {
            anyhow::bail!("Transition 'action' cannot be empty");
        }
        if self.to.is_empty() {
            anyhow::bail!("Transition 'to' cannot be empty");
        }
        Ok(())
    }
}

/// CLI Runner for executing flows
pub struct FlowRunner {
    flow: Flow,
    tracer: Option<prometheos_lite::flow::tracing::SharedTracer>,
    debug_mode: bool,
    runtime: Option<prometheos_lite::flow::RuntimeContext>,
}

impl FlowRunner {
    /// Create a new FlowRunner from a Flow
    pub fn new(flow: Flow) -> Self {
        Self {
            flow,
            tracer: None,
            debug_mode: false,
            runtime: None,
        }
    }

    /// Enable debug mode
    pub fn enable_debug_mode(&mut self) {
        self.debug_mode = true;
    }

    /// Get the tracer if set
    pub fn tracer(&self) -> Option<&prometheos_lite::flow::tracing::SharedTracer> {
        self.tracer.as_ref()
    }

    /// Set the runtime context for service injection
    pub fn with_runtime(mut self, runtime: prometheos_lite::flow::RuntimeContext) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Load a flow from a JSON file
    pub fn from_json_file(path: PathBuf) -> Result<Self> {
        Self::from_json_file_with_runtime(path, None)
    }

    /// Load a flow from a JSON file with a RuntimeContext
    pub fn from_json_file_with_runtime(
        path: PathBuf,
        runtime: Option<prometheos_lite::flow::RuntimeContext>,
    ) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read flow file: {}", path.display()))?;

        let flow_file: FlowFile =
            serde_json::from_str(&content).context("Failed to parse flow file")?;

        // Validate flow file structure according to Flow JSON Schema v1
        flow_file.validate().context("Flow file validation failed")?;

        let flow = Self::build_flow_from_file(flow_file, runtime.clone())?;

        // Create tracer for this flow
        let tracer = prometheos_lite::flow::tracing::create_tracer();

        Ok(Self {
            flow,
            tracer: Some(tracer),
            debug_mode: false,
            runtime,
        })
    }

    /// Build a Flow from a FlowFile using the NodeFactory
    fn build_flow_from_file(
        file: FlowFile,
        runtime: Option<prometheos_lite::flow::RuntimeContext>,
    ) -> Result<Flow> {
        let mut builder = FlowBuilder::new().start(file.start_node.clone());
        let factory: Box<dyn NodeFactory> = if let Some(rt) = runtime {
            Box::new(DefaultNodeFactory::from_runtime(rt))
        } else {
            Box::new(DefaultNodeFactory::new())
        };

        for node_def in &file.nodes {
            let node = factory.create(&node_def.node_type, node_def.config.clone())?;
            // Wrap the node in an IdWrapper to override the id
            let wrapped_node = Arc::new(IdWrapper::new(node_def.id.clone(), node));
            builder = builder.add_node(node_def.id.clone(), wrapped_node);
        }

        // Add transitions
        for transition in &file.transitions {
            builder = builder.add_transition(
                transition.from.clone(),
                transition.action.clone(),
                transition.to.clone(),
            );
        }

        builder.build().context("Failed to build flow from file")
    }

    /// Execute the flow with the given state
    pub async fn run(&mut self, state: &mut SharedState) -> Result<()> {
        self.flow.run(state).await
    }

    /// Run the flow with input data
    pub async fn run_with_input(&mut self, input: serde_json::Value) -> Result<SharedState> {
        let mut state = SharedState::new();

        // Inject input into state
        if let Some(obj) = input.as_object() {
            for (key, value) in obj {
                state.set_input(key.clone(), value.clone());
            }
        }

        // Attach tracer to flow if available
        if let Some(tracer) = &self.tracer {
            let flow_with_tracer = self.flow.clone().with_tracer(tracer.clone());
            self.flow = flow_with_tracer;
        }

        self.flow.run(&mut state).await?;
        Ok(state)
    }
}

/// NodeFactory trait - creates concrete nodes based on node_type
pub trait NodeFactory: Send + Sync {
    /// Create a node from node_type and optional config
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>>;
}

/// Default implementation of NodeFactory
pub struct DefaultNodeFactory {
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
    tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl DefaultNodeFactory {
    pub fn new() -> Self {
        Self {
            model_router: None,
            tool_runtime: None,
            memory_service: None,
        }
    }

    /// Create a DefaultNodeFactory from a RuntimeContext
    pub fn from_runtime(runtime: prometheos_lite::flow::RuntimeContext) -> Self {
        Self {
            model_router: runtime.model_router,
            tool_runtime: runtime.tool_runtime,
            memory_service: runtime.memory_service,
        }
    }

    pub fn with_model_router(mut self, router: std::sync::Arc<prometheos_lite::flow::ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    pub fn with_tool_runtime(mut self, runtime: std::sync::Arc<prometheos_lite::flow::ToolRuntime>) -> Self {
        self.tool_runtime = Some(runtime);
        self
    }

    pub fn with_memory_service(mut self, service: std::sync::Arc<prometheos_lite::flow::MemoryService>) -> Self {
        self.memory_service = Some(service);
        self
    }
}

impl NodeFactory for DefaultNodeFactory {
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>> {
        let node_config = Self::parse_config(&config)?;

        match node_type {
            "planner" => Ok(Arc::new(PlannerNode::new(node_config, self.model_router.clone()))),
            "coder" => Ok(Arc::new(CoderNode::new(node_config, self.model_router.clone()))),
            "reviewer" => Ok(Arc::new(ReviewerNode::new(node_config, self.model_router.clone()))),
            "llm" => Ok(Arc::new(LlmNode::new(node_config, self.model_router.clone(), config))),
            "tool" => Ok(Arc::new(ToolNode::new(node_config, self.tool_runtime.clone()))),
            "file_writer" => Ok(Arc::new(FileWriterNode::new(node_config))),
            "context_loader" => Ok(Arc::new(ContextLoaderNode::new(node_config, self.memory_service.clone()))),
            "memory_write" => Ok(Arc::new(MemoryWriteNode::new(node_config, self.memory_service.clone()))),
            "conditional" => Ok(Arc::new(ConditionalNode::new(node_config))),
            _ => {
                // Default to passthrough for unknown types
                Ok(Arc::new(PassthroughNode::new(node_config)))
            }
        }
    }
}

impl DefaultNodeFactory {
    fn parse_config(config: &Option<serde_json::Value>) -> Result<NodeConfig> {
        if let Some(cfg) = config {
            let retries = cfg["retries"].as_u64().unwrap_or(3) as u8;
            let retry_delay_ms = cfg["retry_delay_ms"].as_u64().unwrap_or(100);
            let timeout_ms = cfg["timeout_ms"].as_u64();

            Ok(NodeConfig {
                retries,
                retry_delay_ms,
                timeout_ms,
            })
        } else {
            Ok(NodeConfig::default())
        }
    }
}

impl Default for DefaultNodeFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// IdWrapper - wraps a node to override its id
struct IdWrapper {
    id: String,
    inner: Arc<dyn Node>,
}

impl IdWrapper {
    fn new(id: String, inner: Arc<dyn Node>) -> Self {
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
struct PlannerNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl PlannerNode {
    fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
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
struct CoderNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl CoderNode {
    fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
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
struct ReviewerNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
}

impl ReviewerNode {
    fn new(config: NodeConfig, model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>) -> Self {
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
struct LlmNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
    prompt_template: Option<String>,
}

impl LlmNode {
    fn new(
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
struct ToolNode {
    config: NodeConfig,
    tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>,
}

impl ToolNode {
    fn new(config: NodeConfig, tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>) -> Self {
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
struct FileWriterNode {
    config: NodeConfig,
}

impl FileWriterNode {
    fn new(config: NodeConfig) -> Self {
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
struct ContextLoaderNode {
    config: NodeConfig,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl ContextLoaderNode {
    fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>) -> Self {
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
struct MemoryWriteNode {
    config: NodeConfig,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl MemoryWriteNode {
    fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>) -> Self {
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
            // Use MemoryService for actual write
            let memory_id = service.create_memory(content.to_string(), prometheos_lite::flow::MemoryType::Semantic, serde_json::json!({})).await?;
            Ok(serde_json::json!({ "memory_id": memory_id }))
        } else {
            // Fallback to placeholder if no MemoryService
            Ok(serde_json::json!({
                "memory_id": "placeholder_id"
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
struct ConditionalNode {
    config: NodeConfig,
}

impl ConditionalNode {
    fn new(config: NodeConfig) -> Self {
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
struct PassthroughNode {
    config: NodeConfig,
}

impl PassthroughNode {
    fn new(config: NodeConfig) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_file_serialization() {
        let flow_file = FlowFile {
            name: "test_flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "node1".to_string(),
            nodes: vec![NodeDefinition {
                id: "node1".to_string(),
                node_type: "placeholder".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let json = serde_json::to_string(&flow_file).unwrap();
        let parsed: FlowFile = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test_flow");
        assert_eq!(parsed.start_node, "node1");
    }

    #[tokio::test]
    async fn test_flow_runner() {
        let node = Arc::new(PassthroughNode::new(NodeConfig::default()));
        let wrapped = Arc::new(IdWrapper::new("test".to_string(), node));

        let flow = FlowBuilder::new()
            .start("test".to_string())
            .add_node("test".to_string(), wrapped)
            .build()
            .unwrap();

        let mut runner = FlowRunner::new(flow);

        let mut state = SharedState::new();
        state.set_input("test".to_string(), serde_json::json!("value"));

        let result = runner.run(&mut state).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_node_factory_llm() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("llm", None).unwrap();
        assert_eq!(node.id(), "llm");
    }

    #[test]
    fn test_node_factory_tool() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("tool", None).unwrap();
        assert_eq!(node.id(), "tool");
    }

    #[test]
    fn test_node_factory_context_loader() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("context_loader", None).unwrap();
        assert_eq!(node.id(), "context_loader");
    }

    #[test]
    fn test_node_factory_memory_write() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("memory_write", None).unwrap();
        assert_eq!(node.id(), "memory_write");
    }

    #[test]
    fn test_node_factory_conditional() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("conditional", None).unwrap();
        assert_eq!(node.id(), "conditional");
    }

    #[test]
    fn test_node_factory_planner() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("planner", None).unwrap();
        assert_eq!(node.id(), "planner");
    }

    #[test]
    fn test_node_factory_coder() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("coder", None).unwrap();
        assert_eq!(node.id(), "coder");
    }

    #[test]
    fn test_node_factory_reviewer() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("reviewer", None).unwrap();
        assert_eq!(node.id(), "reviewer");
    }

    #[test]
    fn test_node_factory_with_model_router() {
        let factory = DefaultNodeFactory::new();
        assert!(factory.model_router.is_none());
    }

    #[test]
    fn test_node_factory_file_writer() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("file_writer", None).unwrap();
        assert_eq!(node.id(), "file_writer");
    }

    #[test]
    fn test_node_factory_unknown_type() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let node = factory.create("unknown_type", None).unwrap();
        assert_eq!(node.id(), "passthrough");
    }

    #[test]
    fn test_node_factory_config_parsing() {
        let factory: Box<dyn NodeFactory> = Box::new(DefaultNodeFactory::new());
        let config = serde_json::json!({
            "retries": 5,
            "retry_delay_ms": 200,
            "timeout_ms": 60000
        });

        let node = factory.create("llm", Some(config)).unwrap();
        let node_config = node.config();
        assert_eq!(node_config.retries, 5);
        assert_eq!(node_config.retry_delay_ms, 200);
        assert_eq!(node_config.timeout_ms, Some(60000));
    }
}
