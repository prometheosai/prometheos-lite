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

# PrometheOS Lite v1.1 Flow Architecture Implementation Plan

This plan implements the complete flow-centric architecture defined in the v1.1 PRD, migrating from the current agent-based system to a universal execution runtime with SQLite memory, process-isolated tools, and parallel flow support.

---

## Technical Decisions

**Memory Backend:** SQLite + sqlite-vec for local-first semantic retrieval with structured tables for metadata and relationships. Local embeddings via provider abstraction (default: local HTTP, fallback: external API).

**Tool Isolation:** Separate child processes with capability/whitelist permission profiles. Docker optional in v1.2+, not required.

**Implementation Strategy:** Sequential phases with working checkpoints. Temporary adapter to wrap agents as nodes, then full migration to Flow runtime.

**Testing:** Preserve existing v1 tests, add comprehensive flow-specific test suite targeting complexity risks.

**Dependencies:** tokio, rusqlite/sqlx, sqlite-vec, tokio::process, tracing, uuid, chrono, tempfile.

---

## Phase 1: Flow Core (Foundation)

### 1.1 Core Types
- Define `SharedState` struct with typed fields (input, context, working, output, meta)
- Define `Action`, `NodeId`, `Input`, `Output` types
- Implement `SharedState` helpers for safe access
- Add unit tests for type safety

### 1.2 Node Trait
- Implement `Node` trait with `id()`, `prep()`, `exec()`, `post()`, `config()`
- Implement `NodeConfig` with retries, retry_delay_ms, timeout_ms
- Add unit tests for trait contract

### 1.3 Flow Engine
- Implement `Flow` struct with start node, nodes HashMap, transitions HashMap
- Implement `run()` execution loop with state mutation
- Add action routing via transition map lookup
- Implement flow validation (dead ends, unreachable nodes, cycles)
- Add unit tests for linear flows

### 1.4 Retry System
- Implement `execute_with_retry()` with exponential backoff
- Add configurable delay handling
- Implement failure propagation
- Add unit tests for retry scenarios

### 1.5 Flow as Node
- Implement `FlowNode` wrapper to compose flows
- Add nested flow support
- Add unit tests for nested execution

### 1.6 Migration Adapter
- Create `AgentNode` adapter wrapping existing Agent trait
- Migrate `PlannerAgent`, `CoderAgent`, `ReviewerAgent` to Node implementations
- Create linear Flow: Planner → Coder → Reviewer
- Add parity tests comparing Flow output vs SequentialOrchestrator output

**Checkpoint:** Flow Core passes all tests, existing agents run as nodes, parity validated.

---

## Phase 2: Orchestration

### 2.1 Maestro
- Implement `Maestro` for flow scheduling
- Add multi-flow orchestration support
- Implement lifecycle tracking
- Add run registry for flow execution history
- Add unit tests

### 2.2 Continuation Engine
- Implement checkpointing (serialize SharedState to disk)
- Implement resume logic (deserialize and continue)
- Add interruption handling
- Implement replay execution capability
- Add unit tests for checkpoint/resume

### 2.3 Flow Types
- Implement branching flow support (conditional transitions)
- Implement looping flows (cycle detection and limits)
- Implement batch flows (iterable input processing)
- Add unit tests for each flow type

**Checkpoint:** Maestro orchestrates flows, continuation engine persists/resumes state, all flow types operational.

---

## Phase 3: Intelligence

### 3.1 Model Router
- Implement `LlmProvider` trait for provider abstraction
- Implement `ModelRouter` with provider/model selection
- Add fallback chains for reliability
- Implement streaming support via callback
- Add unit tests

### 3.2 Tool Runtime
- Implement `Tool` trait with `name()`, `call()`
- Implement `ToolSandboxProfile` with capability/whitelist permissions
- Implement child process execution via `tokio::process`
- Add timeout and output size limits
- Implement network access control
- Add unit tests for sandbox enforcement

### 3.3 LLM Utilities
- Implement unified `call_llm()` utility
- Implement streaming output handler
- Add token limiter (optional)
- Implement retry backoff strategy
- Add unit tests

**Checkpoint:** Model router handles multi-provider scenarios, tools run in isolated processes with permission enforcement.

---

## Phase 4: Memory

