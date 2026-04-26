Got it. Based on the updated repo, you are **already past the old “V1 starts from zero” plan**. Tiny miracle. We can stop pretending you still need to invent flows, state, runtime injection, tracing, and run persistence from scratch.

Your current repo already has:

* Rust 2024 backend with `axum`, `tokio`, `clap`, `rusqlite`, `reqwest`, `serde`, `uuid` and friends. 
* Flow-centric architecture exposed from `src/flow`, including `flow`, `node`, `runtime`, `memory`, `policy`, `rate_limit`, `tracing`, `orchestration`, and migration modules. 
* Deprecated legacy mode behind feature flag, meaning the repo has already committed to flow-first architecture instead of old agent-core confusion. 
* CLI entrypoint through `src/main.rs → cli::run()`. 
* CLI `flow` command with JSON flow loading, debug mode, timeline export, runtime creation, model router, tool runtime, and memory service wiring. 
* A `FlowRunner`, `FlowFile`, `NodeFactory`, and concrete nodes like planner, coder, reviewer, LLM, tool, file writer, context loader, memory write, conditional, and passthrough. 
* `SharedState` already split into `input`, `context`, `working`, `output`, and `meta`, with serialization and merge support. This is exactly the state shape V1 needed. 
* Run registry, flow runs, SQLite persistence, flow events, Maestro scheduling, and continuation checkpoints. 

So the new V1 is not “build the engine.”
The new V1 is:

```txt
make the existing engine boringly reliable, YAML-native, testable, bounded, and ready for future agents
```

That is much better. Less glamorous. More real. The universe apologizes.

---

# PrometheOS Lite V1 Core PRD

## Codename

**V1 Core: Deterministic Flow Runtime**

## Objective

Turn the current PrometheOS Lite Rust backend into a production-grade, local-first, flow-native execution core with:

```txt
YAML Flow → RuntimeContext → FlowRunner → Nodes → SharedState → Trace → FinalOutput
```

V1 must not introduce first-class agents, swarm behavior, learning, or meta-optimization. Those are future epics. V1 must make the current flow system stable enough that another LLM or developer can build V2 without rewriting the floorboards, because software floors are apparently made of spaghetti by default.

---

# Current State Assessment

## Already Implemented

| Area               | Current repo status                           | V1 action                                       |
| ------------------ | --------------------------------------------- | ----------------------------------------------- |
| Rust backend       | Exists, single crate, Rust 2024               | Keep                                            |
| CLI                | `run`, `flow`, `serve` commands exist         | Extend                                          |
| Flow engine        | Exists under `src/flow`                       | Harden                                          |
| RuntimeContext     | Exists with model/tool/memory services        | Keep, refine                                    |
| SharedState        | Exists with input/context/working/output/meta | Use as V1 state                                 |
| NodeFactory        | Exists in CLI runner                          | Move/refactor into library core                 |
| Flow JSON loader   | Exists                                        | Add YAML support                                |
| Trace/timeline     | Exists                                        | Standardize event schema                        |
| Run persistence    | Exists via `RunDb`                            | Wire consistently                               |
| Continuation       | Exists via checkpoint files                   | Integrate with run lifecycle                    |
| Memory service     | Exists                                        | Keep minimal V1 integration                     |
| API server         | Exists                                        | Make thinner                                    |
| Personality engine | Not implemented                               | Add minimal V1 only                             |
| ToolRegistry       | Current tool runtime is command-oriented      | Replace/extend later, but add V1-safe interface |

---

# V1 Non-Negotiable Decisions

## 1. Flow is the execution unit

No first-class Agent in V1.

```txt
V1 unit = Flow
V2 unit = Agent selecting Flow
```

The attached analysis is right: agents are just flows with a decision loop for now, and adding full agents too early creates fake abstractions wearing a tiny hat. 

## 2. YAML becomes the canonical flow format

Current CLI says “Run a flow from a JSON file.” That is useful, but V1 should move to:

```txt
.flow.yaml = canonical
.flow.json = supported compatibility format
```

YAML is what humans will actually write. JSON is what machines tolerate because they have no souls.

## 3. V1 includes minimal V3 spine

V1 must include:

* trace IDs
* run/session IDs
* execution budgets
* basic replay/export
* final output contract

Not full harness. Just the skeleton. This matches the “V1 + V3-core together” correction from your analysis. 

## 4. Personality is a lightweight V1 filter, not a “soul engine”

