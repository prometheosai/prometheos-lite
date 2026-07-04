# Product Surface Inventory

Every identifiable surface in PrometheOS Lite, classified by maturity.

## Status labels

| Label | Meaning |
|---|---|
| **stable alpha** | Documented, tested in CI, part of the alpha golden path |
| **experimental** | Exists and works, but not alpha-promised. May change. |
| **internal** | Infrastructure used by other surfaces. Not user-facing. |
| **deprecated** | Feature-gated, candidates for removal. |
| **future** | Referenced in docs or PRDs but not implemented. |

---

## CLI

| Surface | Status | Notes |
|---|---|---|
| `prometheos work` | stable alpha | Repo Workbench CRUD, artifacts, memory, continue, approve |
| `prometheos repo-workbench` | stable alpha | Alias for `work`; same golden path |
| `prometheos flow` | experimental | Run/resume/replay flows from YAML/JSON |
| `prometheos harness` | experimental | Full harness engine (run, inspect, dry-run, apply, rollback) |
| `prometheos templates` | experimental | Domain template management |
| `prometheos diagnostics` | experimental | Provider/system/validation diagnostics |
| `prometheos serve` | experimental | Start the Axum API server |
| `prometheos bench` | experimental | Benchmark runner |
| `prometheos guardrail` | experimental | Interrupt/trust/outbox management |

## Core systems

| Surface | Status | Notes |
|---|---|---|
| Flow execution engine | experimental | DAG of Nodes with prep/exec/post lifecycle |
| WorkContext lifecycle | experimental | Full create/run/continue/phase/status lifecycle |
| Intent classification | experimental | Routes user messages to direct-LLM or code-gen flows |
| Personality/mode system | experimental | Mode selection, prompt context, constitutional filters |
| Provider routing | stable alpha | OpenRouter-first, configurable chain, tested |
| LLM client (OpenAI-compatible) | stable alpha | Mock-tested HTTP client |
| Artifact provenance | stable alpha | model_invoked, provider, model metadata on artifacts |
| Memory system | experimental | SQLite-backed, embedding/vector search, scoring, summarization |
| Evolution engine | experimental | Domain evolution, playbook learning, A/B testing |
| Artifact generation | experimental | Code, test, plan, research artifact kinds |

## Repo Workbench

| Surface | Status | Notes |
|---|---|---|
| Deterministic static analysis | stable alpha | Tree-sitter-powered, no model required |
| Risk report artifacts | stable alpha | Generated during analysis |
| Patch plan artifacts | stable alpha | Generated during analysis |
| Approval recording | stable alpha | Records decision, does not apply |
| Work continuation | stable alpha | Memory persistence across sessions |

## API server

| Surface | Status | Notes |
|---|---|---|
| REST endpoints | experimental | Conversations, messages, projects, playbooks, work contexts |
| WebSocket streaming | experimental | Real-time flow execution events |
| Health endpoint | stable alpha | Used by CI and smoke tests |
| Request validation | experimental | Server-side input validation |

## Frontend

| Surface | Status | Notes |
|---|---|---|
| Next.js application | experimental | TypeScript, Tailwind, shadcn/ui |
| Conversation UI | experimental | Chat interface with sidebar layout |
| Project pages | experimental | CRUD UI for projects |
| Settings/profile | experimental | User settings and profile management |
| Search command palette | experimental | CMDK-based search |

## Harness engine

| Surface | Status | Notes |
|---|---|---|
| Execution loop | internal | Main harness driver |
| Patch provider | experimental | LLM/heuristic patch generation |
| Patch applier | internal | Rollback-safe patch apply |
| Validation system | internal | Validation with cache, diffing, timeouts |
| Git checkpoint/rollback | experimental | Checkpoint management |
| Confidence scoring | internal | Confidence calibration |
| Risk assessment | internal | Risk engine |
| Selection engine | internal | Best-patch selection |
| Scaling engine | internal | Parallel attempt management |
| Trajectory recording | internal | Execution replay and debugging |
| Time-travel debugger | experimental | Breakpoints, state inspection |
| Regression memory | experimental | Failure pattern learning |
| Repair loop | experimental | Self-repair with retry strategies |
| Sandbox policy | experimental | Isolation policies |
| Multi-tenant isolation | experimental | Per-workspace isolation |
| Plugin system | experimental | Plugin architecture |
| Repo intelligence | experimental | Structure, dependency, doc analysis |
| Benchmark suite | experimental | Benchmark runner with anti-overfitting |
| Trust reporting | experimental | Trust report generation |
| Monitoring dashboard | experimental | Real-time monitoring UI |
| GraphQL API | experimental | Alternate query interface |
| Semantic diff | internal | API, auth, DB, dependency change detection |
| Minimality enforcement | experimental | Patch minimality analysis |