### 4.1 SQLite Schema
- Add rusqlite/sqlx dependencies
- Define schema: memories, semantic_chunks, chunk_embeddings, memory_relationships, flow_runs, flow_events, tool_executions
- Implement database migration system
- Add unit tests for schema

### 4.2 Embedding Provider
- Implement `EmbeddingProvider` trait
- Implement local HTTP embedding provider (default)
- Implement external API-compatible provider (fallback)
- Add unit tests

### 4.3 Memory Service
- Implement `MemoryService` for CRUD operations
- Implement semantic retrieval via sqlite-vec
- Implement episodic logging
- Implement graph relationship queries
- Add unit tests

### 4.4 Flow Integration
- Implement `ContextLoaderNode` for memory retrieval
- Implement `MemoryWriteNode` for persistence
- Add SharedState ↔ MemoryService sync layer
- Add integration tests

**Checkpoint:** SQLite memory service stores/retrieves data, semantic search operational, flows read/write memory.

---

## Phase 5: Advanced Execution

### 5.1 Parallel Flows
- Implement async parallel node execution
- Add concurrency limits
- Implement parallel flow coordination
- Add unit tests for parallel execution

### 5.2 Self-Reflection Loops
- Implement looping nodes for agent-like cycles
- Add loop limit detection
- Implement reflection nodes
- Add unit tests

### 5.3 Batch Processing
- Implement batch flow for iterable inputs
- Add progress tracking
- Implement error aggregation
- Add unit tests

**Checkpoint:** Parallel flows respect concurrency limits, self-reflection loops operate safely, batch processing works.

---

## Phase 6: Developer Experience

### 6.1 CLI Runner
- Update CLI to run flows via Maestro
- Add flow file loading (JSON/YAML)
- Preserve verbose flag and logging
- Add unit tests

### 6.2 Flow Builder DSL
- Implement simplified flow creation API
- Add builder pattern for Flow construction
- Add validation at build time
- Add unit tests

### 6.3 Debug Mode
- Implement step-by-step execution
- Add state snapshot logging
- Implement breakpoint capability
- Add unit tests

### 6.4 Logging & Tracing
- Integrate tracing crate
- Implement structured logging
- Add event timeline tracking
- Add log filtering

**Checkpoint:** CLI runs flows, DSL simplifies flow creation, debug mode enables inspection, tracing provides observability.

---

## Phase 7: Safety & Control

### 7.1 Policy Hooks
- Implement policy hook system
- Add pre/post execution validation
- Implement constitution-policy integration
- Add unit tests

### 7.2 Tool Sandbox Enforcement
- Implement permission model validation
- Add capability checking
- Implement whitelist enforcement
- Add security tests

### 7.3 Rate Limiting
- Implement token budgeting
- Add execution guardrails
- Implement rate limiter
- Add unit tests

**Checkpoint:** Policy hooks validate flows, tool sandbox enforces permissions, rate limiting prevents abuse.

---

## Migration & Cleanup

### 8.1 Remove Legacy Code
- Remove `SequentialOrchestrator` after parity validation
- Remove old `Agent` trait if redundant
- Update CLI to use Flow runtime exclusively
- Remove deprecated modules

### 8.2 Documentation
- Update README with flow architecture
- Add flow construction examples
- Document migration guide
- Update CHANGELOG

### 8.3 Final Testing
- Run full test suite (v1 + v1.1 tests)
- Validate all 7 phases operational
- Performance benchmark (sub-100ms orchestration overhead)
- End-to-end testing with LM Studio

**Final:** Complete v1.1 flow architecture operational, legacy code removed, documentation updated.

---

## Validation Gates

1. Existing v1 CLI behavior reproducible as Flow
2. Planner/Coder/Reviewer Flow produces files
3. Failed node retries correctly
4. Failed tool execution isolated
5. Memory writes, embeds, retrieves
6. Parallel flow respects concurrency limits
7. All validation gates pass before phase completion

---

## Risk Mitigation

- **Complexity explosion:** Each phase has working checkpoint, tests validate incrementally
- **SharedState type safety:** Typed struct instead of HashMap, validation at compile time
- **Graph validation:** Flow construction validates completeness, unreachable nodes, cycles
- **Memory ambiguity:** Concrete SQLite schema with clear table definitions
- **Sandbox ambiguity:** Capability/whitelist profiles with enforcement tests
- **Migration risk:** Parity tests ensure output equivalence before removing legacy code