The personality document is useful, but V1 only needs:

* `PersonalityMode`
* text-based mode selector
* prompt modifier
* post-generation constitutional filter

The four-mode model is good: Companion, Navigator, Anchor, Mirror. 

## 5. Tool security is declared now, hardened later

The blind-spots doc is right: per-tool capability security matters, but full per-invocation sandboxing can be V1.1/V3. For V1 Core, define the model and enforce conservative defaults. 

---

# Target Architecture

```txt
src/
  api/
    mod.rs
    server.rs              # thin router only after refactor
    state.rs
    websocket.rs

  cli/
    mod.rs
    runner.rs              # split later
    commands/
      flow.rs
      serve.rs
      run.rs

  config/
    mod.rs

  flow/
    mod.rs
    flow.rs
    node.rs
    types.rs
    runtime.rs
    tracing.rs
    orchestration.rs
    memory.rs
    intelligence.rs
    policy.rs
    rate_limit.rs

    loader/
      mod.rs
      yaml.rs
      json.rs
      validate.rs

    factory/
      mod.rs
      node_factory.rs
      builtin_nodes.rs

    output/
      mod.rs
      final_output.rs
      evaluation.rs

    budget/
      mod.rs
      execution_budget.rs
      budget_guard.rs

    testing/
      mod.rs
      fixtures.rs
      flow_test_runner.rs

  personality/
    mod.rs
    mode.rs
    selector.rs
    constitution.rs
    prompt.rs

  tools/
    mod.rs
    tool.rs
    registry.rs
    permissions.rs
    command_tool.rs

  utils/
    mod.rs
    ids.rs
    json.rs
    time.rs
    errors.rs
    paths.rs
```

Important: you do **not** need a workspace/multi-crate split immediately. Since the repo is currently a single Rust crate, keep V1 single-crate unless the refactor becomes painful. We are not moving furniture during a fire drill.

---

# V1 Core Data Contracts

## Flow file contract

Canonical YAML:

```yaml
version: "1.0"
name: "codegen_basic"
description: "Plan, generate, review, and write code"
start_node: "planner"

inputs:
  required:
    - task

nodes:
  - id: "planner"
    type: "planner"
    config:
      retries: 2
      timeout_ms: 120000

  - id: "coder"
    type: "coder"
    config:
      retries: 1
      timeout_ms: 180000

  - id: "reviewer"
    type: "reviewer"

transitions:
  - from: "planner"
    action: "continue"
    to: "coder"

  - from: "coder"
    action: "continue"
    to: "reviewer"

outputs:
  primary: "generated"
  include:
    - "review"
```

Compatibility JSON can map to the same internal `FlowFile`.

## SharedState

Keep existing:

```rust
pub struct SharedState {
    pub input: HashMap<String, Value>,
    pub context: HashMap<String, Value>,
    pub working: HashMap<String, Value>,
    pub output: HashMap<String, Value>,
    pub meta: HashMap<String, Value>,
}
```

This is already implemented and serializable. Do not replace it. 

Add conventions:

```txt
meta.run_id
meta.trace_id
meta.flow_name
meta.flow_version
meta.started_at
meta.budget
meta.personality_mode
meta.evaluation
```

## FinalOutput

```rust
pub struct FinalOutput {
    pub run_id: String,
    pub trace_id: String,
    pub flow_name: String,
    pub status: FinalStatus,
    pub answer: serde_json::Value,
    pub outputs: serde_json::Value,
    pub evaluation: Evaluation,
    pub budget: BudgetReport,
    pub events_count: usize,
}
```

## Evaluation

```rust
pub struct Evaluation {
    pub success: bool,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
    pub confidence: Option<f32>,
}
```

V1 evaluator is simple:

* output exists
* required outputs exist
* no failed critical node
* budget not exceeded
* no unsafe tool call

No fake “truth system” yet. Humanity survives.

---

# V1 Core Issues

## Issue #1 — Make YAML the canonical Flow format

**Priority:** Critical
**Status:** New
**Depends on:** Current `FlowRunner`, `FlowFile`

### Context

Current `FlowRunner` loads JSON with `from_json_file_with_runtime`.  V1 must support `.flow.yaml` as the canonical developer-facing format.

### Tasks

* Add `serde_yaml` to `Cargo.toml`.
* Create `src/flow/loader/yaml.rs`.
* Support:

  * `.yaml`
  * `.yml`
  * `.json`
