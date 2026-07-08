//! Advanced Flow Types - branching, looping, batch, and parallel execution

use anyhow::Result;
use futures::future::join_all;

use crate::flow::{Action, Flow, Node, NodeConfig, SharedState};

/// Conditional node that branches based on state conditions
pub struct ConditionalNode {
    id: String,
    condition: Box<dyn Fn(&SharedState) -> bool + Send + Sync>,
    true_action: Action,
    false_action: Action,
}

impl ConditionalNode {
    pub fn new(
        id: String,
        condition: Box<dyn Fn(&SharedState) -> bool + Send + Sync>,
        true_action: Action,
        false_action: Action,
    ) -> Self {
        Self {
            id,
            condition,
            true_action,
            false_action,
        }
    }
}

#[async_trait::async_trait]
impl Node for ConditionalNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "conditional"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    fn post(&self, state: &mut SharedState, _output: serde_json::Value) -> Action {
        if (self.condition)(state) {
            self.true_action.clone()
        } else {
            self.false_action.clone()
        }
    }

    fn config(&self) -> crate::flow::NodeConfig {
        crate::flow::NodeConfig::default()
    }
}

/// Looping node with iteration limit
pub struct LoopNode {
    id: String,
    max_iterations: u32,
    iteration_key: String,
}

impl LoopNode {
    pub fn new(id: String, max_iterations: u32) -> Self {
        let iteration_key = format!("{}_iteration_count", id);
        Self {
            id,
            max_iterations,
            iteration_key,
        }
    }

    fn get_iteration_count(&self, state: &SharedState) -> u32 {
        state
            .get_meta(&self.iteration_key)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    }

    fn increment_iteration(&self, state: &mut SharedState) {
        let count = self.get_iteration_count(state) + 1;
        state.set_meta(self.iteration_key.clone(), serde_json::json!(count));
    }
}

#[async_trait::async_trait]
impl Node for LoopNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "loop"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    fn post(&self, state: &mut SharedState, _output: serde_json::Value) -> Action {
        let count = self.get_iteration_count(state);

        if count >= self.max_iterations {
            "break".to_string()
        } else {
            self.increment_iteration(state);
            "continue".to_string()
        }
    }

    fn config(&self) -> crate::flow::NodeConfig {
        crate::flow::NodeConfig::default()
    }
}

/// Batch flow for processing multiple items through the same node
pub struct BatchFlow {
    flow: Flow,
    input_key: String,
    output_key: String,
}

impl BatchFlow {
    pub fn new(flow: Flow, input_key: String, output_key: String) -> Self {
        Self {
            flow,
            input_key,
            output_key,
        }
    }

    pub async fn run_batch(&self, items: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>> {
        let mut results = Vec::new();

        for (index, item) in items.into_iter().enumerate() {
            let mut state = SharedState::new();
            state.set_input(format!("{}_item_{}", self.input_key, index), item);

            let mut flow = self.flow.clone();
            flow.run(&mut state).await?;
            results.push(
                state
                    .get_output(&self.output_key)
                    .cloned()
                    .unwrap_or(serde_json::json!(null)),
            );
        }

        Ok(results)
    }

    pub async fn run_batch_with_progress<F>(
        &self,
        items: Vec<serde_json::Value>,
        mut progress_callback: F,
    ) -> Result<Vec<serde_json::Value>>
    where
        F: FnMut(usize, usize, &serde_json::Value),
    {
        let total = items.len();
        let mut results = Vec::new();

        for (index, item) in items.into_iter().enumerate() {
            let mut state = SharedState::new();
            state.set_input(format!("{}_item_{}", self.input_key, index), item);

            let mut flow = self.flow.clone();
            flow.run(&mut state).await?;

            let result = state
                .get_output(&self.output_key)
                .cloned()
                .unwrap_or(serde_json::json!(null));
            progress_callback(index + 1, total, &result);
            results.push(result);
        }

        Ok(results)
    }
}

/// Parallel node that executes multiple flows concurrently
pub struct ParallelNode {
    id: String,
    config: NodeConfig,
    flows: Vec<Flow>,
    concurrency_limit: usize,
}

impl ParallelNode {
    pub fn new(id: String, flows: Vec<Flow>, concurrency_limit: usize) -> Self {
        Self {
            id,
            config: NodeConfig::default(),
            flows,
            concurrency_limit,
        }
    }
}

#[async_trait::async_trait]
impl Node for ParallelNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "parallel"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        // Execute flows in parallel with concurrency limit
        for chunk in self.flows.chunks(self.concurrency_limit) {
            let futures: Vec<_> = chunk
                .iter()
                .map(|flow| {
                    let mut flow = flow.clone();
                    async move {
                        let mut state = SharedState::new();
                        flow.run(&mut state).await?;
                        Ok::<_, anyhow::Error>(state)
                    }
                })
                .collect();

            let chunk_results = join_all(futures).await;
            for result in chunk_results {
                let state = result?;
                results.push(state.get_all_outputs());
            }
        }

        Ok(serde_json::json!({ "results": results }))
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> Action {
        if let Some(results) = output["results"].as_array() {
            state.set_output("parallel_results".to_string(), serde_json::json!(results));
        }
        "continue".to_string()
    }

    fn config(&self) -> crate::flow::NodeConfig {
        self.config.clone()
    }
}

/// Reflection node that evaluates output and decides whether to continue looping
pub struct ReflectionNode {
    id: String,
    config: NodeConfig,
    reflection_fn: Box<dyn Fn(&SharedState) -> bool + Send + Sync>,
    max_iterations: u32,
    iteration_key: String,
}

impl ReflectionNode {
    pub fn new(
        id: String,
        reflection_fn: Box<dyn Fn(&SharedState) -> bool + Send + Sync>,
        max_iterations: u32,
    ) -> Self {
        let iteration_key = format!("{}_iteration_count", id);
        Self {
            id,
            config: NodeConfig::default(),
            reflection_fn,
            max_iterations,
            iteration_key,
        }
    }

    fn get_iteration_count(&self, state: &SharedState) -> u32 {
        state
            .get_meta(&self.iteration_key)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    }

    fn increment_iteration(&self, state: &mut SharedState) {
        let count = self.get_iteration_count(state) + 1;
        state.set_meta(self.iteration_key.clone(), serde_json::json!(count));
    }
}

#[async_trait::async_trait]
impl Node for ReflectionNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "reflection"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    fn post(&self, state: &mut SharedState, _output: serde_json::Value) -> Action {
        let count = self.get_iteration_count(state);

        // Check iteration limit
        if count >= self.max_iterations {
            return "break".to_string();
        }

        // Apply reflection function to decide whether to continue
        if (self.reflection_fn)(state) {
            self.increment_iteration(state);
            "continue".to_string()
        } else {
            "break".to_string()
        }
    }

    fn config(&self) -> crate::flow::NodeConfig {
        self.config.clone()
    }
}
