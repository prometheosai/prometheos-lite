# How Flows Work

This guide explains the flow execution model in PrometheOS Lite, from the basic concepts to advanced patterns.

## Core Concepts

### What is a Flow?

A **Flow** is a directed graph of nodes that execute in sequence based on state transitions. Each node performs a specific task, and the flow determines which node executes next based on the action returned by the current node.

### Flow Execution Lifecycle

```
1. Load flow from JSON
2. Create SharedState with input
3. Start at start_node
4. For each node:
   a. prep() - Prepare input from state
   b. exec() - Execute the node (async)
   c. post() - Process output and determine next action
   d. Transition to next node based on action
5. Continue until no transition exists
6. Return final state outputs
```

### SharedState

`SharedState` is the data container that flows through the execution. It has four buckets:

- **input**: Initial input provided to the flow (read-only)
- **context**: Contextual information loaded from memory or external sources
- **working**: Temporary working data between nodes
- **output**: Final outputs produced by nodes
- **meta**: Metadata about execution (node counts, timestamps, etc.)

### Node Lifecycle

Each node implements the `Node` trait with three methods:

```rust
trait Node {
    fn id(&self) -> String;
    fn prep(&self, state: &SharedState) -> Result<Value>;
    async fn exec(&self, input: Value) -> Result<Value>;
    fn post(&self, state: &mut SharedState, output: Value) -> String;
}
```

#### prep()
Synchronous preparation phase. Extracts data from `SharedState` and formats it for the node's execution.

```rust
fn prep(&self, state: &SharedState) -> Result<Value> {
    let task = state.get_input("task")?;
    Ok(json!({ "task": task }))
}
```

#### exec()
Async execution phase. Performs the actual work (LLM calls, file I/O, tool execution, etc.).

```rust
async fn exec(&self, input: Value) -> Result<Value> {
    let task = input["task"].as_str()?;
    let response = llm.generate(task).await?;
    Ok(json!({ "result": response }))
}
```

#### post()
Synchronous post-processing phase. Stores output in `SharedState` and returns the action for the next transition.

```rust
fn post(&self, state: &mut SharedState, output: Value) -> String {
    state.set_output("result", output["result"]);
    "continue".to_string()  // Action determines next node
}
```

## Flow File Structure

A flow file is a JSON document that defines:

```json
{
  "version": "1",
  "name": "Flow Name",
  "description": "What this flow does",
  "start_node": "planner",
  "nodes": [
    {
      "id": "planner",
      "node_type": "planner",
      "config": {
        "retries": 2,
        "retry_delay_ms": 1000
      }
    }
  ],
  "transitions": [
    {
      "from": "planner",
      "action": "continue",
      "to": "coder"
    }
  ]
}
```

### Fields

- **version**: Flow format version (currently "1")
- **name**: Human-readable flow name
- **description**: What the flow does
- **start_node**: ID of the node where execution begins
- **nodes**: Array of node definitions
- **transitions**: Array of transition rules

### Node Definition

- **id**: Unique identifier for the node
- **node_type**: Type of node (determines implementation)
- **config**: Optional configuration (retries, timeout, etc.)

### Transition Definition

- **from**: Source node ID
- **action**: Action string that triggers this transition
- **to**: Target node ID

## State Transitions

Transitions determine the execution path. When a node's `post()` method returns an action, the flow looks up the transition:

```json
{
  "from": "planner",
  "action": "continue",
  "to": "coder"
}
```

If no matching transition exists, the flow terminates.

### Common Actions

- **continue**: Proceed to the next node in the sequence
- **retry**: Retry the current node (with retry logic)
- **skip**: Skip to a different node
- **stop**: Terminate the flow

## Built-in Node Types

### LLM Nodes

**planner**: Creates structured plans
```json
{
  "id": "planner",
  "node_type": "planner"
}
```

**coder**: Generates code
```json
{
  "id": "coder",
  "node_type": "coder"
}
```

**reviewer**: Reviews and refines code
```json
{
  "id": "reviewer",
  "node_type": "reviewer"
}
```

**llm**: Generic LLM node for custom prompts
```json
{
  "id": "custom_llm",
  "node_type": "llm",
  "config": {
    "prompt": "You are a helpful assistant. Answer: {input}"
  }
}
```

### File System Nodes

**file_writer**: Writes files to disk
```json
{
  "id": "file_writer",
  "node_type": "file_writer"
}
```
Writes to `prometheos-output/` directory by default.

### Memory Nodes

**context_loader**: Loads context from memory
```json
{
  "id": "context_loader",
  "node_type": "context_loader"
}
```

