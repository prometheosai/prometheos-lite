# 🧠 PrometheOS Lite

**Flow-centric AI agent orchestration system.**

PrometheOS Lite v1.1 is a Rust-based, local-first system for orchestrating AI agents through a flow-centric architecture. It provides powerful execution capabilities including parallel flows, self-reflection loops, batch processing, debugging, policy enforcement, and rate limiting.

---

## ⚡ Features

### Flow-Centric Architecture
- **Flow Engine**: Execute complex workflows with nodes, transitions, and state management
- **Flow Builder DSL**: Simplified API for constructing flows with builder pattern
- **Nested Flows**: Compose flows within flows for hierarchical orchestration

### Advanced Execution
- **Parallel Flows**: Execute multiple flows concurrently with configurable concurrency limits
- **Self-Reflection Loops**: Loop nodes with reflection functions for iterative improvement
- **Batch Processing**: Process iterable inputs with progress tracking callbacks

### Developer Experience
- **CLI Runner**: Execute flows from JSON files with input data
- **Debug Mode**: Step-by-step execution with state snapshots and breakpoints
- **Logging & Tracing**: Structured logs and event timeline for observability

### Safety & Control
- **Policy Hooks**: Pre/post validation with constitution-policy enforcement
- **Tool Sandbox**: Permission model for command execution with capability checking
- **Rate Limiting**: Token budgeting and execution guardrails

### Intelligence
- **Model Router**: Provider abstraction with fallback chains
- **Tool Runtime**: Tool trait with sandbox profiles and process isolation
- **LLM Utilities**: Unified interface with streaming and retry logic

### Memory
- **SQLite Storage**: Persistent memories with embeddings
- **Semantic Retrieval**: Context-aware memory access
- **Flow Integration**: ContextLoaderNode and MemoryWriteNode

---

## 🚀 Quick Start

### 1. Clone the repository

```bash
git clone https://github.com/prometheosai/prometheos-lite
cd prometheos-lite
```

### 2. Install dependencies

```bash
cargo build
```

### 3. Configure

Create a `prometheos.config.json` file:

```json
{
  "provider": "lmstudio",
  "base_url": "http://localhost:1234/v1",
  "model": "your-model-name"
}
```

### 4. Run a flow

```bash
cargo run -- flow path/to/flow.json --input '{"key": "value"}'
```

### 5. Example Flow

Create a simple flow file `my_flow.json`:

```json
{
  "name": "my_flow",
  "description": "A simple example flow",
  "start_node": "node1",
  "nodes": [
    {
      "id": "node1",
      "node_type": "placeholder",
      "config": null
    }
  ],
  "transitions": []
}
```

---

## 🧠 Architecture

### Flow-Centric Design

```text
Flow
├── Nodes (execution units)
│   ├── prep() - prepare input from state
│   ├── exec() - execute with input
│   └── post() - post-process with output
├── Transitions (node → action → node)
└── SharedState (input, context, working, output, meta)
```

### Key Concepts

- **Node**: Atomic execution unit with prep, exec, post lifecycle
- **Flow**: Directed graph of nodes with transitions
- **SharedState**: Explicit state management across nodes
- **Action**: Output from post() that determines next node

---

## 📦 Project Structure

```text
/src
  /cli        → CLI entrypoint and flow runner
  /flow       → Flow-centric architecture
    /flow.rs          → Flow engine and builder
    /flow_types.rs    → Specialized flow types (parallel, batch, etc.)
    /intelligence.rs  → Model router, tool runtime, LLM utilities
    /memory.rs        → SQLite storage and semantic retrieval
    /debug.rs         → Debug mode with breakpoints
    /tracing.rs       → Logging and event timeline
    /policy.rs        → Policy hooks and validation
    /rate_limit.rs    → Token budgeting and guardrails
  /agents     → Legacy agent interfaces (deprecated)
  /core       → Legacy orchestration (deprecated)
  /llm        → Model client (LM Studio-compatible)
  /fs         → File parsing & writing
  /logger     → Streaming logs
  /config     → Configuration loader
```

---

## 🔌 Model Support

Compatible with:

- Local models via LM Studio (OpenAI-compatible API)
- Any OpenAI-compatible endpoint
- Multiple providers with automatic fallback

---

## 🧑‍💻 Contributing

We welcome contributions.

### Setup

```bash
cargo build
cargo test
```

### Guidelines

- Keep modules simple and focused
- Prefer clarity over abstraction
- Avoid unnecessary complexity
- Ensure features run locally without external dependencies
- Write tests for new features

---

## 📌 Migration from v1.0

The v1.1 release introduces a flow-centric architecture. The old agent-based system is deprecated but still functional.

### Key Changes

- **Agent trait** → **Node trait**
- **SequentialOrchestrator** → **FlowBuilder**
- **Agent execution** → **Flow execution**

### Migration Steps

1. Wrap existing agents with `AgentNode::new(agent)`
2. Build flows using `FlowBuilder`
3. Use `Flow::run()` for execution

See `docs/guides/migration.md` for detailed migration guide.

---

## ⚠️ Disclaimer

This tool generates code automatically.
Always review outputs before using in production.

---

## 🔗 PrometheOS

PrometheOS Lite is part of the PrometheOS ecosystem.

GitHub: [https://github.com/orgs/prometheosai](https://github.com/orgs/prometheosai)

---

## 📚 Documentation Layout

Project documentation is organized under `/docs`:

- `docs/prd` - Product requirements and roadmap docs
- `docs/operations` - Project operations and management setup
- `docs/guides` - Contributor and usage guides
- `docs/architecture` - Technical design references
