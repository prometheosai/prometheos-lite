//! Flow Execution Service - shared execution path for CLI and API
//!
//! This service consolidates flow execution logic that was previously
//! duplicated between the API handler and CLI runner. Both paths now
//! call through this single service.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use crate::flow::budget::{BudgetGuard, ExecutionBudget};
use crate::flow::execution::{ContinuationEngine, Flow, RunDb};
use crate::flow::factory::{DefaultNodeFactory, NodeFactory};
use crate::flow::loader::{FlowFile, FlowLoader, JsonLoader, YamlLoader};
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
    /// Optional work context ID for context tracking
    pub work_context_id: Option<String>,
    /// Optional strict mode enforcer
    pub strict_mode: Option<crate::flow::StrictModeEnforcer>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            personality_mode: None,
            budget: None,
            tracer: None,
            override_intent: None,
            flows_dir: None,
            work_context_id: None,
            strict_mode: None,
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

    pub fn with_work_context_id(mut self, id: String) -> Self {
        self.work_context_id = Some(id);
        self
    }

    pub fn with_strict_mode(mut self, enforcer: crate::flow::StrictModeEnforcer) -> Self {
        self.strict_mode = Some(enforcer);
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
    run_db: Option<Arc<Mutex<RunDb>>>,
    continuation_engine: Option<Arc<Mutex<ContinuationEngine>>>,
}

impl FlowExecutionService {
    /// Create a new FlowExecutionService with default components
    pub fn new(runtime: Arc<RuntimeContext>) -> Result<Self> {
        let flow_selector = Arc::new(DefaultFlowSelector::with_default_dir());
        let intent_classifier = IntentClassifier::new()?;

        // Initialize RunDb and ContinuationEngine with default paths
        let db_path = PathBuf::from(".prometheos/runs.db");
        let run_db = RunDb::new(db_path).ok().map(|db| Arc::new(Mutex::new(db)));
        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = Some(Arc::new(Mutex::new(ContinuationEngine::new(
            checkpoint_dir,
        ))));

        Ok(Self {
            runtime,
            flow_selector,
            intent_classifier,
            run_db,
            continuation_engine,
        })
    }

    /// Create with custom flow selector
    pub fn with_flow_selector(
        runtime: Arc<RuntimeContext>,
        flow_selector: Arc<dyn FlowSelector>,
    ) -> Result<Self> {
        let intent_classifier = IntentClassifier::new()?;

        // Initialize RunDb and ContinuationEngine with default paths
        let db_path = PathBuf::from(".prometheos/runs.db");
        let run_db = RunDb::new(db_path).ok().map(|db| Arc::new(Mutex::new(db)));
        let checkpoint_dir = PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = Some(Arc::new(Mutex::new(ContinuationEngine::new(
            checkpoint_dir,
        ))));

        Ok(Self {
            runtime,
            flow_selector,
            intent_classifier,
            run_db,
            continuation_engine,
        })
    }

    /// Classify a message's intent
    pub async fn classify(
        &self,
        message: &str,
        override_intent: Option<Intent>,
    ) -> Result<ClassificationResult> {
        let classification = self
            .intent_classifier
            .classify_with_override(message, override_intent)
            .await?;
        Ok(ClassificationResult {
            intent: classification.intent,
            confidence: classification.confidence as f64,
            message: message.to_string(),
        })
    }

    /// Select and load a flow file for the given intent
    pub fn load_flow(&self, intent: &Intent, tracer: Option<&SharedTracer>) -> Result<FlowFile> {
        let flow_path = self.flow_selector.select_flow(intent)?;

        // Log flow loaded event
        let flow_name = flow_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if let Some(tracer) = tracer {
            if let Ok(mut t) = tracer.lock() {
                t.log_flow_event(
                    TraceEvent::FlowLoaded {
                        run_id: "".to_string(), // No run_id yet at load time
                        flow_name: flow_name.clone(),
                        path: flow_path.display().to_string(),
                    },
                    None,
                    format!("Loaded flow: {}", flow_name),
                );
            }
        }

        let flow_file = if flow_path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            let loader = YamlLoader::new();
            loader
                .load_from_path(&flow_path)
                .context("Failed to load YAML flow")?
        } else {
            let loader = JsonLoader::new();
            loader
                .load_from_path(&flow_path)
                .context("Failed to load JSON flow")?
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
            builder =
                builder.add_transition(trans.from.clone(), trans.action.clone(), trans.to.clone());
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

        // 1. Generate run_id and trace_id upfront (explicit, not magical)
        let run_id = crate::flow::tracing::Tracer::generate_run_id();
        let trace_id = crate::flow::tracing::Tracer::generate_trace_id();

        // 2. Classify intent
        let classification = self
            .classify(message, options.override_intent.clone())
            .await?;
        let intent = classification.intent;

        // 3. Load flow
        let flow_file = self.load_flow(&intent, options.tracer.as_ref())?;
        let flow_name = flow_file.name.clone();

        // 4. Build flow
        let mut flow = self.build_flow(&flow_file, &options)?;

        // 5. Prepare state with IDs pre-set
        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!(message));
        state.set_run_id(&run_id);
        state.set_trace_id(&trace_id);

