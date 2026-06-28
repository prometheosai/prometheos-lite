# 🧠 PrometheOS Lite

**Flow-centric AI agent orchestration system.**

PrometheOS Lite v1.2 is a Rust-based, local-first system for orchestrating AI agents through a flow-centric architecture. It provides powerful execution capabilities including parallel flows, self-reflection loops, batch processing, debugging, policy enforcement, rate limiting, and WorkContext lifecycle management.

---

## ⚡ Features

### Flow-Centric Architecture
- **Flow Engine**: Execute complex workflows with nodes, transitions, and state management
- **Flow Builder DSL**: Simplified API for constructing flows with builder pattern
- **Nested Flows**: Compose flows within flows for hierarchical orchestration

### WorkContext Management (V1.2)
- **Lifecycle Management**: Track work through phases (Intake, Planning, Execution, Review, Finalization)
- **Autonomy Levels**: Configure Chat, Autonomous, or Review modes for different work styles
- **Approval Policies**: Auto, ManualAll, or RequireForSideEffects for control over execution
- **Artifact Tracking**: Automatically capture and manage outputs from flow execution
- **Domain Profiles**: Pre-configured templates for different work domains (Software, Business, Marketing, etc.)
- **Guardrails**: Built-in safety checks for blocked contexts and approval requirements

### Harness Spine (V1.2.5)
- **WorkOrchestrator**: Central execution loop with hard stop contracts (max iterations, runtime, tool calls, cost)
- **PlaybookResolver**: Intelligent playbook selection based on domain matching and usage history
- **Persistent Work Execution**: WorkContext as default execution path for all operations
- **Intent Routing**: CODING_TASK and APPROVAL intents route to WorkOrchestrator
- **CLI Commands**: `work submit`, `work continue`, `work run` for orchestration
- **API Endpoints**: REST endpoints for intent submission, context continuation, and bounded execution

### Advanced Execution
- **Parallel Flows**: Execute multiple flows concurrently with configurable concurrency limits
- **Self-Reflection Loops**: Loop nodes with reflection functions for iterative improvement
- **Batch Processing**: Process iterable inputs with progress tracking callbacks

### Developer Experience
- **CLI Runner**: Execute flows from JSON files with input data
- **WorkContext CLI**: Create, list, show, and continue WorkContexts from the command line
- **Template Management**: Install and manage domain profile templates
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
  "provider": "openrouter",
  "base_url": "https://openrouter.ai/api",
  "model": "meta-llama/llama-3.3-8b-instruct:free",
  "llm_routing": {
    "billing_source": "openrouter_user",
    "providers": [
      {
        "name": "openrouter_fast",
        "provider_type": "openrouter",
        "enabled": true,
        "base_url": "https://openrouter.ai/api",
        "model": "meta-llama/llama-3.3-8b-instruct:free",
        "api_key_env": "OPENROUTER_API_KEY"
      },
      {
        "name": "openrouter_balanced",
        "provider_type": "openrouter",
        "enabled": true,
        "base_url": "https://openrouter.ai/api",
        "model": "mistralai/mistral-7b-instruct:free",
        "api_key_env": "OPENROUTER_API_KEY"
      }
    ],
    "mode_chains": {
      "fast": ["openrouter_fast", "openrouter_balanced"],
      "balanced": ["openrouter_balanced", "openrouter_fast"],
      "deep": ["openrouter_balanced"],
      "coding": ["openrouter_balanced"]
    }
  }
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

### 6. WorkContext Examples (V1.2)

#### Create a WorkContext

```bash
prometheos work create --title "Build REST API" --domain software --goal "Create a REST API for user management"
```

#### List WorkContexts

```bash
prometheos work list
```

#### Show WorkContext Details

```bash
prometheos work show <context-id>
```

#### Continue a WorkContext

```bash
prometheos work continue <context-id>
```

#### Manage Templates

```bash
# Install default domain profiles
prometheos templates install

# List available templates
prometheos templates list

# Show template details
prometheos templates show "Software Development"
```

#### Programmatic Usage

```rust
use prometheos_lite::work::{WorkContextService, types::WorkDomain};
use std::sync::Arc;

let db = Db::new("prometheos.db")?;
let work_context_service = WorkContextService::new(Arc::new(db));

// Create a software development context
let context = work_context_service.create_context(
    "user-123".to_string(),
    "Build REST API".to_string(),
    WorkDomain::Software,
    "Create a REST API for user management".to_string(),
)?;

// Update phase
work_context_service.update_phase(&mut context, WorkPhase::Planning)?;

// Execute a flow in context
let artifact = execution_service
    .execute_flow_in_context(&mut context, "planning.flow.yaml")
    .await?;
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
  /llm        → Model client (OpenAI-compatible, multi-provider)
  /fs         → File parsing & writing
  /logger     → Streaming logs
  /config     → Configuration loader
```

---

## 🔌 Model Support

Compatible with:

- OpenRouter-first routing with mode-aware chains (`fast`, `balanced`, `deep`, `coding`)
- Any OpenAI-compatible endpoint (BYOK)
- Local providers such as LM Studio/Ollama through provider entries
- Automatic quota/rate-limit cooldown failover

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

## 📌 Migration from v1.1

The v1.2 release introduces WorkContext lifecycle management. The v1.1 flow-centric architecture remains fully functional.

### New Features in V1.2

- **WorkContext**: Track work through phases with lifecycle management
- **Autonomy Levels**: Configure Chat, Autonomous, or Review modes
- **Approval Policies**: Control execution with approval requirements
- **Artifact Tracking**: Automatically capture flow outputs
- **Domain Profiles**: Pre-configured templates for different work domains
- **Guardrails**: Built-in safety checks for blocked contexts

### Migration Steps

No breaking changes. V1.2 is fully additive:
1. Existing flows continue to work without modification
2. Optionally integrate WorkContext for lifecycle tracking
3. Use new CLI commands for WorkContext management

See `docs/v1.2-operation.md` for detailed V1.2 documentation.

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