* Rename `from_json_file_with_runtime` conceptually to:

  ```rust
  from_file_with_runtime(path, runtime)
  ```
* Keep JSON compatibility.

### Acceptance Criteria

* `prometheos flow path/to/flow.yaml --input '{"task":"..."}'` works.
* JSON flow files still work.
* Invalid YAML returns actionable validation error.
* YAML and JSON map to the same internal `FlowFile`.

---

## Issue #2 — Move NodeFactory out of CLI into core library

**Priority:** Critical
**Status:** New
**Depends on:** Current `src/cli/runner.rs`

### Context

`NodeFactory`, `DefaultNodeFactory`, and many concrete nodes currently live inside CLI runner code. That makes API and future harness reuse awkward. Tiny architecture gremlin, easy to fix now. 

### Target

Move to:

```txt
src/flow/factory/
  mod.rs
  node_factory.rs
  builtin_nodes.rs
```

### Tasks

* Move `NodeFactory` trait to library.
* Move `DefaultNodeFactory` to library.
* Move built-in node implementations out of `src/cli/runner.rs`.
* Keep CLI `FlowRunner` thin.

### Acceptance Criteria

* CLI and API can both build flows from the same factory.
* No built-in node logic remains in CLI.
* Tests compile without legacy feature.

---

## Issue #3 — Introduce FlowLoader abstraction

**Priority:** High
**Status:** New

### Target

```rust
pub trait FlowLoader {
    fn load(&self, path: &Path) -> Result<FlowDefinition>;
}
```

### Tasks

* Create `FlowDefinition` or reuse `FlowFile` with version field.
* Add validation module.
* Return rich errors:

  * missing start node
  * duplicate node ID
  * transition target missing
  * unsupported node type
  * missing required input

### Acceptance Criteria

* Loader handles YAML/JSON.
* Validation runs before build.
* Error messages include file path and field name.

---

## Issue #4 — Standardize trace event schema

**Priority:** Critical
**Status:** Partially exists
**Depends on:** `src/flow/tracing.rs`, `src/flow/orchestration.rs`

### Context

The repo already has tracing/timeline and `flow_events` persistence.  V1 must standardize the event names so later harness/replay/learning can rely on them.

### Required events

```txt
RunStarted
RunCompleted
RunFailed
FlowLoaded
FlowValidationFailed
NodeStarted
NodeCompleted
NodeFailed
TransitionTaken
BudgetChecked
ToolRequested
ToolCompleted
MemoryRead
MemoryWrite
EvaluationCompleted
OutputGenerated
```

### Tasks

* Define `TraceEventKind` enum.
* Stop using arbitrary event type strings internally where possible.
* Persist event kind as stable string.
* Add `trace_id` and `run_id` to all emitted events.

### Acceptance Criteria

* Timeline export contains standardized event kinds.
* SQLite events contain `run_id`, `node_id`, `event_type`, `timestamp`, `data`.
* CLI debug output maps directly to events.

---

## Issue #5 — Add ExecutionBudget and BudgetGuard

**Priority:** Critical
**Status:** New
**Depends on:** Flow execution loop

### Context

The blind-spots doc is right: no budget means an agent/flow can burn tokens and tool calls like a caffeinated raccoon with a credit card. Cost awareness must start in V1. 

### Budget model

```rust
pub struct ExecutionBudget {
    pub max_steps: usize,
    pub max_llm_calls: usize,
    pub max_tool_calls: usize,
    pub max_runtime_ms: u64,
    pub max_memory_reads: usize,
    pub max_memory_writes: usize,
}
```

### Tasks

* Add budget to `SharedState.meta`.
* Add `BudgetGuard`.
* Check budget:

  * before each node
  * before LLM call
  * before tool call
  * before memory operation
* Emit `BudgetChecked`.

### Acceptance Criteria

* Flow stops cleanly when budget exceeded.
* FinalOutput includes budget report.
* CLI supports:

  ```bash
  --max-steps
  --max-llm-calls
  --max-runtime-ms
  ```

---

## Issue #6 — FinalOutput contract

**Priority:** Critical
**Status:** New

### Context

One of the blind spots was “atomic unit of value.” V1 must define exactly what one successful run produces. No more “whatever happens to be in output.” Very artistic, very useless.

### Tasks

* Add `src/flow/output/final_output.rs`.
* Convert `SharedState` to `FinalOutput` after flow run.
* Add `Evaluation`.
* CLI prints FinalOutput in JSON by default with optional pretty mode.