        if let Some(ref mode) = options.personality_mode {
            state.set_personality_mode(mode);
        }

        // Set budget guard in state for boundary-level enforcement
        if let Some(budget) = &options.budget {
            let guard = Arc::new(Mutex::new(BudgetGuard::new(budget.clone())));
            state.set_budget_guard(guard);
        }

        // Set strict mode enforcer in state for runtime enforcement
        if let Some(ref strict_mode) = options.strict_mode {
            state.set_strict_mode_enforcer(Arc::new(strict_mode.clone()));

            // V1.5: Enforce no silent failures - validate input
            strict_mode.validate_input(&serde_json::json!(message), "task")?;
        }

        // 7. Execute
        let result = flow.run(&mut state).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // 8. Extract budget report from state
        let budget_report = state.get_budget_report().cloned();

        // 9. Count trace events from tracer if available
        let events_count = if let Some(tracer) = &options.tracer {
            if let Ok(t) = tracer.lock() {
                t.get_logs().len()
            } else {
                0
            }
        } else {
            0
        };

        // 9. Produce evaluation
        let evaluation = Self::evaluate(
            &state,
            &flow_name,
            duration_ms,
            options.tracer.as_ref(),
            &run_id,
        );

        // 10. Emit OutputGenerated and EvaluationCompleted events
        if let Some(tracer) = &options.tracer {
            if let Ok(mut t) = tracer.lock() {
                t.log_flow_event(
                    TraceEvent::OutputGenerated {
                        run_id: run_id.clone(),
                        trace_id: trace_id.clone(),
                        output_key: "primary".to_string(),
                    },
                    None,
                    format!("FinalOutput generated for {}", flow_name),
                );

                t.log_flow_event(
                    TraceEvent::EvaluationCompleted {
                        run_id: run_id.clone(),
                        trace_id: trace_id.clone(),
                        score: Some(evaluation.success_rate()),
                    },
                    None,
                    format!(
                        "Evaluation completed: {:.2} success rate",
                        evaluation.success_rate()
                    ),
                );
            }
        }

        // 11. Extract execution metadata from state
        let execution_metadata: std::collections::HashMap<String, serde_json::Value> =
            state.get_execution_metadata().into_iter().collect();

        // 12. Save checkpoint if continuation engine is available
        if let Some(ref continuation_engine) = self.continuation_engine {
            if let Ok(engine) = continuation_engine.lock() {
                let _ = engine.save_checkpoint(&run_id, &state);
            }
        }

        // 13. Persist run to database if RunDb is available
        if let Some(ref run_db) = self.run_db {
            if let Ok(db) = run_db.lock() {
                use crate::flow::execution::{FlowRun, RunStatus};
                let mut flow_run = FlowRun::new(flow_name.clone());
                flow_run.id = run_id.clone();
                match result {
                    Ok(()) => {
                        flow_run.mark_completed(state.clone());
                    }
                    Err(_) => {
                        flow_run.mark_failed("Execution failed".to_string());
                    }
                }
                let _ = db.save_run(&flow_run);
            }
        }

        // 14. Produce FinalOutput
        let final_output = match result {
            Ok(()) => {
                let primary = state
                    .get_output("llm_response")
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

                let mut output = FinalOutput::success(
                    run_id,
                    trace_id,
                    flow_name,
                    primary,
                    additional,
                    evaluation,
                    budget_report,
                    events_count,
                    duration_ms,
                );
                output.execution_metadata = execution_metadata;
                output
            }
            Err(e) => {
                let mut output =
                    FinalOutput::failure(run_id, trace_id, flow_name, e.to_string(), duration_ms);
                output.execution_metadata = execution_metadata;
                output
            }
        };

        // 15. Validate output if strict mode is enabled
        if let Some(ref strict_mode) = options.strict_mode {
            if final_output.success {
                strict_mode.validate_output(&final_output.primary, "primary_output")?;
            }
        }

        Ok(final_output)
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

        // Generate run_id and trace_id upfront
        let run_id = crate::flow::tracing::Tracer::generate_run_id();
        let trace_id = crate::flow::tracing::Tracer::generate_trace_id();