**memory_write**: Writes data to memory
```json
{
  "id": "memory_write",
  "node_type": "memory_write"
}
```

### Control Flow Nodes

**conditional**: Branching based on state
```json
{
  "id": "conditional",
  "node_type": "conditional",
  "config": {
    "condition": "state.output.quality > 0.8",
    "true_action": "continue",
    "false_action": "retry"
  }
}
```

**passthrough**: No-op node (default for unknown types)

### Tool Nodes

**tool**: Executes external tools
```json
{
  "id": "tool",
  "node_type": "tool",
  "config": {
    "command": "git",
    "args": ["status"]
  }
}
```

## Advanced Patterns

### Sequential Pipeline

Simple linear execution:

```
planner → coder → reviewer → file_writer
```

### Conditional Branching

Branch based on state:

```
reviewer → (if approved) → file_writer
reviewer → (if rejected) → coder
```

### Loop with Counter

Store loop counter in `meta`:

```
start → process → (if count < 3) → process
process → (if count >= 3) → finish
```

### Parallel Execution

Use `ParallelNode` for concurrent flows:

```json
{
  "id": "parallel",
  "node_type": "parallel",
  "config": {
    "flows": ["task1.flow.json", "task2.flow.json"],
    "concurrency": 2
  }
}
```

### Nested Flows

Use `FlowNode` to wrap a flow as a node:

```json
{
  "id": "subflow",
  "node_type": "flow",
  "config": {
    "flow_file": "subtask.flow.json"
  }
}
```

## Error Handling

### Retries

Nodes can be configured with retry logic:

```json
{
  "config": {
    "retries": 3,
    "retry_delay_ms": 1000
  }
}
```

If a node fails, it will retry up to the specified count with exponential backoff.

### Graceful Degradation

Some nodes handle failures gracefully:

- **memory_write**: Skips if embedding server is unavailable
- **context_loader**: Returns empty context if memory service is missing
- **tool**: Returns placeholder if tool runtime is not configured

## Service Injection

Nodes can access services through `RuntimeContext`:

- **ModelRouter**: For LLM calls
- **ToolRuntime**: For tool execution
- **MemoryService**: For memory operations

The CLI injects these services when creating the flow:

```rust
let runtime = RuntimeContext::full(model_router, tool_runtime, memory_service);
FlowRunner::from_json_file_with_runtime(path, Some(runtime))
```

## Debugging

### Enable Debug Mode

```bash
prometheos flow examples/codegen.flow.json --debug
```

Debug mode provides:
- State snapshots before/after each node
- Step-by-step execution
- Detailed error messages

### Checkpointing

Flows can be checkpointed for resumption:

```bash
prometheos flow examples/codegen.flow.json --checkpoint
```

Checkpoints are saved to `.checkpoints/` directory.

## Best Practices

1. **Keep nodes focused**: Each node should do one thing well
2. **Use state buckets appropriately**: 
   - `input` for initial data
   - `context` for loaded context
   - `working` for intermediate data
   - `output` for final results
3. **Design for idempotency**: Nodes should handle being retried safely
4. **Add descriptive names**: Use clear node IDs and flow names
5. **Document your flows**: Add descriptions to flow files
6. **Test incrementally**: Start with simple flows, add complexity gradually

## Example: Complete Code Generation Flow

```json
{
  "version": "1",
  "name": "Code Generation Flow",
  "description": "Plans, generates, reviews, and writes code",
  "start_node": "planner",
  "nodes": [
    {
      "id": "planner",
      "node_type": "planner"
    },
    {
      "id": "coder",
      "node_type": "coder"
    },
    {
      "id": "reviewer",
      "node_type": "reviewer"
    },
    {
      "id": "file_writer",
      "node_type": "file_writer"
    },
    {
      "id": "memory_write",
      "node_type": "memory_write"
    }
  ],
  "transitions": [
    {
      "from": "planner",
      "action": "continue",
      "to": "coder"
    },
    {
      "from": "coder",
      "action": "continue",
      "to": "reviewer"
    },
    {
      "from": "reviewer",
      "action": "continue",
      "to": "file_writer"
    },
    {
      "from": "file_writer",
      "action": "continue",
      "to": "memory_write"
    }
  ]
}
```

Run it:

```bash
prometheos flow examples/codegen.flow.json --input '{"task":"Build a Rust CLI todo app"}'
```

## Further Reading

- [OVERVIEW.md](../../OVERVIEW.md) - Complete architecture documentation
- [examples/README.md](../../examples/README.md) - Available example flows
- [CHANGELOG.md](../../CHANGELOG.md) - Version history and changes