### Acceptance Criteria

* Every successful run returns `FinalOutput`.
* Every failed run returns structured error output.
* `trace_id` always present.
* Required output keys can be validated from flow definition.

---

## Issue #7 — Basic Evaluation Engine

**Priority:** High
**Status:** New

### Tasks

* Add `src/flow/output/evaluation.rs`.
* Evaluate:

  * flow completed
  * required outputs exist
  * no critical node errors
  * budget status
  * unsafe/skipped tool calls
* Store evaluation in `SharedState.meta.evaluation`.

### Acceptance Criteria

* Evaluation always runs.
* FinalOutput includes evaluation.
* Failed evaluation does not panic.

---

## Issue #8 — Run persistence and continuation integration

**Priority:** High
**Status:** Partially exists
**Depends on:** `RunDb`, `ContinuationEngine`

### Context

Run persistence and checkpointing exist but are not fully unified with CLI/API execution. 

### Tasks

* Ensure every CLI/API run creates a `FlowRun`.
* Save state snapshot on:

  * completion
  * failure
  * pause
  * budget exceeded
* Add `prometheos flow resume <run_id>`.
* Add `prometheos flow events <run_id>`.

### Acceptance Criteria

* Run can be inspected after execution.
* Checkpoint can be loaded.
* Resume works for deterministic flows.
* If resume is unsupported for a node type, error clearly says so.

---

## Issue #9 — Minimal replay/export

**Priority:** High
**Status:** Partial via timeline export

### Tasks

* Keep current `--export-timeline`.
* Add:

  ```bash
  prometheos flow replay <run_id>
  ```
* V1 replay can be **observational**, not re-executing tools.
* Print:

  * events
  * node order
  * outputs
  * final evaluation

### Acceptance Criteria

* Replay shows exact event sequence.
* Replay does not require LLM or tool calls.
* Replay works after app restart.

---

## Issue #10 — Minimal Personality Engine

**Priority:** Medium
**Status:** New
**Depends on:** Prompt construction nodes

### Context

The personality architecture doc is good, but V1 should implement only the practical slice: mode selection and constitutional filter. 

### Modules

```txt
src/personality/
  mod.rs
  mode.rs
  selector.rs
  constitution.rs
  prompt.rs
```

### Modes

```rust
pub enum PersonalityMode {
    Companion,
    Navigator,
    Anchor,
    Mirror,
}
```

### Selection rules

* Companion: casual/general
* Navigator: building/planning/executing
* Anchor: overwhelmed/anxious/confused
* Mirror: asks for critique/assessment

### Tasks

* Text-based selector only.
* Inject mode into prompt context for LLM nodes.
* Add post-generation filter:

  * shorten excessive output
  * remove false certainty
  * require gentle tone for Anchor
  * require directness for Mirror

### Acceptance Criteria

* Mode stored in `SharedState.meta.personality_mode`.
* CLI debug output shows selected mode.
* Does not slow response intentionally. The doc correctly says fake typing cadence is cheap theater. 

---

## Issue #11 — Tool permission model v0

**Priority:** High
**Status:** Current tool runtime exists but is command-focused

### Context

`ToolRuntime` currently executes allowed commands with a sandbox profile. That is useful but not enough for the future ToolRegistry/MCP world. V1 should introduce permission vocabulary without building every adapter yet.

### Model

```rust
pub enum ToolPermission {
    Network,
    FileRead,
    FileWrite,
    Shell,
    Env,
}

pub struct ToolPolicy {
    pub allowed_permissions: Vec<ToolPermission>,
    pub require_approval: bool,
}
```

### Tasks

* Add permissions module under `src/tools`.
* Wrap current `ToolRuntime` as `CommandToolRuntime`.
* Add conservative defaults:

  * shell disabled unless explicit
  * file writes restricted to `prometheos-output/`
  * network denied unless explicit

### Acceptance Criteria

* Tool calls emit permission info.
* Unsafe tool call returns structured denial.
* Existing `tool` node still works for allowed commands.

---

## Issue #12 — Tool schema hash placeholder

**Priority:** Medium
**Status:** New

### Context

Tool/schema versioning is a known future landmine. The blind-spots doc correctly points out imported MCP/OpenAPI tool schemas can change remotely. 

### V1 scope

No MCP import yet. Just add the field:

```rust
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub schema_hash: Option<String>,
}
```

