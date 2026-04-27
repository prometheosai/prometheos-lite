//! Flow Execution Service - shared execution path for CLI and API
//!
//! This service consolidates flow execution logic that was previously
//! duplicated between the API handler and CLI runner. Both paths now
//! call through this single service.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Instant;

use crate::flow::budget::{BudgetGuard, ExecutionBudget};
use crate::flow::execution::Flow;
use crate::flow::factory::{DefaultNodeFactory, NodeFactory};
use crate::flow::loader::{FlowFile, FlowLoader, YamlLoader, JsonLoader};
use crate::flow::output::{Evaluation, FinalOutput};
use crate::flow::tracing::{RunId, SharedTracer, TraceEvent, Tracer};
use crate::flow::{RuntimeContext, SharedState};
use crate::intent::{DefaultFlowSelector, FlowSelector, Intent, IntentClassifier};

/// Options for flow execution
#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    /// Optional personality mode to inject
    pub personality_mode: Option<String>,
    /// Optional budget limits
    pub budget: Option<ExecutionBudget>,
    /// Optional tracer for structured logging
    pub tracer: Option<SharedTracer>,
    /// Optional override intent (skips classification)
    pub override_intent: Option<Intent>,
    /// Optional flows directory override
    pub flows_dir: Option<std::path::PathBuf>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            personality_mode: None,
            budget: None,
            tracer: None,
            override_intent: None,
            flows_dir: None,
        }
    }
}

impl ExecutionOptions {
    pub fn with_personality_mode(mut self, mode: String) -> Self {
        self.personality_mode = Some(mode);
        self
    }

    pub fn with_budget(mut self, budget: ExecutionBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    pub fn with_tracer(mut self, tracer: SharedTracer) -> Self {
        self.tracer = Some(tracer);
        self
    }

    pub fn with_override_intent(mut self, intent: Intent) -> Self {
        self.override_intent = Some(intent);
        self
    }
}

/// Result of classifying a message
#[derive(Debug)]
pub struct ClassificationResult {
    pub intent: Intent,
    pub confidence: f64,
    pub message: String,
}

/// Shared flow execution service used by both CLI and API
pub struct FlowExecutionService {
    runtime: Arc<RuntimeContext>,
    flow_selector: Arc<dyn FlowSelector>,
    intent_classifier: IntentClassifier,
}

impl FlowExecutionService {
    /// Create a new FlowExecutionService with default components
    pub fn new(runtime: Arc<RuntimeContext>) -> Result<Self> {
        let flow_selector = Arc::new(DefaultFlowSelector::with_default_dir());
        let intent_classifier = IntentClassifier::new()?;
        Ok(Self {
            runtime,
            flow_selector,
            intent_classifier,
        })
    }

    /// Create with custom flow selector
    pub fn with_flow_selector(
        runtime: Arc<RuntimeContext>,
        flow_selector: Arc<dyn FlowSelector>,
    ) -> Result<Self> {
        let intent_classifier = IntentClassifier::new()?;
        Ok(Self {
            runtime,
            flow_selector,
            intent_classifier,
        })
    }

    /// Classify a message's intent
    pub async fn classify(&self, message: &str, override_intent: Option<Intent>) -> Result<ClassificationResult> {
        let classification = self.intent_classifier
            .classify_with_override(message, override_intent)
            .await?;
        Ok(ClassificationResult {
            intent: classification.intent,
            confidence: classification.confidence as f64,
            message: message.to_string(),
        })
    }

    /// Select and load a flow file for the given intent
    pub fn load_flow(&self, intent: &Intent) -> Result<FlowFile> {
        let flow_path = self.flow_selector.select_flow(intent)?;

        // Log flow loaded event
        let flow_name = flow_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let flow_file = if flow_path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            let loader = YamlLoader::new();
            loader.load_from_path(&flow_path).context("Failed to load YAML flow")?
        } else {
            let loader = JsonLoader::new();
            loader.load_from_path(&flow_path).context("Failed to load JSON flow")?
        };

        Ok(flow_file)
    }

    /// Build a Flow from a FlowFile using a runtime-aware factory
    pub fn build_flow(&self, flow_file: &FlowFile, options: &ExecutionOptions) -> Result<Flow> {
        // Use runtime-aware factory so nodes get ModelRouter, ToolRuntime, MemoryService
        let factory = DefaultNodeFactory::from_runtime((*self.runtime).clone());

        let mut builder = Flow::builder();

        // Add nodes from flow file
        for node_def in &flow_file.nodes {
            let node = factory.create(&node_def.node_type, node_def.config.clone())?;
            builder = builder.add_node(node_def.id.clone(), node);
        }

        // Add transitions
        for trans in &flow_file.transitions {
            builder = builder.add_transition(trans.from.clone(), trans.action.clone(), trans.to.clone());
        }

        // Set start node
        builder = builder.start(flow_file.start_node.clone());

        // Attach tracer if provided
        if let Some(tracer) = &options.tracer {
            builder = builder.with_tracer(tracer.clone());
        }

        // Attach budget guard if provided
        if let Some(budget) = &options.budget {
            let guard = BudgetGuard::new(budget.clone());
            builder = builder.with_budget_guard(guard);
        }

        let flow = builder.build()?;
        Ok(flow)
    }

