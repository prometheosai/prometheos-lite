//! Built-in node implementations

use crate::flow::SharedState;
use crate::flow::node::{Node, NodeConfig};
use anyhow::{Context, Result};
use std::sync::Arc;

// Import guardrail database operations
use crate::db::repository::OutboxOperations;

use crate::context::{ContextBuilder, ContextInputs};
use crate::flow::{MemoryService, MemoryType, ModelRouter, ToolRuntime};
use crate::personality::{ConstitutionalFilter, PersonalityMode, PromptContext};

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

    fn kind(&self) -> &str {
        self.inner.kind()
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
    model_router: Option<std::sync::Arc<ModelRouter>>,
    context_builder: ContextBuilder,
}

impl PlannerNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            config,
            model_router,
            context_builder,
        }
    }
}

#[async_trait::async_trait]
impl Node for PlannerNode {
    fn id(&self) -> String {
        "planner".to_string()
    }

    fn kind(&self) -> &str {
        "planner"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before LLM call
        state
            .check_llm_budget()
            .context("LLM call budget exceeded")?;

        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .context("PlannerNode requires task input")?
            .to_string();

        // Include personality mode in input for use in exec()
        let personality_mode = state.get_personality_mode();

        Ok(serde_json::json!({
            "task": task,
            "personality_mode": personality_mode
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let task = input["task"]
            .as_str()
            .context("Missing task in planner node input")?;

        let router = self
            .model_router
            .as_ref()
            .context("PlannerNode requires ModelRouter to be configured")?;

        // Use memory-aware context building if memory service is available
        let built_context = if self.context_builder.memory_service().is_some() {
            self.context_builder
                .build_with_memory_retrieval(
                    task.to_string(),
                    None, // project_id - could be passed from state in future
                    5,    // Retrieve top 5 relevant memories
                )
                .await
                .context("Failed to build context with memory retrieval")?
        } else {
            let context_inputs = ContextInputs {
                task: task.to_string(),
                plan: None,
                memory: Vec::new(),
                artifacts: Vec::new(),
                system_prompt: Some("You are a planning assistant. Create a structured plan for the following task. Provide a step-by-step plan as a JSON array of strings.".to_string()),
            };
            self.context_builder
                .build(context_inputs)
                .context("Failed to build context with ContextBuilder")?
        };

        let base_prompt = built_context.prompt;

        // Inject personality context if mode is set
        let enhanced_prompt = if let Some(mode_str) = input["personality_mode"].as_str() {
            if let Some(mode) = PersonalityMode::parse(mode_str) {
                let prompt_context = PromptContext::new(mode);
                prompt_context.inject_into_prompt(&base_prompt)
            } else {
                base_prompt
            }
        } else {
            base_prompt
        };

        let result = router.generate_with_metadata(&enhanced_prompt).await?;
        Ok(serde_json::json!({
            "plan": result.content,
            "_metadata": serde_json::to_value(&result).unwrap_or(serde_json::json!({}))
        }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(plan) = output["plan"].as_str() {
            state.set_working("plan".to_string(), serde_json::json!(plan));
        } else if let Some(plan) = output["plan"].as_array() {
            state.set_working("plan".to_string(), serde_json::json!(plan));
        }

        // Store execution metadata if available
        if let Some(metadata) = output.get("_metadata") {
            state.add_execution_metadata(self.id(), metadata.clone());
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
    model_router: Option<std::sync::Arc<ModelRouter>>,
    context_builder: ContextBuilder,
}

impl CoderNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            config,
            model_router,
            context_builder,
        }
    }
}

#[async_trait::async_trait]
impl Node for CoderNode {
    fn id(&self) -> String {
        "coder".to_string()
    }

    fn kind(&self) -> &str {
        "coder"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before LLM call
        state
            .check_llm_budget()
            .context("LLM call budget exceeded")?;

        let plan = state
            .get_working("plan")
            .cloned()
            .unwrap_or(serde_json::json!(null));
        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .context("CoderNode requires task input")?
            .to_string();

        // Include personality mode in input for use in exec()
        let personality_mode = state.get_personality_mode();

        Ok(serde_json::json!({
            "task": task,
            "plan": plan,
            "personality_mode": personality_mode
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let task = input["task"]
            .as_str()
            .context("Missing task in coder node input")?;
        let plan = &input["plan"];

        let router = self
            .model_router
            .as_ref()
            .context("CoderNode requires ModelRouter to be configured")?;

        let plan_str = serde_json::to_string(plan).unwrap_or_default();

        // Use memory-aware context building if memory service is available
        let built_context = if self.context_builder.memory_service().is_some() {
            self.context_builder
                .build_with_memory_retrieval(
                    task.to_string(),
                    None, // project_id - could be passed from state in future
                    5,    // Retrieve top 5 relevant memories
                )
                .await
                .context("Failed to build context with memory retrieval")?
        } else {
            let context_inputs = ContextInputs {
                task: task.to_string(),
                plan: Some(plan_str),
                memory: Vec::new(),
                artifacts: Vec::new(),
                system_prompt: Some("You are a coding assistant. Generate code for the following task based on the provided plan. Provide the generated code only, without explanations.".to_string()),
            };
            self.context_builder
                .build(context_inputs)
                .context("Failed to build context with ContextBuilder")?
        };

        let base_prompt = built_context.prompt;

        // Inject personality context if mode is set
        let enhanced_prompt = if let Some(mode_str) = input["personality_mode"].as_str() {
            if let Some(mode) = PersonalityMode::parse(mode_str) {
                let prompt_context = PromptContext::new(mode);
                prompt_context.inject_into_prompt(&base_prompt)
            } else {
                base_prompt
            }
        } else {
            base_prompt
        };

        let result = router.generate_with_metadata(&enhanced_prompt).await?;
        Ok(serde_json::json!({
            "generated_code": result.content,
            "_metadata": serde_json::to_value(&result).unwrap_or(serde_json::json!({}))
        }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(code) = output["generated_code"].as_str() {
            // Apply constitutional filter based on personality mode
            let filtered_code = if let Some(mode_str) = state.get_personality_mode() {
                if let Some(mode) = PersonalityMode::parse(&mode_str) {
                    let filter = ConstitutionalFilter::new(mode);
                    filter.filter(code)
                } else {
                    code.to_string()
                }
            } else {
                code.to_string()
            };

            state.set_output("generated".to_string(), serde_json::json!(filtered_code));

            if filtered_code.trim().is_empty() {
                return "needs_revision".to_string();
            }
        }

        // Store execution metadata if available
        if let Some(metadata) = output.get("_metadata") {
            state.add_execution_metadata(self.id(), metadata.clone());
        }

        "complete".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Reviewer Node - reviews generated output
pub struct ReviewerNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<ModelRouter>>,
    context_builder: ContextBuilder,
}

impl ReviewerNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            config,
            model_router,
            context_builder,
        }
    }
}

#[async_trait::async_trait]
impl Node for ReviewerNode {
    fn id(&self) -> String {
        "reviewer".to_string()
    }

    fn kind(&self) -> &str {
        "reviewer"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before LLM call
        state
            .check_llm_budget()
            .context("LLM call budget exceeded")?;

        let generated = state
            .get_output("generated")
            .cloned()
            .unwrap_or(serde_json::json!(null));

        // Include personality mode in input for use in exec()
        let personality_mode = state.get_personality_mode();

        Ok(serde_json::json!({
            "generated": generated,
            "personality_mode": personality_mode
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let generated = &input["generated"];

        let router = self
            .model_router
            .as_ref()
            .context("ReviewerNode requires ModelRouter to be configured")?;

        let generated_str = serde_json::to_string(generated).unwrap_or_default();
        let review_task = format!(
            "Review the following generated code for quality and correctness:\n\n{}",
            generated_str
        );

        // Use memory-aware context building if memory service is available
        let built_context = if self.context_builder.memory_service().is_some() {
            self.context_builder
                .build_with_memory_retrieval(
                    review_task.clone(),
                    None, // project_id - could be passed from state in future
                    3,    // Retrieve top 3 relevant memories for review context
                )
                .await
                .context("Failed to build context with memory retrieval")?
        } else {
            let context_inputs = ContextInputs {
                task: review_task,
                plan: None,
                memory: Vec::new(),
                artifacts: Vec::new(),
                system_prompt: Some("You are a code reviewer. Provide a concise review with correctness checks and concrete improvements.".to_string()),
            };
            self.context_builder
                .build(context_inputs)
                .context("Failed to build context with ContextBuilder")?
        };

        let base_prompt = built_context.prompt;

        // Inject personality context if mode is set
        let enhanced_prompt = if let Some(mode_str) = input["personality_mode"].as_str() {
            if let Some(mode) = PersonalityMode::parse(mode_str) {
                let prompt_context = PromptContext::new(mode);
                prompt_context.inject_into_prompt(&base_prompt)
            } else {
                base_prompt
            }
        } else {
            base_prompt
        };

        let result = router.generate_with_metadata(&enhanced_prompt).await?;
        Ok(serde_json::json!({
            "review": result.content,
            "_metadata": serde_json::to_value(&result).unwrap_or(serde_json::json!({}))
        }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(review) = output["review"].as_str() {
            // Apply constitutional filter based on personality mode
            let filtered_review = if let Some(mode_str) = state.get_personality_mode() {
                if let Some(mode) = PersonalityMode::parse(&mode_str) {
                    let filter = ConstitutionalFilter::new(mode);
                    filter.filter(review)
                } else {
                    review.to_string()
                }
            } else {
                review.to_string()
            };

            state.set_output("review".to_string(), serde_json::json!(filtered_review));
        }

        // Store execution metadata if available
        if let Some(metadata) = output.get("_metadata") {
            state.add_execution_metadata(self.id(), metadata.clone());
        }

        let review_text = output["review"].as_str().unwrap_or_default().to_lowercase();
        if review_text.contains("needs_revision")
            || review_text.contains("needs revision")
            || review_text.contains("revise")
            || review_text.contains("fix")
        {
            "needs_revision".to_string()
        } else {
            "approved".to_string()
        }
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// LLM Node - executes an LLM call with configurable prompt template
pub struct LlmNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<ModelRouter>>,
    prompt_template: Option<String>,
    context_builder: ContextBuilder,
}

impl LlmNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        node_config: Option<serde_json::Value>,
        context_builder: ContextBuilder,
    ) -> Self {
        let prompt_template = node_config
            .as_ref()
            .and_then(|cfg| cfg["prompt_template"].as_str())
            .map(|s| s.to_string());
        Self {
            config,
            model_router,
            prompt_template,
            context_builder,
        }
    }
}

#[async_trait::async_trait]
impl Node for LlmNode {
    fn id(&self) -> String {
        "llm".to_string()
    }

    fn kind(&self) -> &str {
        "llm"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before LLM call
        state
            .check_llm_budget()
            .context("LLM call budget exceeded")?;

        let prompt = state
            .get_input("prompt")
            .and_then(|v| v.as_str())
            .context("LlmNode requires prompt input")?
            .to_string();

        // Include personality mode in input for use in exec()
        let personality_mode = state.get_personality_mode();

        Ok(serde_json::json!({
            "prompt": prompt,
            "personality_mode": personality_mode
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let prompt = input["prompt"]
            .as_str()
            .context("Missing prompt in LLM node input")?;

        let router = self
            .model_router
            .as_ref()
            .context("LlmNode requires ModelRouter to be configured")?;

        // Use memory-aware context building if memory service is available
        let built_context = if self.context_builder.memory_service().is_some() {
            self.context_builder
                .build_with_memory_retrieval(
                    prompt.to_string(),
                    None, // project_id - could be passed from state in future
                    5,    // Retrieve top 5 relevant memories
                )
                .await
                .context("Failed to build context with memory retrieval")?
        } else {
            let context_inputs = ContextInputs {
                task: prompt.to_string(),
                plan: None,
                memory: Vec::new(),
                artifacts: Vec::new(),
                system_prompt: None,
            };
            self.context_builder
                .build(context_inputs)
                .context("Failed to build context with ContextBuilder")?
        };

        let final_prompt = if let Some(template) = &self.prompt_template {
            template.replace("{{prompt}}", &built_context.prompt)
        } else {
            built_context.prompt
        };

        // Inject personality context if mode is set
        let enhanced_prompt = if let Some(mode_str) = input["personality_mode"].as_str() {
            if let Some(mode) = PersonalityMode::parse(mode_str) {
                let prompt_context = PromptContext::new(mode);
                prompt_context.inject_into_prompt(&final_prompt)
            } else {
                final_prompt
            }
        } else {
            final_prompt
        };

        let result = router.generate_with_metadata(&enhanced_prompt).await?;
        Ok(serde_json::json!({
            "response": result.content,
            "_metadata": serde_json::to_value(&result).unwrap_or(serde_json::json!({}))
        }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        if let Some(response) = output["response"].as_str() {
            // Apply constitutional filter based on personality mode
            let filtered_response = if let Some(mode_str) = state.get_personality_mode() {
                if let Some(mode) = PersonalityMode::parse(&mode_str) {
                    let filter = ConstitutionalFilter::new(mode);
                    filter.filter(response)
                } else {
                    response.to_string()
                }
            } else {
                response.to_string()
            };

            state.set_output(
                "llm_response".to_string(),
                serde_json::json!(filtered_response),
            );
        }

        // Store execution metadata if available
        if let Some(metadata) = output.get("_metadata") {
            state.add_execution_metadata(self.id(), metadata.clone());
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
    tool_runtime: Option<std::sync::Arc<ToolRuntime>>,
}

impl ToolNode {
    pub fn new(config: NodeConfig, tool_runtime: Option<std::sync::Arc<ToolRuntime>>) -> Self {
        Self {
            config,
            tool_runtime,
        }
    }
}

#[async_trait::async_trait]
impl Node for ToolNode {
    fn id(&self) -> String {
        "tool".to_string()
    }

    fn kind(&self) -> &str {
        "tool"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before tool call
        state
            .check_tool_budget()
            .context("Tool call budget exceeded")?;

        let tool_name = state
            .get_input("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tool_args = state
            .get_input("tool_args")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Build ToolContext from state
        let run_id = state.get_run_id().unwrap_or("unknown".to_string());
        let trace_id = state.get_trace_id().unwrap_or("unknown".to_string());
        let node_id = self.id();

        // Get tool policy from state or use conservative default
        let policy = state
            .get_budget_report()
            .and_then(|report| {
                report.get("tool_policy").and_then(|p| {
                    serde_json::from_value::<crate::tools::ToolPolicy>(p.clone()).ok()
                })
            })
            .unwrap_or_else(crate::tools::ToolPolicy::conservative);

        let work_domain = state
            .get_input("work_domain")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let work_phase = state
            .get_input("work_phase")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let context =
            crate::tools::ToolContext::new(run_id, trace_id, node_id, tool_name.clone(), policy)
                .with_work_context(work_domain, work_phase);

        Ok(serde_json::json!({
            "tool_name": tool_name,
            "tool_args": tool_args,
            "tool_context": context
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let tool_name = input["tool_name"]
            .as_str()
            .context("Missing tool_name in tool node input")?;
        let tool_args = &input["tool_args"];

        // Extract ToolContext from input
        let context: crate::tools::ToolContext =
            serde_json::from_value(input["tool_context"].clone())
                .context("Missing or invalid tool_context in tool node input")?;

        let runtime = self
            .tool_runtime
            .as_ref()
            .context("ToolNode requires ToolRuntime to be configured")?;

        // Parse tool_args as a command and arguments
        let args: Vec<String> = if let Some(arr) = tool_args.as_array() {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        } else {
            vec![]
        };

        let result = runtime.execute_command(tool_name, args, &context).await?;
        Ok(serde_json::json!({ "result": result }))
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

pub struct HarnessRepoMapNode {
    config: NodeConfig,
    repo_path: std::path::PathBuf,
}

impl HarnessRepoMapNode {
    pub fn new(config: NodeConfig, repo_path: std::path::PathBuf) -> Self {
        Self { config, repo_path }
    }
}

#[async_trait::async_trait]
impl Node for HarnessRepoMapNode {
    fn id(&self) -> String {
        "harness.repo_map".to_string()
    }
    fn kind(&self) -> &str {
        "harness.repo_map"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        super::coding_nodes::CodeAnalysisNode::new(self.config.clone(), self.repo_path.clone())
            .prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        super::coding_nodes::CodeAnalysisNode::new(self.config.clone(), self.repo_path.clone())
            .exec(input)
            .await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        super::coding_nodes::CodeAnalysisNode::new(self.config.clone(), self.repo_path.clone())
            .post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessPatchApplyNode {
    config: NodeConfig,
    tool_runtime: Option<std::sync::Arc<ToolRuntime>>,
}

impl HarnessPatchApplyNode {
    pub fn new(config: NodeConfig, tool_runtime: Option<std::sync::Arc<ToolRuntime>>) -> Self {
        Self {
            config,
            tool_runtime,
        }
    }
}

#[async_trait::async_trait]
impl Node for HarnessPatchApplyNode {
    fn id(&self) -> String {
        "harness.patch_apply".to_string()
    }
    fn kind(&self) -> &str {
        "harness.patch_apply"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone()).prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone())
            .exec(input)
            .await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone()).post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessValidateNode {
    config: NodeConfig,
    tool_runtime: Option<std::sync::Arc<ToolRuntime>>,
}

impl HarnessValidateNode {
    pub fn new(config: NodeConfig, tool_runtime: Option<std::sync::Arc<ToolRuntime>>) -> Self {
        Self {
            config,
            tool_runtime,
        }
    }
}

#[async_trait::async_trait]
impl Node for HarnessValidateNode {
    fn id(&self) -> String {
        "harness.validate".to_string()
    }
    fn kind(&self) -> &str {
        "harness.validate"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone()).prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone())
            .exec(input)
            .await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        ToolNode::new(self.config.clone(), self.tool_runtime.clone()).post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessReviewNode {
    config: NodeConfig,
    model_router: Option<std::sync::Arc<ModelRouter>>,
    context_builder: ContextBuilder,
}

impl HarnessReviewNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            config,
            model_router,
            context_builder,
        }
    }
}

#[async_trait::async_trait]
impl Node for HarnessReviewNode {
    fn id(&self) -> String {
        "harness.review".to_string()
    }
    fn kind(&self) -> &str {
        "harness.review"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        ReviewerNode::new(
            self.config.clone(),
            self.model_router.clone(),
            self.context_builder.clone(),
        )
        .prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        ReviewerNode::new(
            self.config.clone(),
            self.model_router.clone(),
            self.context_builder.clone(),
        )
        .exec(input)
        .await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        ReviewerNode::new(
            self.config.clone(),
            self.model_router.clone(),
            self.context_builder.clone(),
        )
        .post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessRiskNode(HarnessReviewNode);

impl HarnessRiskNode {
    pub fn new(
        config: NodeConfig,
        model_router: Option<std::sync::Arc<ModelRouter>>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self(HarnessReviewNode::new(
            config,
            model_router,
            context_builder,
        ))
    }
}

#[async_trait::async_trait]
impl Node for HarnessRiskNode {
    fn id(&self) -> String {
        "harness.risk".to_string()
    }
    fn kind(&self) -> &str {
        "harness.risk"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        self.0.prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        self.0.exec(input).await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        self.0.post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.0.config()
    }
}

pub struct HarnessCompletionNode {
    config: NodeConfig,
}

impl HarnessCompletionNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for HarnessCompletionNode {
    fn id(&self) -> String {
        "harness.completion".to_string()
    }
    fn kind(&self) -> &str {
        "harness.completion"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        TerminalNode::new(self.config.clone()).prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        TerminalNode::new(self.config.clone()).exec(input).await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        TerminalNode::new(self.config.clone()).post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessAttemptPoolNode {
    config: NodeConfig,
}

impl HarnessAttemptPoolNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for HarnessAttemptPoolNode {
    fn id(&self) -> String {
        "harness.attempt_pool".to_string()
    }
    fn kind(&self) -> &str {
        "harness.attempt_pool"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        ConditionalNode::new(self.config.clone()).prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        ConditionalNode::new(self.config.clone()).exec(input).await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        ConditionalNode::new(self.config.clone()).post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

pub struct HarnessContextDistillNode {
    config: NodeConfig,
    memory_service: Option<std::sync::Arc<MemoryService>>,
}

impl HarnessContextDistillNode {
    pub fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<MemoryService>>) -> Self {
        Self {
            config,
            memory_service,
        }
    }
}

#[async_trait::async_trait]
impl Node for HarnessContextDistillNode {
    fn id(&self) -> String {
        "harness.context_distill".to_string()
    }
    fn kind(&self) -> &str {
        "harness.context_distill"
    }
    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        ContextLoaderNode::new(self.config.clone(), self.memory_service.clone()).prep(state)
    }
    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        ContextLoaderNode::new(self.config.clone(), self.memory_service.clone())
            .exec(input)
            .await
    }
    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        ContextLoaderNode::new(self.config.clone(), self.memory_service.clone()).post(state, output)
    }
    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
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

    fn kind(&self) -> &str {
        "file_writer"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before file write
        state
            .check_memory_write_budget()
            .context("Memory write budget exceeded")?;

        let content = state
            .get_output("generated")
            .cloned()
            .unwrap_or(serde_json::json!(null));
        let file_path = state
            .get_input("file_path")
            .and_then(|v| v.as_str())
            .context("FileWriterNode requires file_path input")?
            .to_string();

        // Build ToolContext from state
        let run_id = state.get_run_id().unwrap_or("unknown".to_string());
        let trace_id = state.get_trace_id().unwrap_or("unknown".to_string());
        let node_id = self.id();

        // Get tool policy from state or use conservative default
        let policy = state
            .get_budget_report()
            .and_then(|report| {
                report.get("tool_policy").and_then(|p| {
                    serde_json::from_value::<crate::tools::ToolPolicy>(p.clone()).ok()
                })
            })
            .unwrap_or_else(crate::tools::ToolPolicy::conservative);

        let work_domain = state
            .get_input("work_domain")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let work_phase = state
            .get_input("work_phase")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let context = crate::tools::ToolContext::new(
            run_id,
            trace_id,
            node_id,
            "file_writer".to_string(),
            policy,
        )
        .with_work_context(work_domain, work_phase);

        // Use PathGuard to validate the path
        let path_guard = crate::tools::PathGuard::default();
        let canonical_path = path_guard.validate_path(&file_path)?;

        Ok(serde_json::json!({
            "content": content,
            "file_path": canonical_path,
            "tool_context": context
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let content = input["content"]
            .as_str()
            .context("Missing content in file writer node input")?;
        let file_path = input["file_path"]
            .as_str()
            .context("Missing file_path in file writer node input")?;

        // Extract ToolContext for idempotency
        let context: crate::tools::ToolContext =
            serde_json::from_value(input["tool_context"].clone())
                .context("Missing or invalid tool_context in file writer node input")?;

        // Generate idempotency key for this operation
        let operation_hash = crate::flow::IdempotencyKey::compute_operation_hash(
            "file_writer",
            &serde_json::json!({"path": file_path, "content": content}),
        );
        let idempotency_key = crate::flow::IdempotencyKey::new(
            context.run_id.clone(),
            context.node_id.clone(),
            operation_hash,
        );

        // Check outbox for duplicate operation
        let db_path = ".prometheos/runs.db";
        if std::path::Path::new(db_path).exists()
            && let Ok(db) = crate::db::repository::Db::new(db_path)
        {
            if let Ok(existing) = OutboxOperations::get_outbox_entry_by_hash(
                &db,
                &context.run_id,
                &context.node_id,
                &idempotency_key.key,
            ) && let Some(entry) = existing
                && entry.status == "completed"
            {
                // Return cached result instead of re-executing
                return Ok(serde_json::json!({
                    "success": true,
                    "file_path": file_path,
                    "bytes_written": content.len(),
                    "idempotency_key": idempotency_key.key,
                    "from_cache": true,
                    "cached_output": entry.output
                }));
            }

            // Create outbox entry for this operation
            let _ = OutboxOperations::create_outbox_entry(
                &db,
                &context.run_id,
                &context.trace_id,
                &context.node_id,
                "file_writer",
                &idempotency_key.key,
            );
        }

        // Proceed with write
        std::fs::write(file_path, content)
            .with_context(|| format!("Failed to write file: {}", file_path))?;

        // Mark outbox entry as completed
        if std::path::Path::new(db_path).exists()
            && let Ok(db) = crate::db::repository::Db::new(db_path)
            && let Ok(entry) = OutboxOperations::get_outbox_entry_by_hash(
                &db,
                &context.run_id,
                &context.node_id,
                &idempotency_key.key,
            )
            && let Some(ref entry) = entry
        {
            let _ = OutboxOperations::mark_outbox_completed(
                &db,
                &entry.id,
                &serde_json::json!({
                    "success": true,
                    "file_path": file_path,
                    "bytes_written": content.len()
                })
                .to_string(),
            );
        }

        Ok(serde_json::json!({
            "success": true,
            "file_path": file_path,
            "bytes_written": content.len(),
            "idempotency_key": idempotency_key.key,
            "from_cache": false
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
    memory_service: Option<std::sync::Arc<MemoryService>>,
}

impl ContextLoaderNode {
    pub fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<MemoryService>>) -> Self {
        Self {
            config,
            memory_service,
        }
    }
}

#[async_trait::async_trait]
impl Node for ContextLoaderNode {
    fn id(&self) -> String {
        "context_loader".to_string()
    }

    fn kind(&self) -> &str {
        "context_loader"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before memory read
        state
            .check_memory_read_budget()
            .context("Memory read budget exceeded")?;

        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .context("MemoryReadNode requires task input")?
            .to_string();

        Ok(serde_json::json!({ "query": task }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let query = input["query"]
            .as_str()
            .context("Missing query in context loader node input")?;

        let service = self
            .memory_service
            .as_ref()
            .context("ContextLoaderNode requires MemoryService to be configured")?;

        // Use MemoryService for actual retrieval
        let memories = service.semantic_search(query, 5).await?;
        Ok(serde_json::json!({ "context": memories }))
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
    memory_service: Option<std::sync::Arc<MemoryService>>,
}

impl MemoryWriteNode {
    pub fn new(config: NodeConfig, memory_service: Option<std::sync::Arc<MemoryService>>) -> Self {
        Self {
            config,
            memory_service,
        }
    }
}

#[async_trait::async_trait]
impl Node for MemoryWriteNode {
    fn id(&self) -> String {
        "memory_write".to_string()
    }

    fn kind(&self) -> &str {
        "memory_write"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check budget before memory write
        state
            .check_memory_write_budget()
            .context("Memory write budget exceeded")?;

        let task = state
            .get_input("task")
            .and_then(|v| v.as_str())
            .context("MemoryWriteNode requires task input")?
            .to_string();

        Ok(serde_json::json!({ "task": task }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let content = input["task"]
            .as_str()
            .context("Missing task in memory write node input")?;

        let service = self
            .memory_service
            .as_ref()
            .context("MemoryWriteNode requires MemoryService to be configured")?;

        // Use MemoryService for actual write - fail on embedding server errors
        let memory_id = service
            .create_memory(
                content.to_string(),
                MemoryType::Semantic,
                serde_json::json!({}),
            )
            .await
            .context("Memory write failed - embedding server may be unavailable")?;

        Ok(serde_json::json!({ "memory_id": memory_id, "status": "success" }))
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

    fn kind(&self) -> &str {
        "conditional"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let condition = state
            .get_input("condition")
            .and_then(|v| v.as_str())
            .context("ConditionalNode requires condition input")?
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
                condition
                    .parse::<bool>()
                    .context("Failed to parse condition as boolean")?
            }
        };

        Ok(serde_json::json!({ "result": result }))
    }

    fn post(&self, _state: &mut SharedState, output: serde_json::Value) -> String {
        let result = output["result"]
            .as_bool()
            .context("ConditionalNode post requires boolean result");

        match result {
            Ok(true) => "true".to_string(),
            Ok(false) => "false".to_string(),
            Err(_) => "error".to_string(),
        }
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Passthrough Node - does nothing, passes through
pub struct PassthroughNode {
    config: NodeConfig,
}

/// Terminal Node - explicit end marker for flow graphs.
/// It performs no state mutation and returns no outgoing action.
pub struct TerminalNode {
    config: NodeConfig,
}

impl TerminalNode {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Node for TerminalNode {
    fn id(&self) -> String {
        "terminal".to_string()
    }

    fn kind(&self) -> &str {
        "terminal"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({ "terminal": true }))
    }

    fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
        "end".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
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

    fn kind(&self) -> &str {
        "passthrough"
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
    use crate::flow::SharedState;

    #[test]
    fn test_context_loader_prep_exec_match() {
        let node = ContextLoaderNode::new(NodeConfig::default(), None);
        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test query"));

        let prep_output = node.prep(&state).unwrap();
        assert!(prep_output.get("query").is_some());
        assert_eq!(prep_output["query"], "test query");
        assert!(prep_output.get("task").is_none());
    }
}