### Acceptance Criteria

* Native tools can expose metadata.
* Future imported tools can store schema hash.
* No full versioning system required yet.

---

## Issue #13 — Flow test runner

**Priority:** High
**Status:** New

### Context

YAML flows become a mini programming language, whether anyone admits it or not. The blind-spots doc is correct: without testing, flows become bash scripts from hell. 

### CLI

```bash
prometheos flow test examples/codegen_basic.flow.yaml \
  --fixture tests/fixtures/codegen_basic.input.json \
  --expect tests/fixtures/codegen_basic.expected.json
```

### Tasks

* Add `src/flow/testing`.
* Support fixtures:

  * input JSON
  * expected output JSON
  * expected event kinds
* Mock LLM mode:

  * deterministic responses for planner/coder/reviewer

### Acceptance Criteria

* Flow tests run without real LLM.
* CI can run flow tests.
* Failure output identifies node/step mismatch.

---

## Issue #14 — Benchmark baseline

**Priority:** High
**Status:** New

### Context

Without benchmarks, V5 “learning” later is just vibes with a spreadsheet. The blind-spots doc is painfully correct here. 

### Metrics

```txt
task_success_rate
median_runtime_ms
llm_calls_per_run
tool_calls_per_run
budget_exceeded_rate
flow_failure_rate
```

### Tasks

* Add `prometheos bench run`.
* Add `benchmarks/`.
* Add baseline tasks:

  * direct chat
  * planning flow
  * codegen flow
  * memory read/write flow
* Output JSON report.

### Acceptance Criteria

* Benchmarks run locally.
* Report includes metrics above.
* V1 baseline committed to repo.

---

## Issue #15 — API thin-layer refactor

**Priority:** Critical
**Status:** Partially needed

### Context

Earlier analysis found `server.rs` was doing too much. If you’ve reduced it, good. If not, this remains critical. API must call the same flow runtime as CLI.

### Target

```txt
HTTP request
 → parse input
 → create run
 → execute FlowRunner/Maestro
 → stream events
 → return FinalOutput
```

### Tasks

* Move orchestration out of API routes.
* API should not manually call LLM for planner/coder/reviewer.
* API should not build prompts directly.
* API should only select/load flow and execute.

### Acceptance Criteria

* No direct `LlmClient::generate` in API route handlers.
* WebSocket events mirror trace events.
* API and CLI produce same FinalOutput contract.

---

## Issue #16 — CLI cleanup

**Priority:** Medium
**Status:** Needed

### Context

`src/cli/mod.rs` currently wires config, runtime, memory, tool runtime, server, and command logic. Functional, but it will bloat faster than a startup pitch deck. 

### Target

```txt
src/cli/
  mod.rs
  commands/
    flow.rs
    serve.rs
    run.rs
  runtime_builder.rs
```

### Acceptance Criteria

* Runtime construction reused by `flow` and `serve`.
* CLI command files under ~150 lines where realistic.
* No duplicated RuntimeContext setup.

---

## Issue #17 — Docs: V1 architecture and developer guide

**Priority:** High

### Docs

```txt
docs/v1-core.md
docs/flows-yaml.md
docs/runtime-context.md
docs/tracing-and-replay.md
docs/personality-v1.md
docs/tool-permissions.md
```

### Acceptance Criteria

* Another LLM can read docs and implement V1 without needing this conversation.
* Include examples.
* Include non-goals.

---

# Required Dependencies to Add

```toml
serde_yaml = "0.9"
thiserror = "1.0"
jsonschema = "0.18" # optional for flow/tool schema validation
```

Potentially later:

```toml
tracing-opentelemetry = "0.x"
opentelemetry = "0.x"
```

Do not add OpenTelemetry in V1 unless you enjoy feeding complexity after midnight.

---

# V1 Core Definition of Done

V1 is done when:

```txt
1. YAML flows run from CLI
2. JSON flows still run
3. Flow building logic lives outside CLI
4. API and CLI use same runtime path
5. Every run has run_id + trace_id
6. Every run emits standard events
7. Every run returns FinalOutput
8. Budget limits are enforced
9. Timeline/replay works observationally
10. Flow tests run in CI
11. Benchmarks produce baseline JSON
12. Minimal personality mode selector works
13. Tool permission model exists with conservative defaults
```

---

# V1.1 Guardrails Scope

V1.1 is not a feature party. It is the “you forgot the seatbelts” release.