        let mut flow = self.build_flow(flow_file, &options)?;

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!(message));
        state.set_run_id(&run_id);
        state.set_trace_id(&trace_id);

        if let Some(ref mode) = options.personality_mode {
            state.set_personality_mode(mode);
        }

        // Set budget guard in state for boundary-level enforcement
        if let Some(budget) = &options.budget {
            let guard = Arc::new(Mutex::new(BudgetGuard::new(budget.clone())));
            state.set_budget_guard(guard);
        }

        let result = flow.run(&mut state).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Extract budget report, count events, produce evaluation
        let budget_report = state.get_budget_report().cloned();
        let events_count = if let Some(tracer) = &options.tracer {
            if let Ok(t) = tracer.lock() {
                t.get_logs().len()
            } else {
                0
            }
        } else {
            0
        };
        let evaluation = Self::evaluate(
            &state,
            &flow_name,
            duration_ms,
            options.tracer.as_ref(),
            &run_id,
        );

        // Emit OutputGenerated and EvaluationCompleted events
        if let Some(tracer) = &options.tracer {
            if let Ok(mut t) = tracer.lock() {
                t.log_flow_event(
                    TraceEvent::OutputGenerated {
                        run_id: run_id.clone(),
                        trace_id: trace_id.clone(),
                        output_key: "primary".to_string(),
                    },
                    None,
                    format!("FinalOutput generated for {}", flow_name),
                );

                t.log_flow_event(
                    TraceEvent::EvaluationCompleted {
                        run_id: run_id.clone(),
                        trace_id: trace_id.clone(),
                        score: Some(evaluation.success_rate()),
                    },
                    None,
                    format!(
                        "Evaluation completed: {:.2} success rate",
                        evaluation.success_rate()
                    ),
                );
            }
        }

        // Extract execution metadata from state
        let execution_metadata: std::collections::HashMap<String, serde_json::Value> =
            state.get_execution_metadata().into_iter().collect();

        // Save checkpoint if continuation engine is available
        if let Some(ref continuation_engine) = self.continuation_engine {
            if let Ok(engine) = continuation_engine.lock() {
                let _ = engine.save_checkpoint(&run_id, &state);
            }
        }

        // Persist run to database if RunDb is available
        if let Some(ref run_db) = self.run_db {
            if let Ok(db) = run_db.lock() {
                use crate::flow::execution::{FlowRun, RunStatus};
                let mut flow_run = FlowRun::new(flow_name.clone());
                flow_run.id = run_id.clone();
                match result {
                    Ok(()) => {
                        flow_run.mark_completed(state.clone());
                    }
                    Err(_) => {
                        flow_run.mark_failed("Execution failed".to_string());
                    }
                }
                let _ = db.save_run(&flow_run);
            }
        }

        match result {
            Ok(()) => {
                let primary = state
                    .get_output("llm_response")
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

                let mut output = FinalOutput::success(
                    run_id,
                    trace_id,
                    flow_name,
                    primary,
                    additional,
                    evaluation,
                    budget_report,
                    events_count,
                    duration_ms,
                );
                output.execution_metadata = execution_metadata;
                Ok(output)
            }
            Err(e) => {
                let mut output =
                    FinalOutput::failure(run_id, trace_id, flow_name, e.to_string(), duration_ms);
                output.execution_metadata = execution_metadata;
                Ok(output)
            }
        }
    }

    /// Produce an Evaluation from a completed execution's state
    pub fn evaluate(
        state: &SharedState,
        flow_name: &str,
        duration_ms: u64,
        tracer: Option<&SharedTracer>,
        run_id: &str,
    ) -> Evaluation {
        let run_id_str = state.get_run_id().unwrap_or_else(|| run_id.to_string());

        // Derive metrics from trace events if available, otherwise fall back to state size
        let (nodes_executed, nodes_failed, transitions_taken) = if let Some(tracer) = tracer {
            if let Ok(t) = tracer.lock() {
                let logs = t.get_logs();
                let nodes_executed = logs
                    .iter()
                    .filter(|entry| matches!(entry.event_type, TraceEvent::NodeCompleted { .. }))
                    .count() as u32;
                let nodes_failed = logs
                    .iter()
                    .filter(|entry| matches!(entry.event_type, TraceEvent::NodeFailed { .. }))
                    .count() as u32;
                let transitions_taken = logs
                    .iter()
                    .filter(|entry| matches!(entry.event_type, TraceEvent::TransitionTaken { .. }))
                    .count() as u32;
                (nodes_executed, nodes_failed, transitions_taken)
            } else {
                // Fallback to state size
                let nodes_executed = state.working.len() as u32 + state.output.len() as u32;
                let nodes_failed = 0;
                let transitions_taken = nodes_executed.saturating_sub(1);
                (nodes_executed, nodes_failed, transitions_taken)
            }
        } else {
            // Fallback to state size
            let nodes_executed = state.working.len() as u32 + state.output.len() as u32;
            let nodes_failed = 0;
            let transitions_taken = nodes_executed.saturating_sub(1);
            (nodes_executed, nodes_failed, transitions_taken)
        };

        Evaluation::new(
            run_id_str,
            flow_name.to_string(),
            nodes_executed,
            nodes_failed,
            transitions_taken,
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
