So now we’re doing it properly. Finally.
You’re basically compressing a distributed AI organism into a **clean execution primitive** instead of letting it sprawl like a startup pitch deck.

Let’s build this like something that actually survives contact with reality.

---

# 🧠 PrometheOS Lite v1.1 — PRD (Flow-Centric Architecture)

---

## ## 1. Executive Definition

PrometheOS Lite v1.1 is a **Flow-native AI execution engine** built in Rust, designed to orchestrate intelligent systems through:

* deterministic + agentic flows
* composable nodes
* shared execution state
* pluggable intelligence + tools
* persistent memory integration

The system must:

* run locally (edge-first)
* support zero-cost operation paths
* enable agentic coding + automation
* scale into distributed architecture without redesign

---

## ## 2. Core Principles (Non-Negotiable)

1. **Everything is a Flow**
2. **Execution = Node lifecycle**
3. **State is explicit (SharedState)**
4. **No vendor lock-in**
5. **Async is optional, not invasive**
6. **Retry + failure are first-class**
7. **Orchestration is thin, not bloated**

---

# ⚙️ 3. System Architecture

---

## ### Layer 0 — Flow Core (NEW)

### Responsibilities:

* execution model
* graph traversal
* state handling
* failure control

---

### 🔹 Core Types

```rust
type SharedState = HashMap<String, serde_json::Value>;
type Action = String;
type NodeId = String;
```

---

### 🔹 Node Trait

```rust
trait Node {
    fn id(&self) -> NodeId;

    fn prep(&self, state: &SharedState) -> Result<Input>;
    fn exec(&self, input: Input) -> Result<Output>;
    fn post(&self, state: &mut SharedState, output: Output) -> Action;

    fn config(&self) -> NodeConfig;
}
```

---

### 🔹 NodeConfig

```rust
struct NodeConfig {
    retries: u8,
    retry_delay_ms: u64,
    timeout_ms: Option<u64>,
}
```

---

### 🔹 Flow

```rust
struct Flow {
    start: NodeId,
    nodes: HashMap<NodeId, Box<dyn Node>>,
    transitions: HashMap<(NodeId, Action), NodeId>,
}
```

---

### 🔹 Execution Engine

```rust
fn run(&mut self, state: &mut SharedState) -> Result<()> {
    let mut current = self.start.clone();

    loop {
        let node = self.nodes.get(&current)?;

        let input = node.prep(state)?;
        let output = self.execute_with_retry(node, input)?;
        let action = node.post(state, output);

        match self.transitions.get(&(current.clone(), action)) {
            Some(next) => current = next.clone(),
            None => break,
        }
    }

    Ok(())
}
```

---

### 🔹 Retry + Fallback

```rust
fn execute_with_retry(...) {
    // retry loop
    // exponential backoff optional
    // fallback execution on failure
}
```

---

## ### Layer 1 — Orchestration

---

### 🔹 maestro

Responsibilities:

* flow scheduling
* multi-flow orchestration
* lifecycle tracking
* run registry

---

### 🔹 continuation-engine

Responsibilities:

* resume flows
* checkpoint state
* handle interruptions
* replay execution

---

### 🔹 Flow Types Supported

* linear
* branching
* looping
* nested flows
* batch flows
* parallel flows

---

## ### Layer 2 — Intelligence

---

### 🔹 model-router

Responsibilities:

* route LLM calls
* select provider/model
* fallback chains
* streaming support

---

### 🔹 tools-runtime

Responsibilities:

* execute tools (CLI / APIs)
* sandbox execution
* structured I/O

---

### 🔹 Tool Interface

```rust
trait Tool {
    fn name(&self) -> String;
    fn call(&self, input: ToolInput) -> ToolOutput;
}
```

---

## ### Layer 3 — Memory

---

### 🔹 memory-service

Responsibilities:

* long-term memory
* semantic retrieval
* episodic logs
* graph relationships

---

### 🔹 Integration with Flow

* Nodes read/write SharedState
* Sync to memory-service when needed
* Retrieve context at entry nodes