## V1.1 Epics

### V1.1 Epic A — Per-invocation sandbox hardening

From blind-spots doc: Deno permissions are process-wide, so long-term per-tool isolation matters. 

Scope:

* one subprocess per risky tool invocation
* dynamic permission narrowing
* symlink/path escape checks
* file read/write allowlist

### V1.1 Epic B — Flow versioning

Scope:

* store exact flow definition per run
* resume uses stored flow version
* flow file has `version`
* reject incompatible resume

### V1.1 Epic C — Better interrupt handling

Scope:

* interrupt context schema
* timeout
* invalid decision validation
* no escalation chain yet

### V1.1 Epic D — Trust policy stub

Scope:

* mark external tools as trusted/untrusted
* untrusted tool requires approval
* no marketplace governance yet

---

# V2 Scope — Agent-Native Runtime

V2 begins only after V1 Core is stable.

## V2 Goal

```txt
Flow → Skill → Agent
```

## V2 Adds

* `Skill` abstraction:

  ```rust
  Skill = named Flow capability
  ```
* `AgentProfile`
* `AgentExecutor`
* `AgentRegistry`
* allowed skills
* allowed tools
* memory scope
* model policy

## V2 Non-goals

* no swarm
* no learning
* no autonomous agent creation

## V2 Critical Rule

Agent selects a skill. Agent does not manually execute business logic.

```txt
API → AgentExecutor → Agent → Skill → FlowRunner
```

---

# V3 Scope — Harness Control System

V3 is where the system becomes trustworthy instead of merely functional.

## V3 Adds

* `HarnessRunner`
* `HarnessSession`
* approval gates
* advanced budgets
* replay engine
* outbox/idempotency for side-effect tools
* loop detection
* OpenTelemetry spans
* cost model per LLM/tool call

## Why V3 matters

Your analysis correctly says harness is the secret weapon. I’d go further: it is the system’s adult supervision. Without it, swarm becomes a committee of caffeinated interns. 

---

# V4 Scope — Swarm-Native Runtime

## V4 Adds

* `SwarmRunner`
* `CoordinatorAgent`
* `SwarmPlan`
* `SwarmTask`
* task dependency graph
* parallel scheduling
* consensus engine
* inter-agent message bus

## V4 Rule

No swarm execution outside Harness.

```txt
Harness → SwarmRunner → AgentExecutor → Skill → Flow
```

---

# V5 Scope — Learning Layer

## V5 Adds

* experience store
* reflection engine
* extracted heuristics
* adaptive context injection
* prompt improvement proposals
* metrics-driven routing

## V5 Rule

No silent mutation.

Learning can suggest. Humans or governance approve.

---

# V6 Scope — Meta-Optimization

## V6 Adds

* system intelligence graph
* optimization proposals
* simulation engine
* flow/agent/model strategy optimizer
* rollback/audit

## V6 Rule

The system may propose architecture changes, but it may not rewrite itself without approval.

---

# V7 Scope — Persistent Identity

## V7 Adds

* long-term self-model
* behavioral consistency
* strategy profile
* identity memory
* stable interaction style over time

## V7 Relation to Personality V1

V1 personality is mode selection and filtering.
V7 identity is persistent behavioral continuity.

Do not confuse them. One is a jacket. The other is a spine.

---

# Current State → Updated Priority Order

Given the repo is already ahead, your next order is:

```txt
1. YAML loader
2. Move NodeFactory/built-in nodes from CLI to flow core
3. Standardize trace event schema
4. Add FinalOutput + Evaluation
5. Add ExecutionBudget
6. Wire RunDb/Continuation into CLI/API path
7. Add flow test runner
8. Add benchmark baseline
9. Add minimal personality mode selector
10. Refactor API/CLI thin layers
```

---

# One-line implementation instruction for another LLM

```txt
Implement PrometheOS Lite V1 Core by hardening the existing Rust flow-centric architecture: add YAML flow loading, move node factory and built-in nodes from CLI into flow core, standardize trace events, enforce execution budgets, return a FinalOutput contract with evaluation, integrate run persistence/replay, add flow testing and benchmark baselines, implement minimal personality mode selection, and keep agents/swarm/learning out of V1 except as future-facing data model stubs.
```

That’s the path. Not the fantasy path. The actual one.

Build this, and V2 stops being a conceptual leap. It becomes a small, satisfying refactor. Which, in software, is basically a religious experience.
