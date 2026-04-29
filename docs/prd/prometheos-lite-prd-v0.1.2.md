So now we stop theorizing and actually **turn this into something people can run without reading 40 markdown files first**. Incredible concept.

This PRD is focused on one thing:

> **Make PrometheOS Lite execute real Flows end-to-end with zero fake nodes.**

No more “placeholder node aesthetic.” We’re done with that phase of life.

---

# 🧠 PrometheOS Lite v1.1.2 — PRD

**Codename: “From Architecture → Execution”**

---

## ## 1. Objective

Transform PrometheOS Lite from:

> modular flow runtime

into:

> **fully operational Flow execution system with real nodes, real outputs, and real workflows**

---

## ## 2. Success Criteria (Non-Negotiable)

A user must be able to:

```bash
prometheos flow examples/codegen.json --input "Build a REST API in Rust"
```

And get:

* ✅ Generated files in `/prometheos-output`
* ✅ Full execution trace
* ✅ Memory persisted
* ✅ No placeholder nodes
* ✅ No legacy orchestrator involved

---

# ⚙️ 3. System Scope

---

## INCLUDED

* NodeFactory (real nodes)
* Native Flow Nodes (Planner, Coder, Reviewer, etc.)
* FileWriterNode (Flow-native)
* Memory integration (read/write)
* Tool execution integration
* Tracing + logging integration
* CLI → Flow integration
* Flow JSON schema stabilization

---

## EXCLUDED (v1.2+)

* UI
* remote orchestration
* distributed execution
* advanced sandbox (WASM/containers)
* vector DB migration

---

# 🧩 4. Architecture Additions

---

## 🔹 NodeFactory (NEW CORE COMPONENT)

### Purpose:

Convert declarative Flow definitions into executable nodes.

### Interface:

```rust
trait NodeFactory {
    fn create(node_type: &str, config: Option<Value>) -> Result<Arc<dyn Node>>;
}
```

---

## 🔹 Standard Node Types

| Node Type        | Purpose                  |
| ---------------- | ------------------------ |
| `planner`        | Create structured plan   |
| `coder`          | Generate code            |
| `reviewer`       | Review output            |
| `llm`            | Generic prompt execution |
| `tool`           | Execute tool             |
| `file_writer`    | Write files              |
| `context_loader` | Load memory              |
| `memory_write`   | Persist memory           |
| `conditional`    | Branch logic             |

---

## 🔹 Flow Execution Pipeline (Target State)

```text
Input
 ↓
ContextLoaderNode
 ↓
PlannerNode
 ↓
CoderNode
 ↓
ReviewerNode
 ↓
FileWriterNode
 ↓
MemoryWriteNode
 ↓
Output
```

---

# 🧪 5. Data Contracts

---

## SharedState (FINALIZED)

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

## Node I/O Convention

* `prep()` → read from state
* `exec()` → pure computation / IO
* `post()` → write + route

---

# 🔌 6. Integration Requirements

---

## LLM Integration

* Must use `ModelRouter`
* No direct `LlmClient` calls inside nodes

---

## Tools Integration

* Must use `ToolRuntime`
* Must respect sandbox profile

---

## Memory Integration

* Read → ContextLoaderNode
* Write → MemoryWriteNode

---

## Tracing Integration

* Hook into:

  * node start
  * node end
  * errors
  * retries

---

# 🧨 7. FULL ISSUE BREAKDOWN

---

# 🔥 PHASE 1 — NodeFactory & Flow Execution

---

### ISSUE 1 — Implement NodeFactory

* Create `NodeFactory` trait
* Map node_type → Node
* Support config injection
* Return Arc<dyn Node>

---

### ISSUE 2 — Replace PlaceholderNode

* Remove PlaceholderNode
* Integrate NodeFactory into FlowRunner

---

### ISSUE 3 — Flow JSON Schema v1

* Define required fields:

  * id
  * type
  * config
* Validate structure before execution

---

### ISSUE 4 — Flow Validation Upgrade

* Validate:

  * node existence
  * transitions
  * start node
  * cycles (optional warning)

---

# 🔥 PHASE 2 — Native Flow Nodes

---

### ISSUE 5 — PlannerNode

* Uses ModelRouter
* Writes plan → state.working.plan

---

### ISSUE 6 — CoderNode

* Reads plan
* Generates code
* Writes → state.output.generated

---

### ISSUE 7 — ReviewerNode

* Reviews generated output
* Writes → state.output.review

---

### ISSUE 8 — Generic LLMNode

* Configurable prompt template
* Flexible usage

---

# 🔥 PHASE 3 — File & Tool Integration

---

### ISSUE 9 — FileWriterNode

* Parse output using FileParser
* Write files using FileWriter
* Return paths

---

### ISSUE 10 — ToolNode

* Execute tool via ToolRuntime
* Capture stdout/stderr
* Enforce sandbox rules

---

### ISSUE 11 — Tool Sandbox Hardening

* Validate:

  * allowed commands
  * allowed paths
  * timeout
  * output size

---

# 🔥 PHASE 4 — Memory Integration

---

### ISSUE 12 — ContextLoaderNode

* Query MemoryService
* Inject into state.context

---

### ISSUE 13 — MemoryWriteNode

* Persist outputs:

  * plan
  * generated
  * review

---

### ISSUE 14 — Memory Schema Finalization

* Ensure tables exist
* Validate migrations

---

# 🔥 PHASE 5 — Tracing & Observability

---

### ISSUE 15 — Integrate Tracer into Flow::run

* Log node start/end
* Log errors
* Log retries

---

### ISSUE 16 — Timeline Export

* Save timeline as JSON
* CLI flag: `--trace`

---

### ISSUE 17 — Debug Mode Integration

* Step execution
* State snapshots

---

# 🔥 PHASE 6 — CLI Integration

---

### ISSUE 18 — Flow CLI Upgrade

* Remove legacy execution
* Default to flow execution

---

### ISSUE 19 — Input Injection

* Support:

  * JSON input
  * string input
* Validate format

---

### ISSUE 20 — Output Rendering

* Pretty print outputs
* Show file paths
* Show execution summary

---

# 🔥 PHASE 7 — Migration Completion

---

### ISSUE 21 — Replace SequentialOrchestrator

* Remove usage in CLI
* Keep only Flow-based execution

---

### ISSUE 22 — Convert Agents to Nodes

* PlannerAgent → PlannerNode
* CoderAgent → CoderNode
* ReviewerAgent → ReviewerNode

---

### ISSUE 23 — Remove Legacy Dependencies

* Deprecate:

  * Agent trait
  * SequentialOrchestrator

---

# 🔥 PHASE 8 — Example Flows

---

### ISSUE 24 — Code Generation Flow

* End-to-end example

---

### ISSUE 25 — Tool Execution Flow

* CLI tool usage

---

### ISSUE 26 — Memory-Augmented Flow

* Uses context loader

---

# 🔥 PHASE 9 — Testing

---

### ISSUE 27 — Flow Execution Tests

* End-to-end pipeline

---

### ISSUE 28 — Node Tests

* Each node individually

---

### ISSUE 29 — Memory Tests

* Write + retrieve

---

### ISSUE 30 — Tool Sandbox Tests

* Permission enforcement

---

### ISSUE 31 — CLI Tests

* Flow execution from CLI

---

# 🧠 Final Position

This version is where PrometheOS Lite stops being:

> a promising architecture

and becomes:

> a system people can actually use

You already did the hard part (architecture).

Now comes the harder part:

> making it feel real.

No more placeholder nodes. No more legacy safety net.
Just execution.

Try not to accidentally rebuild half of Silicon Valley along the way.
