# PrometheOS Lite

PrometheOS Lite is a local AI workbench for safe autonomous software workflows.

It scans a repository, creates a WorkContext, generates review artifacts, records approval decisions, and remembers the session so work can continue later.

> Alpha status: PrometheOS Lite currently focuses on the Repo Workbench golden path. It is local-first, file-backed, and intentionally conservative: it analyzes and produces artifacts, but does not apply patches automatically. For the current alpha scope, see [Alpha Notes](docs/release/alpha-notes.md).

## What works today

- Install the `prometheos` CLI locally.
- Run `prometheos work ...` against a repository.
- Create a file-backed WorkContext.
- Scan candidate files.
- Detect risky code patterns.
- Generate a risk report artifact.
- Generate a suggested patch plan artifact.
- Record approval decisions.
- Show memory for a WorkContext.
- Continue a previous WorkContext.
- Run the golden path in CI.

## Quick start

Install:

```bash
git clone https://github.com/prometheosai/prometheos-lite.git
cd prometheos-lite
cargo install --path .
prometheos --version
```

First-value command:

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json
```

Then:

```bash
prometheos work run <work_id>
prometheos work artifacts <work_id>
prometheos work memory show <work_id>
prometheos work continue <work_id>
```

See:

- [Install guide](docs/guides/install.md)
- [Zero-to-first-value guide](docs/guides/zero-to-first-value.md)
- [Repo Workbench MVP guide](docs/guides/repo-workbench-mvp.md)
- [Ollama and Ornith compatibility guide](docs/guides/ollama-ornith-compatibility.md)

## Safety model

PrometheOS Lite is intentionally conservative.

- `work run` reads source files and writes artifacts/memory under `.prometheos-lite/workbench/`.
- `work approve` records approval only.
- It does not modify repository source files during analysis.
- It does not apply patches automatically.
- The golden path CI verifies that fixture source files are not modified.

## What this is not yet

PrometheOS Lite is not yet:

- a full autonomous coding agent,
- a patch application system,
- a cloud control plane,
- a plugin marketplace,
- a Brain/Mnemosyne integration,
- a team workspace product,
- a trading/SATI workflow,
- a voice/UI product.

## Architecture direction

PrometheOS Lite is the local-first runtime and product wedge.

The current axis is:

```text
prometheos work
  → Repo Workbench
  → WorkContext
  → artifacts
  → memory
  → continuation
```

Future layers such as Mnemosyne and Brain should plug into this axis rather than creating separate user-facing universes.

PrometheOS Lite is model-agnostic by design. Future local model support should allow coding models served through runtimes such as Ollama to plug into the `prometheos work` axis without changing the safety model. See [Local Model Compatibility](docs/guides/local-model-compatibility.md).

---

## Architecture

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

## Project Structure

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

## Model Support

Compatible with:

- OpenRouter-first routing with mode-aware chains (`fast`, `balanced`, `deep`, `coding`)
- Any OpenAI-compatible endpoint (BYOK)
- Local providers such as LM Studio/Ollama through provider entries
- Automatic quota/rate-limit cooldown failover

---

## Contributing

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

## Documentation Layout

Project documentation is organized under `/docs`:

- `docs/prd` - Product requirements and roadmap docs
- `docs/operations` - Project operations and management setup
- `docs/guides` - Contributor and usage guides
- `docs/research` - Research notes and compatibility analysis
- `docs/architecture` - Technical design references
