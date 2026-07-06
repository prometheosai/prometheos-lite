# PrometheOS Lite

PrometheOS Lite is a local-first AI workbench for safe autonomous software workflows.

Its stable alpha surface is Repo Workbench: a conservative CLI workflow that scans repositories, generates review artifacts, records approval decisions, preserves memory, and continues work later without modifying source files.

Under the hood, PrometheOS Lite is evolving into a broader runtime for agentic software work, with flow orchestration, provider routing, harness validation, API server surfaces, and frontend experiments.

> Alpha status: the supported alpha path is `prometheos work`. Experimental surfaces exist in the repo but are not yet promised as stable product surfaces. See the [Product Surface Inventory](docs/guides/product-surface-inventory.md) for the full classification.

## Stable alpha surface

- `prometheos work` — Repo Workbench CRUD, artifacts, memory, continue, approve
- Repo Workbench deterministic static analysis (tree-sitter, no model required)
- WorkContext creation, run, and continuation
- Risk report and suggested patch plan artifacts
- Approval recording
- Artifact provenance (model invoked/provider/model metadata)
- Provider routing (OpenRouter-first with mode-aware chains)
- LLM client (OpenAI-compatible, multi-provider)
- Provider configuration docs and tests
- Mock OpenAI-compatible provider integration tests
- Optional ignored/manual local endpoint smoke test
- Linux install smoke CI
- Golden path CI

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
- [Ornith manual validation guide](docs/guides/ornith-manual-validation.md)

## Safety model

PrometheOS Lite is intentionally conservative.

- `work run` reads source files and writes artifacts/memory under `.prometheos-lite/workbench/`.
- `work approve` records approval only.
- It does not modify repository source files during analysis.
- It does not apply patches automatically.
- The golden path CI verifies that fixture source files are not modified.

## Architecture at a glance

```
prometheos work
  → Repo Workbench
  → WorkContext
  → deterministic analysis
  → artifacts + provenance
  → memory
  → continuation
```

PrometheOS Lite:

```
PrometheOS Lite
├── Stable alpha
│   └── Repo Workbench CLI
├── Experimental
│   ├── Flow runtime
│   ├── API server
│   ├── Harness engine
│   ├── Frontend
│   └── Memory system
├── Internal
│   ├── SQLite repositories
│   ├── Provider routing
│   ├── Configuration
│   └── Observability
└── Future
    ├── Brain
    ├── Mnemosyne
    ├── Cloud/team control plane
    └── Full autonomous coding
```

## Experimental surfaces

PrometheOS Lite contains several experimental surfaces that exist in the codebase but are not part of the stable alpha promise:

- **Flow execution engine** — DAG-based node orchestration with prep/exec/post lifecycle, transitions, and shared state
- **API server** (`prometheos serve`) — Axum-based HTTP server
- **Harness engine** — Full harness lifecycle (run, inspect, dry-run, apply, rollback)
- **Frontend** — Web UI surface
- **Memory system** — SQLite-backed with embedding/vector search, scoring, and summarization
- **Additional CLI commands** — `flow`, `harness`, `templates`, `diagnostics`, `bench`, `guardrail`

These surfaces may change significantly. They are not alpha-promised.

## What this is not yet

PrometheOS Lite is not yet:

- a full autonomous coding agent,
- a patch application system,
- a cloud control plane,
- a plugin marketplace,
- a Brain/Mnemosyne integration,
- a team workspace product,
- a trading/SATI workflow,
- a voice/UI product,
- a stable frontend product,
- a benchmark-claiming model product.

## Documentation

- [Install guide](docs/guides/install.md)
- [Zero-to-first-value guide](docs/guides/zero-to-first-value.md)
- [Repo Workbench MVP guide](docs/guides/repo-workbench-mvp.md)
- [Product Surface Inventory](docs/guides/product-surface-inventory.md)
- [Provider configuration guide](docs/guides/provider-configuration.md)
- [Ollama and Ornith compatibility guide](docs/guides/ollama-ornith-compatibility.md)
- [Ornith manual validation guide](docs/guides/ornith-manual-validation.md)
- [`prometheos serve` / API server status](docs/guides/serve-api-status.md)
- [Frontend alpha status](docs/guides/frontend-alpha-status.md)
- [Local frontend demo](docs/guides/local-frontend-demo.md)
- [Model-layer positioning](docs/research/model-layer-positioning.md)
- [Autonomous loop graduation criteria](docs/research/autonomous-loop-graduation-criteria.md)
- Architecture audit: [Provider architecture audit](docs/research/provider-architecture-audit.md)
- [Alpha release notes](docs/release/v1.6.1-alpha.1.md)

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