---

# 🔄 4. Canonical Execution Flow

---

### Example:

```
User Input
   ↓
Gateway
   ↓
Maestro
   ↓
Flow.run()
   ↓
Node.prep()
   ↓
Node.exec() → model-router / tools-runtime
   ↓
Node.post()
   ↓
Action → Next Node
   ↓
Final Output
```

---

# 🧩 5. Utility Functions (Required)

---

## 🔹 Core Utilities

* `call_llm()`
* `execute_tool()`
* `load_context()`
* `save_context()`
* `log_event()`
* `validate_output()`

---

## 🔹 Advanced Utilities

* retry backoff strategy
* streaming output handler
* token limiter
* caching layer (optional)
* tracing hooks

---

# 🧱 6. Data Contracts

---

## 🔹 SharedState Convention

```json
{
  "input": {},
  "context": {},
  "working": {},
  "output": {},
  "meta": {}
}
```

---

## 🔹 Node I/O

* Input: derived from SharedState
* Output: transient
* Post: writes to SharedState

---

# 🚨 7. Non-Functional Requirements

---

### Performance

* sub-100ms orchestration overhead
* async support via tokio

### Reliability

* deterministic retries
* failure isolation per node

### Security

* sandboxed tool execution
* no arbitrary shell execution by default

### Extensibility

* plug-in architecture for tools + models

---

# 🧨 8. FULL ISSUE BREAKDOWN (Execution Plan)

---

## 🔥 PHASE 1 — Flow Core

---

### 1. Node Trait + Config

* implement Node trait
* implement NodeConfig
* unit tests

---

### 2. Flow Engine

* implement Flow struct
* transitions map
* execution loop

---

### 3. Action Routing

* implement transition resolution
* default action handling

---

### 4. Retry System

* retry loop
* delay handling
* failure propagation

---

### 5. SharedState

* define schema
* implement helpers

---

### 6. Flow as Node

* implement FlowNode wrapper
* nested flow support

---

## 🔥 PHASE 2 — Orchestration

---

### 7. Maestro Integration

* connect Flow execution
* run lifecycle tracking

---

### 8. Continuation Engine

* checkpointing
* resume logic
* serialization

---

### 9. Run Registry

* track runs
* status lifecycle
* logs

---

## 🔥 PHASE 3 — Intelligence

---

### 10. Model Router Core

* provider abstraction
* routing strategy

---

### 11. Tool Runtime

* tool trait
* execution engine

---

### 12. LLM Utility

* unified call interface
* streaming support

---

## 🔥 PHASE 4 — Memory

---

### 13. Memory Sync Layer

* SharedState ↔ memory-service bridge

---

### 14. Context Loader Node

* retrieval logic
* injection into state

---

### 15. Memory Write Node

* persist outputs

---

## 🔥 PHASE 5 — Advanced Execution

---

### 16. Batch Flow

* iterable execution

---

### 17. Parallel Execution

* async parallel nodes
* concurrency limits

---

### 18. Looping / Self-Reflection Nodes

* agent-like cycles

---

## 🔥 PHASE 6 — Developer Experience

---

### 19. CLI Runner

* run flows locally

---

### 20. Flow Builder DSL

* simplified flow creation

---

### 21. Debug Mode

* step-by-step execution
* state snapshots

---

### 22. Logging + Tracing

* structured logs
* event timeline

---

## 🔥 PHASE 7 — Safety & Control

---

### 23. Policy Hook Integration

* connect constitution-policy

---

### 24. Tool Sandbox

* restrict execution
* permission model

---

### 25. Rate Limiting / Budgeting

* token control
* execution guardrails

---

# 🧠 Final Positioning

What you’re building now is no longer:

* a collection of services
* an agent framework
* or a coding tool

It becomes:

> **a universal execution runtime for intelligent systems**

Which is much harder… and much more valuable.

And yes, this version is finally something that:

* scales
* stays clean
* and doesn’t collapse under its own ambition

Now it’s just execution.