## Infrastructure

| Surface | Status | Notes |
|---|---|---|
| SQLite database layer | internal | 20+ repository modules |
| Configuration loading | internal | JSON/env/defaults |
| Control files system | internal | SOUL.md, SKILLS.md, FLOWS.md, etc. |
| Logger | internal | Structured terminal logging |
| Async job queue | internal | Priority, retry, status tracking |
| Tool permissions | internal | Permission ledger, path guard, trust policies |
| Runtime policy | internal | Domain enforcement, placeholder detection |
| File utilities | internal | File parsing, writing, validation |
| Utilities (IDs, errors, time, etc.) | internal | Shared helpers |
| Idempotency | internal | Idempotency key system |
| Loop detection | internal | Infinite loop prevention |
| Snapshot/resume | internal | Flow state persistence and restore |
| OpenTelemetry integration | internal | Trace export |
| Flow budget guards | internal | Execution budget enforcement |

## CI

| Surface | Status | Notes |
|---|---|---|
| Rust Checks (fmt, clippy, test, build) | stable alpha | Runs on every PR/push to main |
| Repo Workbench golden path | stable alpha | Verifies fixture files unchanged |
| Linux install smoke | stable alpha | Verifies `cargo install` + version |
| Guardrail tests | stable alpha | Policy enforcement tests |
| Anti-placeholder check | stable alpha | Prevents TODO/mock/stub in production code |
| Patch provider diagnostics | stable alpha | Verifies patch provider error handling |
| Fallback confinement | stable alpha | Confines `with_fallback_allowed` to repo tool |
| Anti-regression (no direct LLM in API) | stable alpha | Ensures API goes through FlowExecutionService |
| Provider config validation tests | stable alpha | Config parsing, mode chains, missing keys |
| Mock provider smoke tests | stable alpha | 10 tests for OpenAI-compatible client |

## Documentation

| Surface | Status | Notes |
|---|---|---|
| Install guide | stable alpha | `docs/guides/install.md` |
| Zero-to-first-value guide | stable alpha | `docs/guides/zero-to-first-value.md` |
| Repo Workbench MVP guide | stable alpha | `docs/guides/repo-workbench-mvp.md` |
| Provider configuration guide | stable alpha | `docs/guides/provider-configuration.md` |
| Ollama/Ornith compatibility guide | stable alpha | `docs/guides/ollama-ornith-compatibility.md` |
| Local model compatibility guide | stable alpha | `docs/guides/local-model-compatibility.md` |
| How flows work | experimental | `docs/guides/how-flows-work.md` |
| WorkContext guide | experimental | `docs/guides/workcontext-guide.md` |
| Architecture docs | experimental | Multiple files under `docs/architecture/` |
| PRD documents | experimental | 16 files under `docs/prd/` |
| Release notes | stable alpha | `docs/release/v1.6.1-alpha.1.md` |
| Research notes | experimental | `docs/research/` |
| README | stable alpha | Public-facing front door |

## Deprecated

| Surface | Status | Notes |
|---|---|---|
| Legacy agents (Planner/Coder/Reviewer) | deprecated | Feature-gated behind `legacy` |
| Sequential orchestrator | deprecated | Feature-gated behind `legacy` |
| AgentNode adapter | deprecated | Wraps legacy agents as flow nodes |

## Future

| Surface | Status | Notes |
|---|---|---|
| Brain | future | Referenced in README as not yet implemented |
| Mnemosyne | future | Referenced in README as not yet implemented |
| Plugin marketplace | future | Listed in release notes |
| Cloud/team control plane | future | Listed in release notes |
| UI/voice layer | future | Listed in release notes |
| Benchmark claims | future | Listed as explicitly not included |
| Automatic patch application | future | Listed as explicitly not included |
| Full autonomous coding | future | Listed as explicitly not included |

## Gap analysis

Surfaces that exist in the repo but have no alpha-level documentation or CI:

- Harness engine (60+ modules) — works, tested internally, no user docs
- Frontend (Next.js app) — compiles, no CI, no alpha promise
- API server — runs, has tests, no alpha guide
- Memory system — works, no alpha guide
- Flow engine — works, partial docs
- Plugin system — exists, unreferenced
- Time-travel debugger — exists, unreferenced
- Monitoring dashboard — exists, unreferenced
- GraphQL API — exists, unreferenced

These are not bugs. They are intentionally out of alpha scope. The alpha
delivers the CLI-local Repo Workbench golden path. The rest exists for
future milestones.
