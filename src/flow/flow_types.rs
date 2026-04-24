//! Advanced Flow Types - branching, looping, batch, and parallel execution

use anyhow::Result;
use async_trait::async_trait;
use futures::future::join_all;

use crate::flow::{Node, NodeConfig, Action, SharedState, Flow};

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
            results.push(state.get_output(&self.output_key).cloned().unwrap_or(serde_json::json!(null)));
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
            
            let result = state.get_output(&self.output_key).cloned().unwrap_or(serde_json::json!(null));
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

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        let mut results = Vec::new();
        
        // Execute flows in parallel with concurrency limit
        for chunk in self.flows.chunks(self.concurrency_limit) {
            let futures: Vec<_> = chunk.iter().map(|flow| {
                let mut flow = flow.clone();
                async move {
                    let mut state = SharedState::new();
                    flow.run(&mut state).await?;
                    Ok::<_, anyhow::Error>(state)
                }
            }).collect();
            
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowBuilder, Node, NodeConfig, Input, Output};
    use async_trait::async_trait;
    use std::sync::Arc;

    // Simple node for testing
    struct TestNode {
        id: String,
        config: NodeConfig,
    }

    impl TestNode {
        fn new(id: String) -> Self {
            Self {
                id,
                config: NodeConfig::default(),
            }
        }
    }

    #[async_trait]
    impl Node for TestNode {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn prep(&self, state: &SharedState) -> Result<Input> {
            // Get the batch item if it exists
            if let Some(item) = state.get_input("batch_input_item_0") {
                Ok(serde_json::json!({ "item": item }))
            } else if let Some(item) = state.get_input("batch_input_item_1") {
                Ok(serde_json::json!({ "item": item }))
            } else if let Some(item) = state.get_input("batch_input_item_2") {
                Ok(serde_json::json!({ "item": item }))
            } else {
                Ok(serde_json::json!({}))
            }
        }

        async fn exec(&self, input: Input) -> Result<Output> {
            Ok(input)
        }

        fn post(&self, state: &mut SharedState, output: Output) -> Action {
            // Set the output for batch collection
            state.set_output("batch_output".to_string(), output);
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            self.config.clone()
        }
    }

    #[tokio::test]
    async fn test_conditional_node() {
        let condition = Box::new(|state: &SharedState| {
            state.get_input("test").and_then(|v| v.as_bool()).unwrap_or(false)
        });

        let node = ConditionalNode::new(
            "cond".to_string(),
            condition,
            "true_branch".to_string(),
            "false_branch".to_string(),
        );

        let mut state = SharedState::new();
        state.set_input("test".to_string(), serde_json::json!(true));

        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "true_branch");

        state.set_input("test".to_string(), serde_json::json!(false));
        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "false_branch");
    }

    #[tokio::test]
    async fn test_loop_node() {
        let node = LoopNode::new("loop_node".to_string(), 3);

        let mut state = SharedState::new();
        
        // First 3 iterations should continue
        for i in 0..3 {
            let action = node.post(&mut state, serde_json::json!({}));
            assert_eq!(action, "continue");
            assert_eq!(node.get_iteration_count(&state), i + 1);
        }

        // 4th iteration should break
        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "break");
    }

    #[tokio::test]
    async fn test_batch_flow() {
        let node = TestNode::new("process".to_string());
        let base_flow = FlowBuilder::new()
            .start("process".to_string())
            .add_node("process".to_string(), Arc::new(node))
            .build()
            .unwrap();

        let batch = BatchFlow::new(base_flow, "batch_input".to_string(), "batch_output".to_string());
        
        let items = vec![
            serde_json::json!("item1"),
            serde_json::json!("item2"),
            serde_json::json!("item3"),
        ];

        let results = batch.run_batch(items).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_parallel_node() {
        let node1 = TestNode::new("flow1".to_string());
        let node2 = TestNode::new("flow2".to_string());
        let node3 = TestNode::new("flow3".to_string());

        let flow1 = FlowBuilder::new()
            .start("flow1".to_string())
            .add_node("flow1".to_string(), Arc::new(node1))
            .build()
            .unwrap();

        let flow2 = FlowBuilder::new()
            .start("flow2".to_string())
            .add_node("flow2".to_string(), Arc::new(node2))
            .build()
            .unwrap();

        let flow3 = FlowBuilder::new()
            .start("flow3".to_string())
            .add_node("flow3".to_string(), Arc::new(node3))
            .build()
            .unwrap();

        let parallel = ParallelNode::new("parallel".to_string(), vec![flow1, flow2, flow3], 2);

        let mut state = SharedState::new();
        let input = parallel.prep(&state).unwrap();
        let output = parallel.exec(input).await.unwrap();
        let action = parallel.post(&mut state, output);

        assert_eq!(action, "continue");
        assert!(state.get_output("parallel_results").is_some());
    }

    #[tokio::test]
    async fn test_reflection_node() {
        let reflection_fn = Box::new(|state: &SharedState| {
            state.get_output("quality_score")
                .and_then(|v| v.as_f64())
                .map(|score| score < 0.8)
                .unwrap_or(false)
        });

        let node = ReflectionNode::new("reflection".to_string(), reflection_fn, 5);

        let mut state = SharedState::new();
        
        // Low quality score - should continue
        state.set_output("quality_score".to_string(), serde_json::json!(0.5));
        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "continue");
        assert_eq!(node.get_iteration_count(&state), 1);

        // High quality score - should break
        state.set_output("quality_score".to_string(), serde_json::json!(0.9));
        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "break");

        // Test iteration limit
        state.set_output("quality_score".to_string(), serde_json::json!(0.5));
        for _ in 0..5 {
            node.post(&mut state, serde_json::json!({}));
        }
        let action = node.post(&mut state, serde_json::json!({}));
        assert_eq!(action, "break");
    }

    #[tokio::test]
    async fn test_batch_flow_with_progress() {
        let node = TestNode::new("process".to_string());
        let base_flow = FlowBuilder::new()
            .start("process".to_string())
            .add_node("process".to_string(), Arc::new(node))
            .build()
            .unwrap();

        let batch = BatchFlow::new(base_flow, "batch_input".to_string(), "batch_output".to_string());
        
        let items = vec![
            serde_json::json!("item1"),
            serde_json::json!("item2"),
            serde_json::json!("item3"),
        ];

        let mut progress_calls = Vec::new();
        let results = batch.run_batch_with_progress(items, |current, total, result| {
            progress_calls.push((current, total, result.clone()));
        }).await.unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(progress_calls.len(), 3);
        assert_eq!(progress_calls[0].0, 1);
        assert_eq!(progress_calls[0].1, 3);
        assert_eq!(progress_calls[2].0, 3);
    }
}
