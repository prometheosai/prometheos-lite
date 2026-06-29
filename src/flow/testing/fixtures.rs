//! Test fixtures for flow testing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Test fixture for flow input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFixture {
    /// Input data for the flow
    pub input: serde_json::Value,
    /// Expected output data
    pub expected_output: Option<serde_json::Value>,
    /// Expected event kinds to be emitted
    pub expected_events: Vec<String>,
}

impl TestFixture {
    /// Create a new test fixture
    pub fn new(input: serde_json::Value) -> Self {
        Self {
            input,
            expected_output: None,
            expected_events: Vec::new(),
        }
    }

    /// Set expected output
    pub fn with_expected_output(mut self, output: serde_json::Value) -> Self {
        self.expected_output = Some(output);
        self
    }

    /// Set expected events
    pub fn with_expected_events(mut self, events: Vec<String>) -> Self {
        self.expected_events = events;
        self
    }

    /// Load fixture from JSON file
    pub fn from_json(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let fixture: Self = serde_json::from_str(&content)?;
        Ok(fixture)
    }

    /// Save fixture to JSON file
    pub fn to_json(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Test expectation for flow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestExpectation {
    /// Expected output keys and values
    pub outputs: HashMap<String, serde_json::Value>,
    /// Expected node execution order
    pub node_order: Vec<String>,
    /// Minimum number of steps
    pub min_steps: Option<usize>,
    /// Maximum number of steps
    pub max_steps: Option<usize>,
}

impl TestExpectation {
    /// Create a new test expectation
    pub fn new() -> Self {
        Self {
            outputs: HashMap::new(),
            node_order: Vec::new(),
            min_steps: None,
            max_steps: None,
        }
    }

    /// Add expected output
    pub fn with_output(mut self, key: String, value: serde_json::Value) -> Self {
        self.outputs.insert(key, value);
        self
    }

    /// Set expected node order
    pub fn with_node_order(mut self, order: Vec<String>) -> Self {
        self.node_order = order;
        self
    }

    /// Set step bounds
    pub fn with_step_bounds(mut self, min: Option<usize>, max: Option<usize>) -> Self {
        self.min_steps = min;
        self.max_steps = max;
        self
    }
}

impl Default for TestExpectation {
    fn default() -> Self {
        Self::new()
    }
}