    /// Execute a flow end-to-end: classify → select → load → build → run → produce FinalOutput
    pub async fn execute_message(
        &self,
        message: &str,
        options: ExecutionOptions,
    ) -> Result<FinalOutput> {
        let start = Instant::now();

        // 1. Classify intent
        let classification = self.classify(message, options.override_intent.clone()).await?;
        let intent = classification.intent;

        // 2. Load flow
        let flow_file = self.load_flow(&intent)?;
        let flow_name = flow_file.name.clone();

        // 3. Build flow
        let mut flow = self.build_flow(&flow_file, &options)?;

        // 4. Prepare state
        let mut state = SharedState::new();
        state.set_input("message".to_string(), serde_json::json!(message));

        if let Some(ref mode) = options.personality_mode {
            state.set_personality_mode(mode);
        }

        // 5. Execute
        let result = flow.run(&mut state).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // 6. Produce FinalOutput
        match result {
            Ok(()) => {
                let run_id = state.get_run_id().unwrap_or_else(|| Tracer::generate_run_id());
                let primary = state.get_output("llm_response")
                    .or_else(|| state.get_output("generated"))
                    .or_else(|| state.get_output("review"))
                    .cloned()
                    .unwrap_or(serde_json::json!(null));

                let mut additional = std::collections::HashMap::new();
                for (key, value) in &state.output {
                    if key != "llm_response" && key != "generated" {
                        additional.insert(key.clone(), value.clone());
                    }
                }

                let final_output = FinalOutput::success(
                    run_id,
                    flow_name,
                    primary,
                    additional,
                    duration_ms,
                );

                Ok(final_output)
            }
            Err(e) => {
                let run_id = state.get_run_id().unwrap_or_else(|| Tracer::generate_run_id());
                Ok(FinalOutput::failure(run_id, flow_name, e.to_string(), duration_ms))
            }
        }
    }

    /// Execute a flow from a pre-loaded FlowFile (skip classification)
    pub async fn execute_flow_file(
        &self,
        flow_file: &FlowFile,
        message: &str,
        options: ExecutionOptions,
    ) -> Result<FinalOutput> {
        let start = Instant::now();
        let flow_name = flow_file.name.clone();

        let mut flow = self.build_flow(flow_file, &options)?;

        let mut state = SharedState::new();
        state.set_input("message".to_string(), serde_json::json!(message));

        if let Some(ref mode) = options.personality_mode {
            state.set_personality_mode(mode);
        }

        let result = flow.run(&mut state).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(()) => {
                let run_id = state.get_run_id().unwrap_or_else(|| Tracer::generate_run_id());
                let primary = state.get_output("llm_response")
                    .or_else(|| state.get_output("generated"))
                    .or_else(|| state.get_output("review"))
                    .cloned()
                    .unwrap_or(serde_json::json!(null));

                let mut additional = std::collections::HashMap::new();
                for (key, value) in &state.output {
                    if key != "llm_response" && key != "generated" {
                        additional.insert(key.clone(), value.clone());
                    }
                }

                Ok(FinalOutput::success(run_id, flow_name, primary, additional, duration_ms))
            }
            Err(e) => {
                let run_id = state.get_run_id().unwrap_or_else(|| Tracer::generate_run_id());
                Ok(FinalOutput::failure(run_id, flow_name, e.to_string(), duration_ms))
            }
        }
    }

    /// Produce an Evaluation from a completed execution's state
    pub fn evaluate(state: &SharedState, flow_name: &str, duration_ms: u64) -> Evaluation {
        let run_id = state.get_run_id().unwrap_or_default();

        // Count nodes from working/output keys as a rough proxy
        let nodes_executed = state.working.len() as u32 + state.output.len() as u32;

        Evaluation::new(
            run_id,
            flow_name.to_string(),
            nodes_executed,
            0, // nodes_failed - not tracked in state currently
            0, // transitions_taken - not tracked in state currently
            duration_ms,
        )
    }

    /// Get a reference to the runtime context
    pub fn runtime(&self) -> &Arc<RuntimeContext> {
        &self.runtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_options_default() {
        let opts = ExecutionOptions::default();
        assert!(opts.personality_mode.is_none());
        assert!(opts.budget.is_none());
        assert!(opts.tracer.is_none());
        assert!(opts.override_intent.is_none());
    }

    #[test]
    fn test_execution_options_builder() {
        let opts = ExecutionOptions::default()
            .with_personality_mode("engineer".to_string())
            .with_budget(ExecutionBudget::with_steps(10));

        assert_eq!(opts.personality_mode, Some("engineer".to_string()));
        assert!(opts.budget.is_some());
    }
}
