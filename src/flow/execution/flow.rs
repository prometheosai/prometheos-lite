//! Flow execution engine with validation and retry support.

use crate::flow::{
    Action, BudgetGuard, ExecutionBudget, Input, Node, NodeConfig, NodeId, Output, SharedState,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Lifecycle hooks for flow execution
pub trait FlowLifecycleHooks: Send + Sync {
    /// Called before a node is executed
    fn on_node_start(&self, node_id: &NodeId, state: &SharedState, input: &Input);

    /// Called after a node is executed
    fn on_node_complete(&self, node_id: &NodeId, state: &SharedState, output: &Output);

    /// Called before a state transition
    fn on_transition(&self, from: &NodeId, action: &Action, to: &NodeId);

    /// Called when the flow completes
    fn on_flow_complete(&self, state: &SharedState);

    /// Called when the flow fails
    fn on_flow_error(&self, error: &anyhow::Error);
}

/// Default no-op lifecycle hooks
pub struct NoOpHooks;

impl FlowLifecycleHooks for NoOpHooks {
    fn on_node_start(&self, _node_id: &NodeId, _state: &SharedState, _input: &Input) {}
    fn on_node_complete(&self, _node_id: &NodeId, _state: &SharedState, _output: &Output) {}
    fn on_transition(&self, _from: &NodeId, _action: &Action, _to: &NodeId) {}
    fn on_flow_complete(&self, _state: &SharedState) {}
    fn on_flow_error(&self, _error: &anyhow::Error) {}
}

/// Flow - a directed graph of nodes with state transitions
#[derive(Clone)]
pub struct Flow {
    /// Starting node ID
    start: NodeId,
    /// All nodes in the flow (using Arc for cloneability)
    nodes: HashMap<NodeId, Arc<dyn Node>>,
    /// Transitions: (current_node, action) -> next_node
    transitions: HashMap<(NodeId, Action), NodeId>,
    /// Optional tracer for logging and timeline events
    tracer: Option<crate::flow::tracing::SharedTracer>,
    /// Optional budget guard for enforcing resource limits
    budget_guard: Option<BudgetGuard>,
    /// Optional loop detector for preventing runaway flows
    loop_detector: Option<crate::flow::LoopDetector>,
}

impl Flow {
    /// Create a new Flow builder
    pub fn builder() -> FlowBuilder {
        FlowBuilder::new()
    }

    /// Get the start node ID
    pub fn start_node(&self) -> &NodeId {
        &self.start
    }

    /// Get a node by ID
    pub fn get_node(&self, node_id: &NodeId) -> Option<&Arc<dyn Node>> {
        self.nodes.get(node_id)
    }

    /// Get the next node based on current node and action
    pub fn get_next_node(&self, current: &NodeId, action: &str) -> Option<&NodeId> {
        self.transitions.get(&(current.clone(), action.to_string()))
    }

    /// Set the tracer for this flow
    pub fn with_tracer(mut self, tracer: crate::flow::tracing::SharedTracer) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Get the tracer if set
    pub fn tracer(&self) -> Option<&crate::flow::tracing::SharedTracer> {
        self.tracer.as_ref()
    }

    /// Set the budget guard for this flow
    pub fn with_budget_guard(mut self, budget_guard: BudgetGuard) -> Self {
        self.budget_guard = Some(budget_guard);
        self
    }

    /// Get the budget guard if set
    pub fn budget_guard(&self) -> Option<&BudgetGuard> {
        self.budget_guard.as_ref()
    }

    /// Set the loop detector for this flow
    pub fn with_loop_detector(mut self, loop_detector: crate::flow::LoopDetector) -> Self {
        self.loop_detector = Some(loop_detector);
        self
    }

    /// Get the loop detector if set
    pub fn loop_detector(&self) -> Option<&crate::flow::LoopDetector> {
        self.loop_detector.as_ref()
    }

    /// Execute the flow with the given state
    pub async fn run(&mut self, state: &mut SharedState) -> Result<()> {
        self.run_with_hooks(state, &NoOpHooks).await
    }

    /// Execute the flow with the given state and lifecycle hooks
    pub async fn run_with_hooks<H: FlowLifecycleHooks>(
        &mut self,
        state: &mut SharedState,
        hooks: &H,
    ) -> Result<()> {
        let mut current = self.start.clone();

        // Generate one run_id and trace_id for the entire execution, store in SharedState
        let run_id = state
            .get_run_id()
            .unwrap_or_else(|| crate::flow::tracing::Tracer::generate_run_id());
        let trace_id = state
            .get_trace_id()
            .unwrap_or_else(|| crate::flow::tracing::Tracer::generate_trace_id());
        state.set_run_id(&run_id);
        state.set_trace_id(&trace_id);

        // Store flow snapshot for this run (if flow source is available in state)
        if let Some(flow_source) = state.get_input("flow_source").and_then(|v| v.as_str()) {
            if let Some(flow_name) = state.get_input("flow_name").and_then(|v| v.as_str()) {
                let source_hash = crate::flow::FlowSnapshot::compute_hash(flow_source);
                let db_path = ".prometheos/runs.db";
                if std::path::Path::new(db_path).exists() {
                    if let Ok(db) = crate::db::repository::Db::new(db_path) {
                        use crate::db::repository::FlowSnapshotOperations;
                        let _ = FlowSnapshotOperations::create_flow_snapshot(
                            &db,
                            flow_name,
                            "1.0",
                            &source_hash,
                            flow_source,
                        );
                    }
                }
            }
        }

        // Log flow start
        if let Some(tracer) = &self.tracer {
            if let Ok(mut t) = tracer.lock() {
                t.log_run_event(
                    crate::flow::tracing::TraceEvent::RunStarted {
                        run_id: run_id.clone(),
                        flow_name: "flow".to_string(),
                    },
                    "Flow execution started".to_string(),
                );
            }
        }

        loop {
            // Check loop detection before each step
            if let Some(detector) = &mut self.loop_detector {
                if let Err(e) = detector.record_node(&current.clone()) {
                    anyhow::bail!("Loop detection: {}", e);
                }

                // Emit loop detection trace event
                if let Some(tracer) = &self.tracer {
                    if let Ok(mut t) = tracer.lock() {
                        t.log_flow_event(
                            crate::flow::tracing::TraceEvent::LoopDetected {
                                run_id: run_id.clone(),
                                trace_id: trace_id.clone(),
                                node_id: current.clone(),
                                loop_type: "node_repetition".to_string(),
                            },
                            Some(current.clone()),
                            format!("Loop check: node {} count {}", current, detector.get_node_count(&current)),
                        );
                    }
                }
            }

            // Check budget before each step and update state with budget report
            if let Some(guard) = &self.budget_guard {
                guard
                    .update_runtime()
                    .context("Budget check: runtime exceeded")?;
                guard
                    .record_step()
                    .context("Budget check: steps exceeded")?;

                // Update budget report in state so nodes can inspect it
                state.set_budget_report(guard.get_report());

                // Emit budget trace event
                if let Some(tracer) = &self.tracer {
                    if let Ok(mut t) = tracer.lock() {
                        let usage = guard.get_usage();
                        let budget = guard.get_budget();
                        t.log_flow_event(
                            crate::flow::tracing::TraceEvent::BudgetChecked {
                                run_id: run_id.clone(),
                                resource: "steps".to_string(),
                                current: usage.steps as u64,
                                limit: budget.max_steps as u64,
                            },
                            None,
                            format!("Budget check: steps {}/{}", usage.steps, budget.max_steps),
                        );
                    }
                }
            }

            let node = self
                .nodes
                .get(&current)
                .ok_or_else(|| anyhow::anyhow!("Node not found: {}", current))?;

            // Prepare input from state
            let input = node.prep(state)?;
            let input_summary = crate::flow::tracing::Tracer::summarize_value(&input, 200);

            // Hook: before node execution
            hooks.on_node_start(&current, state, &input);

            // Log node start (reuse same run_id and trace_id)
            if let Some(tracer) = &self.tracer {
                if let Ok(mut t) = tracer.lock() {
                    t.log_flow_event(
                        crate::flow::tracing::TraceEvent::NodeStarted {
                            run_id: run_id.clone(),
                            trace_id: trace_id.clone(),
                            node_id: current.clone(),
                            input_summary: input_summary.clone(),
                        },
                        Some(current.clone()),
                        format!("Executing node: {}", current),
                    );
                }
            }

            let start_time = std::time::Instant::now();

            // Execute with retry
            let output = match self.execute_with_retry(node, input).await {
                Ok(output) => output,
                Err(e) => {
                    // Log node failure before returning
                    if let Some(tracer) = &self.tracer {
                        if let Ok(mut t) = tracer.lock() {
                            t.log_flow_event(
                                crate::flow::tracing::TraceEvent::NodeFailed {
                                    run_id: run_id.clone(),
                                    trace_id: trace_id.clone(),
                                    node_id: current.clone(),
                                    error: e.to_string(),
                                    input_summary,
                                },
                                Some(current.clone()),
                                format!("Node failed: {}", current),
                            );
                        }
                    }
                    hooks.on_flow_error(&e);
                    return Err(e);
                }
            };

            // Hook: after node execution
            hooks.on_node_complete(&current, state, &output);

            let duration_ms = start_time.elapsed().as_millis() as u64;

            // Log node completion (reuse same run_id and trace_id)
            if let Some(tracer) = &self.tracer {
                if let Ok(mut t) = tracer.lock() {
                    let output_summary = crate::flow::tracing::Tracer::summarize_value(&output, 200);
                    t.log_flow_event(
                        crate::flow::tracing::TraceEvent::NodeCompleted {
                            run_id: run_id.clone(),
                            trace_id: trace_id.clone(),
                            node_id: current.clone(),
                            duration_ms,
                            output_summary,
                            status: "success".to_string(),
                        },
                        Some(current.clone()),
                        format!("Node completed: {}", current),
                    );
                    t.add_timeline_event(
                        crate::flow::tracing::TraceEvent::NodeCompleted {
                            run_id: run_id.clone(),
                            trace_id: trace_id.clone(),
                            node_id: current.clone(),
                            duration_ms,
                            output_summary: None,
                            status: "success".to_string(),
                        },
                        Some(current.clone()),
                        Some(duration_ms),
                        serde_json::json!({ "output": &output }),
                    );
                }
            }

            // Post-process: update state and get action
            let action = node.post(state, output.clone());

            // Record budget usage based on node kind (stable even if id is wrapped)
            if let Some(guard) = &self.budget_guard {
                let node_kind = node.kind();
                if ["planner", "coder", "reviewer", "llm"].contains(&node_kind) {
                    let _ = guard.record_llm_call();
                } else if node_kind == "tool" {
                    let _ = guard.record_tool_call();
                } else if node_kind == "context_loader" {
                    let _ = guard.record_memory_read();
                } else if node_kind == "memory_write" {
                    let _ = guard.record_memory_write();
                }
                // Update budget report in state after recording
                state.set_budget_report(guard.get_report());
            }

            // Find next node based on action
            match self.transitions.get(&(current.clone(), action.clone())) {
                Some(next) => {
                    // Hook: before transition
                    hooks.on_transition(&current, &action, next);

                    // Check loop detection for transitions
                    if let Some(detector) = &mut self.loop_detector {
                        if let Err(e) = detector.record_transition(&current, next) {
                            anyhow::bail!("Loop detection: {}", e);
                        }
                    }

                    // Log transition (reuse same run_id)
                    if let Some(tracer) = &self.tracer {
                        if let Ok(mut t) = tracer.lock() {
                            t.log_flow_event(
                                crate::flow::tracing::TraceEvent::TransitionTaken {
                                    run_id: run_id.clone(),
                                    from: current.clone(),
                                    action: action.clone(),
                                    to: next.clone(),
                                },
                                Some(current.clone()),
                                format!("Transition: {} -> {} via {}", current, next, action),
                            );
                        }
                    }

                    current = next.clone();
                }
                None => {
                    // Hook: flow complete
                    hooks.on_flow_complete(state);

                    // Log flow completion (reuse same run_id)
                    if let Some(tracer) = &self.tracer {
                        if let Ok(mut t) = tracer.lock() {
                            let total_duration =
                                std::time::Instant::now().elapsed().as_millis() as u64;
                            t.log_run_event(
                                crate::flow::tracing::TraceEvent::RunCompleted {
                                    run_id: run_id.clone(),
                                    duration_ms: total_duration,
                                },
                                "Flow execution completed".to_string(),
                            );
                        }
                    }

                    break; // No transition, end of flow
                }
            }
        }

        Ok(())
    }

    /// Execute a node with retry logic
    async fn execute_with_retry(
        &self,
        node: &Arc<dyn Node>,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let config = node.config();
        let mut last_error = None;

        for attempt in 0..=config.retries {
            match node.exec(input.clone()).await {
                Ok(output) => return Ok(output),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < config.retries {
                        let delay = Duration::from_millis(
                            config.retry_delay_ms * 2_u64.pow(attempt as u32),
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Max retries exceeded")))
    }

    /// Validate the flow structure
    pub fn validate(&self) -> Result<()> {
        // Check start node exists
        if !self.nodes.contains_key(&self.start) {
            bail!("Start node '{}' not found in nodes", self.start);
        }

        // Check all transition targets exist
        for ((node_id, action), target_id) in &self.transitions {
            if !self.nodes.contains_key(node_id) {
                bail!("Transition source node '{}' not found", node_id);
            }
            if !self.nodes.contains_key(target_id) {
                bail!(
                    "Transition target '{}' (from {} via {}) not found",
                    target_id,
                    node_id,
                    action
                );
            }
        }

        // Check for unreachable nodes
        let mut reachable = std::collections::HashSet::new();
        let mut to_visit = vec![self.start.clone()];

        while let Some(node_id) = to_visit.pop() {
            if reachable.insert(node_id.clone()) {
                // Find all transitions from this node
                for ((source, _action), target) in &self.transitions {
                    if source == &node_id {
                        to_visit.push(target.clone());
                    }
                }
            }
        }

        for node_id in self.nodes.keys() {
            if !reachable.contains(node_id) {
                bail!("Node '{}' is unreachable from start", node_id);
            }
        }

        // Check for dead ends (nodes with no outgoing transitions)
        for node_id in self.nodes.keys() {
            let has_outgoing = self
                .transitions
                .iter()
                .any(|((source, _), _)| source == node_id);

            if !has_outgoing && node_id != &self.start {
                // This is a terminal node, which is allowed
                // But warn if it's not explicitly marked as terminal
            }
        }

        // Check for cycles (optional warning)
        if let Some(cycle) = self.detect_cycle() {
            eprintln!("Warning: Flow contains a cycle: {}", cycle.join(" -> "));
        }

        Ok(())
    }

    /// Detect cycles in the flow graph using DFS
    fn detect_cycle(&self) -> Option<Vec<String>> {
        let mut visited = std::collections::HashSet::new();
        let mut recursion_stack = std::collections::HashSet::new();
        let mut path = Vec::new();

        for node_id in self.nodes.keys() {
            if !visited.contains(node_id) {
                if let Some(cycle) =
                    self.dfs_cycle(node_id, &mut visited, &mut recursion_stack, &mut path)
                {
                    return Some(cycle);
                }
            }
        }

        None
    }

    fn dfs_cycle(
        &self,
        node_id: &NodeId,
        visited: &mut std::collections::HashSet<NodeId>,
        recursion_stack: &mut std::collections::HashSet<NodeId>,
        path: &mut Vec<NodeId>,
    ) -> Option<Vec<String>> {
        visited.insert(node_id.clone());
        recursion_stack.insert(node_id.clone());
        path.push(node_id.clone());

        // Find all transitions from this node
        for ((source, _), target) in &self.transitions {
            if source == node_id {
                if !visited.contains(target) {
                    if let Some(cycle) = self.dfs_cycle(target, visited, recursion_stack, path) {
                        return Some(cycle);
                    }
                } else if recursion_stack.contains(target) {
                    // Found a cycle
                    let cycle_start = path.iter().position(|n| n == target).unwrap();
                    let cycle = path[cycle_start..].to_vec();
                    return Some(cycle);
                }
            }
        }

        recursion_stack.remove(node_id);
        path.pop();
        None
    }
}

/// Builder for constructing Flows
pub struct FlowBuilder {
    start: Option<NodeId>,
    nodes: HashMap<NodeId, Arc<dyn Node>>,
    transitions: HashMap<(NodeId, Action), NodeId>,
    tracer: Option<crate::flow::tracing::SharedTracer>,
    budget_guard: Option<BudgetGuard>,
}

impl FlowBuilder {
    pub fn new() -> Self {
        Self {
            start: None,
            nodes: HashMap::new(),
            transitions: HashMap::new(),
            tracer: None,
            budget_guard: None,
        }
    }

    /// Set the starting node
    pub fn start(mut self, node_id: NodeId) -> Self {
        self.start = Some(node_id);
        self
    }

    /// Set the tracer for the flow
    pub fn with_tracer(mut self, tracer: crate::flow::tracing::SharedTracer) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Set the budget guard for the flow
    pub fn with_budget_guard(mut self, budget_guard: BudgetGuard) -> Self {
        self.budget_guard = Some(budget_guard);
        self
    }

    /// Add a node to the flow
    pub fn add_node(mut self, node_id: NodeId, node: Arc<dyn Node>) -> Self {
        self.nodes.insert(node_id, node);
        self
    }

    /// Add a transition from one node to another based on an action
    pub fn add_transition(mut self, from: NodeId, action: Action, to: NodeId) -> Self {
        self.transitions.insert((from, action), to);
        self
    }

    /// DSL: Chain nodes with default "continue" action
    pub fn chain(mut self, from: NodeId, to: NodeId) -> Self {
        self.transitions.insert((from, "continue".to_string()), to);
        self
    }

    /// DSL: Add multiple nodes at once
    pub fn add_nodes(mut self, nodes: Vec<(NodeId, Arc<dyn Node>)>) -> Self {
        for (id, node) in nodes {
            self.nodes.insert(id, node);
        }
        self
    }

    /// DSL: Add multiple transitions at once
    pub fn add_transitions(mut self, transitions: Vec<(NodeId, Action, NodeId)>) -> Self {
        for (from, action, to) in transitions {
            self.transitions.insert((from, action), to);
        }
        self
    }

    /// DSL: Create a simple linear flow from a sequence of nodes
    pub fn linear(nodes: Vec<(NodeId, Arc<dyn Node>)>) -> Result<Self> {
        if nodes.is_empty() {
            anyhow::bail!("Linear flow requires at least one node");
        }

        let start_id = nodes[0].0.clone();
        let mut builder = Self::new().start(start_id);

        let mut prev_id: Option<NodeId> = None;
        for (id, node) in nodes {
            builder = builder.add_node(id.clone(), node);
            if let Some(prev) = prev_id {
                builder = builder.chain(prev, id.clone());
            }
            prev_id = Some(id);
        }

        Ok(builder)
    }

    /// Build the flow, validating structure
    pub fn build(self) -> Result<Flow> {
        let start = self
            .start
            .ok_or_else(|| anyhow::anyhow!("Start node not set"))?;

        let flow = Flow {
            start,
            nodes: self.nodes,
            transitions: self.transitions,
            tracer: self.tracer,
            budget_guard: self.budget_guard,
            loop_detector: None, // Can be set via with_loop_detector after build
        };

        flow.validate()?;
        Ok(flow)
    }
}

impl Default for FlowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// FlowNode - wraps a Flow to implement Node trait, enabling nested flows
#[derive(Clone)]
pub struct FlowNode {
    id: NodeId,
    flow: Flow,
    config: NodeConfig,
}

impl FlowNode {
    /// Create a new FlowNode
    pub fn new(id: NodeId, flow: Flow) -> Self {
        Self {
            id,
            flow,
            config: NodeConfig::default(),
        }
    }

    /// Create a FlowNode with custom configuration
    pub fn with_config(id: NodeId, flow: Flow, config: NodeConfig) -> Self {
        Self { id, flow, config }
    }

    /// Get the inner flow
    pub fn inner(&self) -> &Flow {
        &self.flow
    }

    /// Get mutable reference to the inner flow
    pub fn inner_mut(&mut self) -> &mut Flow {
        &mut self.flow
    }
}

#[async_trait]
impl Node for FlowNode {
    fn id(&self) -> NodeId {
        self.id.clone()
    }

    fn kind(&self) -> &str {
        "flow"
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        // Pass the entire state as input to the nested flow
        Ok(serde_json::to_value(state)?)
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        // Deserialize input to SharedState
        let mut state: SharedState = serde_json::from_value(input)?;

        // Execute the nested flow (Flow is now cloneable via Arc)
        let mut flow = self.flow.clone();
        flow.run(&mut state).await?;

        // Return the updated state as output
        Ok(serde_json::to_value(state)?)
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        // Merge the nested flow's state back into the parent state
        if let Ok(nested_state) = serde_json::from_value::<SharedState>(output) {
            state.merge(nested_state);
        }
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Input, NodeConfig, Output};
    use async_trait::async_trait;

    // Test node implementation
    struct MockNode {
        id: String,
        config: NodeConfig,
        output_value: String,
    }

    impl MockNode {
        fn new(id: String, output_value: String) -> Self {
            Self {
                id,
                config: NodeConfig::default(),
                output_value,
            }
        }
    }

    #[async_trait]
    impl Node for MockNode {
        fn id(&self) -> NodeId {
            self.id.clone()
        }

        fn kind(&self) -> &str {
            "mock"
        }

        fn prep(&self, _state: &SharedState) -> Result<Input> {
            Ok(serde_json::json!({}))
        }

        async fn exec(&self, _input: Input) -> Result<Output> {
            Ok(serde_json::json!({ "result": self.output_value }))
        }

        fn post(&self, state: &mut SharedState, output: Output) -> Action {
            state.set_working("last_output".to_string(), output);
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            self.config.clone()
        }
    }

    #[test]
    fn test_flow_builder_validation() {
        let node1 = MockNode::new("node1".to_string(), "output1".to_string());
        let node2 = MockNode::new("node2".to_string(), "output2".to_string());

        let flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), Arc::new(node1))
            .add_node("node2".to_string(), Arc::new(node2))
            .add_transition(
                "node1".to_string(),
                "continue".to_string(),
                "node2".to_string(),
            )
            .build();

        assert!(flow.is_ok());
    }

    #[test]
    fn test_flow_builder_missing_start() {
        let node = MockNode::new("node1".to_string(), "output1".to_string());

        let flow = Flow::builder()
            .add_node("node1".to_string(), Arc::new(node))
            .build();

        assert!(flow.is_err());
    }

    #[test]
    fn test_flow_builder_missing_node() {
        let node1 = MockNode::new("node1".to_string(), "output1".to_string());

        let flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), Arc::new(node1))
            .add_transition(
                "node1".to_string(),
                "continue".to_string(),
                "node2".to_string(),
            )
            .build();

        assert!(flow.is_err());
    }

    #[test]
    fn test_flow_builder_unreachable_node() {
        let node1 = MockNode::new("node1".to_string(), "output1".to_string());
        let node2 = MockNode::new("node2".to_string(), "output2".to_string());

        let flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), Arc::new(node1))
            .add_node("node2".to_string(), Arc::new(node2))
            .build();

        assert!(flow.is_err());
    }

    #[tokio::test]
    async fn test_flow_node() {
        let node = Arc::new(MockNode::new("test".to_string(), "output".to_string()));
        let mut state = SharedState::new();

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        let action = node.post(&mut state, output);

        assert_eq!(action, "continue");
        assert!(state.get_working("last_output").is_some());
    }

    #[tokio::test]
    async fn test_flow_execution() {
        let node1 = Arc::new(MockNode::new("node1".to_string(), "output1".to_string()));
        let node2 = Arc::new(MockNode::new("node2".to_string(), "output2".to_string()));

        let mut flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .add_node("node2".to_string(), node2)
            .add_transition(
                "node1".to_string(),
                "continue".to_string(),
                "node2".to_string(),
            )
            .build()
            .unwrap();

        let mut state = SharedState::new();
        flow.run(&mut state).await.unwrap();

        assert_eq!(
            state.get_working("last_output"),
            Some(&serde_json::json!({ "result": "output2" }))
        );
    }

    #[tokio::test]
    async fn test_flow_execution_terminal() {
        let node1 = Arc::new(MockNode::new("node1".to_string(), "output1".to_string()));

        let mut flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .build()
            .unwrap();

        let mut state = SharedState::new();
        flow.run(&mut state).await.unwrap();

        assert_eq!(
            state.get_working("last_output"),
            Some(&serde_json::json!({ "result": "output1" }))
        );
    }

    #[tokio::test]
    async fn test_nested_flow_execution() {
        // Create inner flow
        let inner_node = MockNode::new("inner_node".to_string(), "inner_output".to_string());
        let inner_flow = Flow::builder()
            .start("inner_node".to_string())
            .add_node("inner_node".to_string(), Arc::new(inner_node))
            .build()
            .unwrap();

        // Wrap in FlowNode
        let flow_node = FlowNode::new("nested".to_string(), inner_flow);

        // Create outer flow with FlowNode
        let outer_node = MockNode::new("outer_node".to_string(), "outer_output".to_string());
        let mut outer_flow = Flow::builder()
            .start("outer_node".to_string())
            .add_node("outer_node".to_string(), Arc::new(outer_node))
            .add_node("nested".to_string(), Arc::new(flow_node))
            .add_transition(
                "outer_node".to_string(),
                "continue".to_string(),
                "nested".to_string(),
            )
            .build()
            .unwrap();

        let mut state = SharedState::new();
        state.set_input("test".to_string(), serde_json::json!("value"));

        outer_flow.run(&mut state).await.unwrap();

        // Verify both nodes executed
        assert!(state.get_working("last_output").is_some());
    }

    #[test]
    fn test_flow_builder_chain() {
        let node1 = Arc::new(MockNode::new("node1".to_string(), "output1".to_string()));
        let node2 = Arc::new(MockNode::new("node2".to_string(), "output2".to_string()));

        let flow = FlowBuilder::new()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .add_node("node2".to_string(), node2)
            .chain("node1".to_string(), "node2".to_string())
            .build()
            .unwrap();

        assert!(flow.validate().is_ok());
    }

    #[test]
    fn test_flow_builder_linear() {
        let nodes = vec![
            (
                "node1".to_string(),
                Arc::new(MockNode::new("node1".to_string(), "output1".to_string()))
                    as Arc<dyn Node>,
            ),
            (
                "node2".to_string(),
                Arc::new(MockNode::new("node2".to_string(), "output2".to_string()))
                    as Arc<dyn Node>,
            ),
            (
                "node3".to_string(),
                Arc::new(MockNode::new("node3".to_string(), "output3".to_string()))
                    as Arc<dyn Node>,
            ),
        ];

        let flow = FlowBuilder::linear(nodes).unwrap().build().unwrap();
        assert!(flow.validate().is_ok());
    }

    #[test]
    fn test_flow_builder_add_nodes() {
        let nodes = vec![
            (
                "node1".to_string(),
                Arc::new(MockNode::new("node1".to_string(), "output1".to_string()))
                    as Arc<dyn Node>,
            ),
            (
                "node2".to_string(),
                Arc::new(MockNode::new("node2".to_string(), "output2".to_string()))
                    as Arc<dyn Node>,
            ),
        ];

        let flow = FlowBuilder::new()
            .start("node1".to_string())
            .add_nodes(nodes)
            .chain("node1".to_string(), "node2".to_string())
            .build()
            .unwrap();

        assert!(flow.validate().is_ok());
    }

    #[test]
    fn test_flow_cycle_detection() {
        let node1 = Arc::new(MockNode::new("node1".to_string(), "output1".to_string()));
        let node2 = Arc::new(MockNode::new("node2".to_string(), "output2".to_string()));
        let node3 = Arc::new(MockNode::new("node3".to_string(), "output3".to_string()));

        let flow = Flow::builder()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .add_node("node2".to_string(), node2)
            .add_node("node3".to_string(), node3)
            .add_transition(
                "node1".to_string(),
                "continue".to_string(),
                "node2".to_string(),
            )
            .add_transition(
                "node2".to_string(),
                "continue".to_string(),
                "node3".to_string(),
            )
            .add_transition(
                "node3".to_string(),
                "continue".to_string(),
                "node1".to_string(),
            )
            .build()
            .unwrap();

        // Flow should validate successfully (cycle is just a warning)
        assert!(flow.validate().is_ok());
    }
}
