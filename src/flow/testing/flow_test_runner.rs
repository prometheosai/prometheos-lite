//! Flow test runner for deterministic flow testing

use super::fixtures::{TestExpectation, TestFixture};
use crate::flow::loader::{FlowFile, FlowLoader, JsonLoader, YamlLoader};
use crate::flow::{
    DefaultNodeFactory, Flow, FlowBuilder, NodeFactory, SharedState, TraceMetrics, Tracer,
};
use anyhow::Result;
use std::path::PathBuf;

/// Flow test runner for deterministic testing
pub struct FlowTestRunner {
    /// Path to the flow file
    flow_path: PathBuf,
    /// Scripted node responses (node_id -> response)
    scripted_responses: std::collections::HashMap<String, String>,
    /// Tracer for capturing events
    tracer: Option<std::sync::Arc<std::sync::Mutex<Tracer>>>,
}

impl FlowTestRunner {
    /// Create a new flow test runner
    pub fn new(flow_path: PathBuf) -> Self {
        Self {
            flow_path,
            scripted_responses: std::collections::HashMap::new(),
            tracer: None,
        }
    }

    /// Add a deterministic scripted response for a specific node.
    pub fn with_scripted_response(mut self, node_id: String, response: String) -> Self {
        self.scripted_responses.insert(node_id, response);
        self
    }

    /// Backward-compatible alias for scripted responses.
    pub fn with_mock_response(self, node_id: String, response: String) -> Self {
        self.with_scripted_response(node_id, response)
    }

    /// Enable tracing
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(std::sync::Arc::new(std::sync::Mutex::new(Tracer::new())));
        self
    }

    /// Load the flow file
    fn load_flow(&self) -> Result<FlowFile> {
        let extension = self
            .flow_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow::anyhow!("No file extension"))?;

        let flow_file = match extension {
            "yaml" | "yml" => {
                let loader = YamlLoader::new();
                loader.load_from_path(&self.flow_path)?
            }
            "json" => {
                let loader = JsonLoader::new();
                loader.load_from_path(&self.flow_path)?
            }
            _ => anyhow::bail!("Unsupported file extension: {}", extension),
        };

        Ok(flow_file)
    }

    /// Build the flow from the loaded flow file
    fn build_flow(&self, flow_file: &FlowFile) -> Result<Flow> {
        let factory = DefaultNodeFactory::new();
        let mut builder = FlowBuilder::new();

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

        // Build the flow
        let flow = builder.build()?;

        Ok(flow)
    }

    /// Run a test with the given fixture
    pub async fn run_test(&self, fixture: &TestFixture) -> Result<TestResult> {
        let flow_file = self.load_flow()?;
        let mut flow = self.build_flow(&flow_file)?;

        // Create input state
        let mut state = SharedState::new();
        for (key, value) in fixture
            .input
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Input must be an object"))?
        {
            state.set_input(key.clone(), value.clone());
        }
        if !self.scripted_responses.is_empty() {
            let mut response_map = serde_json::Map::new();
            for (node_id, response) in &self.scripted_responses {
                response_map.insert(node_id.clone(), serde_json::Value::String(response.clone()));
            }
            state.set_input(
                "__scripted_responses".to_string(),
                serde_json::Value::Object(response_map),
            );
        }

        // Execute the flow
        let execution_result = flow.run(&mut state).await;

        let outputs = state.get_all_outputs();
        let (events, metrics) = if let Some(tracer) = &self.tracer {
            if let Ok(tracer) = tracer.lock() {
                let events = tracer.export_timeline()?;
                let metrics = tracer.get_metrics();
                (events, metrics)
            } else {
                (String::new(), TraceMetrics::default())
            }
        } else {
            (String::new(), TraceMetrics::default())
        };

        Ok(TestResult {
            success: execution_result.is_ok(),
            outputs,
            events,
            error: execution_result.err().map(|e| e.to_string()),
            metrics,
        })
    }

    /// Run a test with expectations
    pub async fn run_test_with_expectations(
        &self,
        fixture: &TestFixture,
        expectations: &TestExpectation,
    ) -> Result<TestResult> {
        let result = self.run_test(fixture).await?;

        // Validate outputs
        for (key, expected_value) in &expectations.outputs {
            if let Some(actual_value) = result.outputs.get(key) {
                if actual_value != expected_value {
                    anyhow::bail!(
                        "Output mismatch for key '{}': expected {:?}, got {:?}",
                        key,
                        expected_value,
                        actual_value
                    );
                }
            } else {
                anyhow::bail!("Missing output key: {}", key);
            }
        }

        // Validate step bounds if specified
        let event_count = if result.events.is_empty() {
            0
        } else {
            serde_json::from_str::<serde_json::Value>(&result.events)
                .ok()
                .and_then(|v| v.as_array().map(|arr| arr.len()))
                .unwrap_or(0)
        };

        if let Some(min_steps) = expectations.min_steps {
            if event_count < min_steps {
                anyhow::bail!("Too few steps: {} (minimum {})", event_count, min_steps);
            }
        }

        if let Some(max_steps) = expectations.max_steps {
            if event_count > max_steps {
                anyhow::bail!("Too many steps: {} (maximum {})", event_count, max_steps);
            }
        }

        Ok(result)
    }
}

/// Result of a flow test
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Whether the test passed
    pub success: bool,
    /// Output data
    pub outputs: serde_json::Value,
    /// Event timeline (JSON string)
    pub events: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Execution metrics (LLM calls, tool calls, etc.)
    pub metrics: TraceMetrics,
}
