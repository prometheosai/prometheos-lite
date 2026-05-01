# PRD V1.5.1 — Proof & Hardening

Status: **Implementation-ready**
Scope: **Post-V1.5 verification, hardening, and exposure layer**
Purpose: turn V1.5 from “implemented in code” into “proven under execution,” because apparently code existing is not the same as code being trustworthy. Shocking development.

V1.5 introduced the stabilization/control layer:

```text
ContextBudgeter
Memory pruning / summarization
Evaluation signals
Observability / tracing
Execution metadata
```

V1.5.1 does **not** add major new product capabilities. It hardens the V1.5 layer by proving that these systems are actually wired into runtime execution paths, not sitting in isolated modules like museum exhibits.

---

# 1. Product Goal

## V1.5.1 Goal

Prove, persist, and expose the operational control layer introduced in V1.5.

Only four goals:

```text
1. Add integration tests proving context budget enforcement.
2. Add integration tests proving memory pruning/summarization is actually used during execution.
3. Persist and expose V1.5 execution metadata through WorkContext/API/CLI.
4. Prove trace propagation across orchestrator → flow → node → memory → LLM/tool.
```

## Non-Goals

V1.5.1 must **not** become another feature swamp.

Do **not** add:

```text
- new agent types
- new swarm behavior
- new UI dashboard
- new skill evolution logic
- MCP/A2A expansion
- vector DB migration
- async database refactor
- new model providers
- new memory architecture
```

Those belong later. This version is about proof, wiring, test coverage, and metadata exposure.

---

# 2. Architecture Context

PrometheOS Lite is currently evolving toward an open-source multi-agent operating layer. The current core path is roughly:

```text
User Intent
  → WorkOrchestrator
  → WorkContext
  → Flow Runtime
  → Node Execution
  → Memory Retrieval / Context Assembly
  → Context Budgeting
  → LLM / Tool Execution
  → Artifacts / Evaluation / Metadata
  → WorkContext Persistence
  → API / CLI Exposure
```

V1.5 added control mechanisms around that path.

V1.5.1 must prove the control mechanisms are not optional sidecars.

The system must demonstrate:

```text
No oversized context reaches model execution.
No memory injection bypasses pruning/budgeting.
No evaluation metadata disappears after execution.
No trace chain breaks between orchestrator and lower execution layers.
```

In normal human terms: no invisible magic, no “trust me bro,” no haunted execution path.

---

# 3. Required Runtime Invariants

These invariants must hold after V1.5.1.

## INV-1: All LLM-bound context must pass through budget enforcement

Any path that builds prompt/context for an LLM call must call the budgeter.

Acceptable:

```text
Memory → ContextAssembler → ContextBudgeter → ModelProvider
```

Unacceptable:

```text
Memory → prompt string → ModelProvider
```

## INV-2: ContextBudgeter must produce inspectable metadata

Every budget operation must produce metadata like:

```rust
pub struct ContextBudgetMetadata {
    pub max_tokens: usize,
    pub estimated_input_tokens_before: usize,
    pub estimated_input_tokens_after: usize,
    pub dropped_sections: Vec<DroppedContextSection>,
    pub summarized_sections: Vec<SummarizedContextSection>,
    pub preserved_sections: Vec<String>,
    pub budget_strategy: String,
}
```

Names may differ, but the information must exist.

## INV-3: Memory pruning/summarization must be used during real execution

It is not enough to test the pruning module directly.

The integration test must prove:

```text
WorkContext execution
  → retrieves oversized memory/context
  → prunes and/or summarizes it
  → continues execution under budget
  → persists metadata showing this happened
```

## INV-4: Execution metadata must survive persistence

After execution, metadata must be readable from persisted `WorkContext`.

This metadata must be visible through:

```text
API
CLI
Repository/persistence layer
```

## INV-5: Trace context must flow end-to-end

Trace propagation must cover:

```text
WorkOrchestrator
  → FlowExecution
  → NodeExecution
  → Memory layer
  → LLM/tool call
```

The trace does not need full Jaeger/Grafana deployment in V1.5.1, but the system must produce structured trace/span identifiers and prove propagation in tests.

---

# 4. Data Model Requirements

## 4.1 WorkContext Execution Metadata

Add or harden this field if already present:

```rust
pub struct WorkContext {
    // existing fields...

    pub execution_metadata: Option<ExecutionMetadata>,
}
```

Recommended shape:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    pub last_run_id: Option<String>,
    pub last_trace_id: Option<String>,

    pub context_budget: Option<ContextBudgetMetadata>,
    pub memory_control: Option<MemoryControlMetadata>,
    pub evaluation: Option<EvaluationMetadata>,
    pub observability: Option<ObservabilityMetadata>,

    pub updated_at: DateTime<Utc>,
}
```

## 4.2 Context Budget Metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudgetMetadata {
    pub max_tokens: usize,
    pub estimated_tokens_before: usize,
    pub estimated_tokens_after: usize,

    pub sections_before: usize,
    pub sections_after: usize,

    pub dropped_sections: Vec<DroppedContextSection>,
    pub summarized_sections: Vec<SummarizedContextSection>,

    pub enforcement_result: ContextBudgetResult,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextBudgetResult {
    WithinBudget,
    ReducedToBudget,
    FailedToFit,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroppedContextSection {
    pub section_id: String,
    pub section_kind: String,
    pub estimated_tokens: usize,
    pub reason: String,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizedContextSection {
    pub section_id: String,
    pub section_kind: String,
    pub original_estimated_tokens: usize,
    pub summarized_estimated_tokens: usize,
    pub strategy: String,
}
```

## 4.3 Memory Control Metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryControlMetadata {
    pub memories_retrieved: usize,
    pub memories_included: usize,
    pub memories_pruned: usize,
    pub memories_summarized: usize,

    pub pruning_strategy: String,
    pub summarization_strategy: Option<String>,

    pub pruned_memory_ids: Vec<String>,
    pub summarized_memory_ids: Vec<String>,
}
```

## 4.4 Evaluation Metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationMetadata {
    pub success_score: Option<f32>,
    pub confidence_score: Option<f32>,

    pub signals: Vec<EvaluationSignalRecord>,

    pub revision_count: usize,
    pub failure_reason: Option<String>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationSignalRecord {
    pub signal_type: String,
    pub severity: String,
    pub message: String,
    pub source: String,
    pub timestamp: DateTime<Utc>,
}
```

## 4.5 Observability Metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityMetadata {
    pub trace_id: String,
    pub root_span_id: String,

    pub span_count: usize,
    pub spans: Vec<SpanSummary>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanSummary {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub layer: String,
    pub status: String,
}
```

Recommended span layers:

```text
orchestrator
flow
node
memory
context_budget
llm
tool
artifact
evaluation
```

---

# 5. API Requirements

## 5.1 WorkContext Response Must Include Execution Metadata

Existing endpoint:

```http
GET /work-contexts/:id
```

Must include:

```json
{
  "id": "ctx_123",
  "goal": "...",
  "status": "in_progress",
  "phase": "execution",
  "execution_metadata": {
    "last_run_id": "run_123",
    "last_trace_id": "trace_abc",
    "context_budget": {
      "max_tokens": 4096,
      "estimated_tokens_before": 9130,
      "estimated_tokens_after": 3820,
      "enforcement_result": "ReducedToBudget"
    },
    "memory_control": {
      "memories_retrieved": 30,
      "memories_included": 8,
      "memories_pruned": 18,
      "memories_summarized": 4
    },
    "observability": {
      "trace_id": "trace_abc",
      "span_count": 7
    }
  }
}
```

## 5.2 Add Optional Metadata-Focused Endpoint

Recommended:

```http
GET /work-contexts/:id/metadata
```

Returns only execution metadata.

Purpose: keep the normal context response usable while allowing deeper debugging.

Response:

```json
{
  "work_context_id": "ctx_123",
  "execution_metadata": {
    "context_budget": {},
    "memory_control": {},
    "evaluation": {},
    "observability": {}
  }
}
```

## 5.3 API Error Rules

If metadata is missing:

```http
200 OK
```

```json
{
  "work_context_id": "ctx_123",
  "execution_metadata": null
}
```

Do **not** fail the request just because a context has never executed. That would be silly, therefore statistically likely unless explicitly forbidden.

---

# 6. CLI Requirements

## 6.1 Add `work metadata`

Command:

```bash
prometheos work metadata <context_id>
```

Output:

```text
WorkContext Metadata
ID: ctx_123
Last Run: run_123
Trace ID: trace_abc

Context Budget
Before: 9130 tokens
After: 3820 tokens
Max: 4096 tokens
Result: ReducedToBudget
Dropped Sections: 12
Summarized Sections: 4

Memory Control
Retrieved: 30
Included: 8
Pruned: 18
Summarized: 4

Observability
Root Span: span_root
Span Count: 7
Layers: orchestrator → flow → node → memory → llm
```

## 6.2 Add `--metadata` flag to `work show`

Command:

```bash
prometheos work show <context_id> --metadata
```

Must display the normal context plus execution metadata.

## 6.3 CLI Missing Metadata Behavior

If no metadata exists:

```text
No execution metadata available for this WorkContext yet.
```

No panic. No stack trace. No theatrical Rust explosion.

---

# 7. Test Strategy

V1.5.1 is mostly test-driven hardening.

Required test categories:

```text
Integration tests
Repository/persistence tests
API tests
CLI tests
Trace propagation tests
Negative/failure tests
```

---

# 8. GitHub Issues

## Epic 1 — Prove Context Budget Enforcement

### Issue V1.5.1-001 — Add End-to-End Context Budget Integration Test

Labels:

```text
v1.5.1, testing, context-control, high-priority
```

Type:

```text
Integration Test
```

### Problem

V1.5 added context budget control, but the system must prove that oversized execution context is reduced before reaching model execution.

Without this, the ContextBudgeter may exist as a decorative object. Very pretty. Completely useless.

### Requirements

Create an integration test that:

```text
1. Creates a WorkContext.
2. Injects or simulates oversized memory/context.
3. Runs a flow/node that would invoke an LLM/model provider.
4. Forces context assembly above the configured max token budget.
5. Verifies ContextBudgeter runs.
6. Verifies model/provider receives context under budget.
7. Verifies budget metadata is attached to WorkContext execution_metadata.
```

### Suggested Test File

```text
tests/context_budget_integration_test.rs
```

### Required Test

```rust
#[tokio::test]
async fn test_oversized_context_is_budgeted_before_llm_call() {
    // arrange
    // create WorkContext
    // inject oversized memory/context
    // configure low max token budget

    // act
    // execute flow through WorkOrchestrator

    // assert
    // model input token estimate <= max budget
    // execution_metadata.context_budget exists
    // estimated_tokens_before > max budget
    // estimated_tokens_after <= max budget
    // enforcement_result == ReducedToBudget
}
```

### Acceptance Criteria

* Test fails if ContextBudgeter is not called.
* Test fails if final LLM-bound context exceeds max budget.
* Test fails if metadata is not persisted.
* Test uses real orchestration path, not direct unit calls only.
* No fake “manual call budgeter in test” nonsense.

### Definition of Done

```text
cargo test test_oversized_context_is_budgeted_before_llm_call
```

passes.

---

### Issue V1.5.1-002 — Add Negative Test for Impossible Budget

Labels:

```text
v1.5.1, testing, context-control, negative-test
```

Type:

```text
Integration Test
```

### Problem

The system must fail safely when the minimum required context cannot fit into the max budget.

### Requirements

Create a test where:

```text
1. System prompt + required task context already exceed the max budget.
2. ContextBudgeter cannot reduce below max safely.
3. Execution fails with structured error.
4. WorkContext metadata records FailedToFit.
```

### Expected Error

Use a typed error like:

```rust
ContextBudgetError::CannotFitRequiredContext
```

or equivalent existing project error type.

### Acceptance Criteria

* No panic.
* No silent truncation of required context.
* No LLM call is made.
* Metadata records budget failure.

### Definition of Done

```text
cargo test test_context_budget_fails_safely_when_required_context_cannot_fit
```

passes.

---

### Issue V1.5.1-003 — Add Model Invocation Guard Against Budget Bypass

Labels:

```text
v1.5.1, architecture, testing, context-control
```

Type:

```text
Runtime Guard
```

### Problem

Even if the main execution path uses the budgeter, future developers may add model calls that bypass it. Because apparently entropy has a GitHub account.

### Requirements

Add a guard around model/LLM invocation requiring budget metadata.

Recommended approach:

```rust
pub struct BudgetedModelRequest {
    pub messages: Vec<ModelMessage>,
    pub budget_metadata: ContextBudgetMetadata,
}
```

The model provider should accept budgeted requests, not raw prompt strings, wherever feasible.

If full refactor is too intrusive, add a runtime assertion:

```rust
if request.budget_metadata.is_none() {
    return Err(ModelError::MissingContextBudgetMetadata);
}
```

### Acceptance Criteria

* LLM/model execution path cannot bypass budget enforcement.
* Tests prove missing budget metadata blocks execution.
* Existing flows still pass.

### Definition of Done

```text
cargo test model_requires_budgeted_context
```

passes.

---

## Epic 2 — Prove Memory Pruning / Summarization Is Used During Execution

### Issue V1.5.1-004 — Add End-to-End Memory Pruning Integration Test

Labels:

```text
v1.5.1, memory, testing, high-priority
```

Type:

```text
Integration Test
```

### Problem

Memory pruning must be proven inside real execution, not just tested as an isolated function.

### Requirements

Create an integration test that:

```text
1. Creates a WorkContext.
2. Inserts many memories into memory storage.
3. Marks some memories high relevance/importance.
4. Marks others stale/low relevance.
5. Runs a flow requiring memory retrieval.
6. Verifies memory pruning runs before context injection.
7. Verifies high-priority memories survive.
8. Verifies low-priority memories are pruned.
9. Verifies WorkContext metadata records pruning.
```

### Suggested Test File

```text
tests/memory_control_integration_test.rs
```

### Required Test

```rust
#[tokio::test]
async fn test_memory_pruning_runs_during_work_context_execution() {
    // arrange
    // create context
    // insert high-priority and low-priority memories
    // configure small memory/context budget

    // act
    // run orchestrated flow

    // assert
    // included memory ids contain important memories
    // pruned memory ids contain low-priority memories
    // metadata.memory_control.memories_pruned > 0
}
```

### Acceptance Criteria

* Test uses WorkOrchestrator or actual flow runtime.
* Test fails if memory pruning is not called.
* Test proves priority-aware pruning.
* Metadata is persisted.

### Definition of Done

```text
cargo test test_memory_pruning_runs_during_work_context_execution
```

passes.

---

### Issue V1.5.1-005 — Add End-to-End Memory Summarization Integration Test

Labels:

```text
v1.5.1, memory, summarization, testing
```

Type:

```text
Integration Test
```

### Problem

Summarization must be proven as part of execution when memory is too large but still relevant.

### Requirements

Create an integration test where:

```text
1. A long but relevant memory is retrieved.
2. It exceeds the memory/context section budget.
3. The system summarizes it instead of dropping it.
4. The summarized version is injected into context.
5. Metadata records summarization.
```

### Required Test

```rust
#[tokio::test]
async fn test_relevant_large_memory_is_summarized_not_dropped() {
    // arrange
    // insert large relevant memory
    // configure budget that requires compression

    // act
    // run flow

    // assert
    // memory id appears in summarized_memory_ids
    // summarized token estimate < original token estimate
    // memory not listed as dropped
}
```

### Acceptance Criteria

* Relevant memory is summarized, not pruned.
* Summary metadata includes original and summarized token estimate.
* Execution continues under budget.
* No raw oversized memory reaches model provider.

### Definition of Done

```text
cargo test test_relevant_large_memory_is_summarized_not_dropped
```

passes.

---

### Issue V1.5.1-006 — Add Failure Test for Summarizer Failure

Labels:

```text
v1.5.1, memory, negative-test
```

Type:

```text
Failure Test
```

### Problem

If summarization fails, the system must not silently inject oversized memory.

### Requirements

Create a test where summarizer fails intentionally.

Expected behavior:

Either:

```text
A. fallback to pruning if safe
```

or:

```text
B. fail execution with structured error
```

But it must not:

```text
- silently continue with oversized context
- drop required context without metadata
- panic
```

### Acceptance Criteria

* Summarizer failure is captured.
* Execution path is deterministic.
* Metadata records summarization failure or fallback.
* No oversized context reaches LLM/model.

### Definition of Done

```text
cargo test test_summarizer_failure_does_not_bypass_budget
```

passes.

---

## Epic 3 — Persist and Expose V1.5 Execution Metadata

### Issue V1.5.1-007 — Persist ExecutionMetadata on WorkContext After Flow Execution

Labels:

```text
v1.5.1, persistence, work-context, high-priority
```

Type:

```text
Backend
```

### Problem

V1.5 metadata is useless if it dies after execution. That would be telemetry cosplay.

### Requirements

After every orchestrated execution, persist:

```text
context_budget metadata
memory_control metadata
evaluation metadata
observability metadata
last_run_id
last_trace_id
updated_at
```

### Required Persistence Behavior

When flow execution completes:

```text
WorkOrchestrator
  → receives ExecutionReport / FlowResult
  → extracts V1.5 metadata
  → updates WorkContext.execution_metadata
  → persists WorkContext
```

When flow execution fails:

```text
WorkOrchestrator
  → records partial metadata
  → records failure reason
  → persists metadata before returning error
```

### Acceptance Criteria

* Metadata persists on success.
* Metadata persists on failure where possible.
* Reloading WorkContext returns same metadata.
* Existing WorkContext tests still pass.

### Definition of Done

```text
cargo test test_execution_metadata_persisted_after_success
cargo test test_execution_metadata_persisted_after_failure
```

pass.

---

### Issue V1.5.1-008 — Add Repository Tests for ExecutionMetadata Roundtrip

Labels:

```text
v1.5.1, persistence, testing
```

Type:

```text
Repository Test
```

### Problem

Serialization/persistence must be proven independently.

### Requirements

Create repository-level test:

```text
1. Create WorkContext with full ExecutionMetadata.
2. Save it.
3. Reload it.
4. Assert all nested metadata survives.
```

### Required Test

```rust
#[tokio::test]
async fn test_work_context_execution_metadata_roundtrip() {
    // arrange
    // create metadata with context_budget, memory_control, evaluation, observability

    // act
    // persist and reload

    // assert
    // metadata is deeply equal
}
```

### Acceptance Criteria

* Nested structs serialize/deserialize correctly.
* No metadata fields are silently dropped.
* Works with SQLite/current persistence layer.

### Definition of Done

```text
cargo test test_work_context_execution_metadata_roundtrip
```

passes.

---

### Issue V1.5.1-009 — Expose Execution Metadata in API WorkContext Response

Labels:

```text
v1.5.1, api, metadata
```

Type:

```text
Backend API
```

### Problem

Execution metadata must be visible to API consumers.

### Requirements

Update:

```text
GET /work-contexts/:id
```

to include:

```text
execution_metadata
```

Update response DTOs.

If `WorkContextResponse` already exists, add:

```rust
pub execution_metadata: Option<ExecutionMetadataResponse>
```

or use the shared serializable type if acceptable.

### Acceptance Criteria

* API response includes metadata when present.
* API response returns `execution_metadata: null` when absent.
* No breaking response panic when older contexts lack metadata.
* API integration test added.

### Required Test

```rust
#[tokio::test]
async fn test_get_work_context_includes_execution_metadata() {}
```

### Definition of Done

```text
cargo test test_get_work_context_includes_execution_metadata
```

passes.

---

### Issue V1.5.1-010 — Add API Endpoint: GET /work-contexts/:id/metadata

Labels:

```text
v1.5.1, api, metadata, debugging
```

Type:

```text
Backend API
```

### Problem

Debugging tools should retrieve metadata without fetching the entire WorkContext.

### Endpoint

```http
GET /work-contexts/:id/metadata
```

### Response

```json
{
  "work_context_id": "ctx_123",
  "execution_metadata": {
    "last_run_id": "run_123",
    "last_trace_id": "trace_abc",
    "context_budget": {},
    "memory_control": {},
    "evaluation": {},
    "observability": {}
  }
}
```

### Acceptance Criteria

* Returns metadata if present.
* Returns `execution_metadata: null` if absent.
* Returns 404 if WorkContext does not exist.
* API route wired into router.

### Required Test

```rust
#[tokio::test]
async fn test_get_work_context_metadata_endpoint() {}
```

### Definition of Done

```text
cargo test test_get_work_context_metadata_endpoint
```

passes.

---

### Issue V1.5.1-011 — Add CLI Command: work metadata

Labels:

```text
v1.5.1, cli, metadata
```

Type:

```text
CLI
```

### Problem

Developers need to inspect V1.5 metadata from CLI.

### Command

```bash
prometheos work metadata <context_id>
```

### Output Must Include

```text
- WorkContext ID
- last run ID
- trace ID
- context budget before/after/max
- budget result
- memory retrieved/included/pruned/summarized
- evaluation score/signals summary
- trace span count and layers
```

### Acceptance Criteria

* Command works for context with metadata.
* Command works for context without metadata.
* Missing context returns clean error.
* Output is human-readable.

### Required Test

If CLI integration tests exist:

```rust
#[test]
fn test_cli_work_metadata_displays_execution_metadata() {}
```

Otherwise add a command handler test.

### Definition of Done

```text
cargo test test_cli_work_metadata_displays_execution_metadata
```

passes, or equivalent handler test passes.

---

### Issue V1.5.1-012 — Add CLI Flag: work show --metadata

Labels:

```text
v1.5.1, cli, metadata
```

Type:

```text
CLI
```

### Problem

`work show` should optionally display metadata without requiring a separate command.

### Command

```bash
prometheos work show <context_id> --metadata
```

### Acceptance Criteria

* Existing `work show` behavior unchanged by default.
* `--metadata` prints execution metadata section.
* Missing metadata prints clean message.
* No duplicated output logic; share formatter with `work metadata`.

### Definition of Done

```text
cargo test test_cli_work_show_metadata_flag
```

passes.

---

## Epic 4 — Prove Trace Propagation Across Runtime Layers

### Issue V1.5.1-013 — Add TraceContext Type and Runtime Propagation Contract

Labels:

```text
v1.5.1, observability, tracing, architecture
```

Type:

```text
Backend
```

### Problem

Tracing must propagate through runtime layers as structured context, not as disconnected log statements.

### Requirements

Introduce or harden:

```rust
pub struct TraceContext {
    pub trace_id: String,
    pub current_span_id: String,
    pub parent_span_id: Option<String>,
    pub work_context_id: Option<String>,
    pub run_id: Option<String>,
}
```

Each layer must receive or derive trace context:

```text
WorkOrchestrator
  → FlowRuntime
  → NodeExecutor
  → MemoryService
  → ModelProvider / ToolRuntime
```

### Required Methods

Suggested:

```rust
impl TraceContext {
    pub fn root(work_context_id: String, run_id: String) -> Self;
    pub fn child_span(&self, name: impl Into<String>) -> Self;
}
```

### Acceptance Criteria

* Trace ID remains same across child spans.
* Span ID changes per layer.
* Parent span ID links properly.
* TraceContext included in metadata summary.

### Definition of Done

```text
cargo test test_trace_context_child_span_preserves_trace_id
```

passes.

---

### Issue V1.5.1-014 — Instrument WorkOrchestrator with Root Span

Labels:

```text
v1.5.1, observability, orchestrator
```

Type:

```text
Backend
```

### Problem

Trace propagation must start at the orchestration boundary.

### Requirements

When execution starts:

```text
WorkOrchestrator::submit_user_intent
WorkOrchestrator::continue_context
WorkOrchestrator::run_until_blocked_or_complete
```

must create or receive root trace context.

Root span name:

```text
work_orchestrator.run
```

Root span attributes:

```text
work_context_id
run_id
domain
phase
autonomy_level
```

### Acceptance Criteria

* Trace root exists for every execution.
* Root trace ID persisted in WorkContext metadata.
* Tests prove root span creation.

### Definition of Done

```text
cargo test test_orchestrator_creates_root_trace
```

passes.

---

### Issue V1.5.1-015 — Instrument Flow Runtime and Node Execution with Child Spans

Labels:

```text
v1.5.1, observability, flow-runtime
```

Type:

```text
Backend
```

### Problem

Flow and node execution must be traceable.

### Requirements

Create child spans for:

```text
flow.execute
node.execute
node.success
node.failure
```

Span attributes:

```text
flow_id
node_id
node_type
work_context_id
run_id
```

### Acceptance Criteria

* Every executed node creates span summary.
* Failed nodes record failure status.
* Span hierarchy links to flow span.
* Metadata includes node span summaries.

### Definition of Done

```text
cargo test test_flow_and_node_trace_spans_are_recorded
```

passes.

---

### Issue V1.5.1-016 — Instrument Memory Retrieval, Pruning, and Summarization

Labels:

```text
v1.5.1, observability, memory
```

Type:

```text
Backend
```

### Problem

Memory control is one of the most failure-prone parts of the system. It must be traceable.

### Requirements

Create spans for:

```text
memory.retrieve
memory.prune
memory.summarize
context.budget.apply
```

Span attributes:

```text
memories_retrieved
memories_pruned
memories_summarized
tokens_before
tokens_after
budget_result
```

### Acceptance Criteria

* Memory spans appear during execution.
* Context budget span appears during execution.
* Span metadata matches execution metadata.
* Test proves span presence.

### Definition of Done

```text
cargo test test_memory_and_budget_trace_spans_are_recorded
```

passes.

---

### Issue V1.5.1-017 — Instrument LLM and Tool Calls

Labels:

```text
v1.5.1, observability, llm, tools
```

Type:

```text
Backend
```

### Problem

The most expensive and risky operations are LLM and tool calls. Naturally, those are exactly where systems love to become opaque garbage.

### Requirements

Create spans for:

```text
llm.call
tool.call
```

Span attributes for LLM:

```text
provider
model
estimated_input_tokens
estimated_output_tokens
budget_enforced
latency_ms
status
```

Span attributes for tools:

```text
tool_name
tool_kind
sandbox_mode
idempotency_key
latency_ms
status
```

### Acceptance Criteria

* LLM calls record trace span.
* Tool calls record trace span.
* Failed calls record error status.
* Trace ID remains same from orchestrator to call layer.
* Test proves propagation.

### Definition of Done

```text
cargo test test_trace_propagates_to_llm_and_tool_calls
```

passes.

---

### Issue V1.5.1-018 — Add End-to-End Trace Propagation Integration Test

Labels:

```text
v1.5.1, observability, integration-test, high-priority
```

Type:

```text
Integration Test
```

### Problem

Individual tracing tests are not enough. The full runtime chain must be proven.

### Required Path

Test this chain:

```text
WorkOrchestrator
  → FlowRuntime
  → NodeExecution
  → MemoryService
  → ContextBudgeter
  → LLM or ToolRuntime
```

### Required Test

```rust
#[tokio::test]
async fn test_trace_propagates_across_orchestrator_flow_node_memory_and_llm_or_tool() {
    // arrange
    // create WorkContext
    // create test flow with memory + llm/tool node
    // configure trace collector

    // act
    // run orchestrator

    // assert
    // one trace_id across all spans
    // root span exists
    // flow span parent is root
    // node span parent is flow
    // memory span parent links under node or flow
    // llm/tool span exists
    // metadata.observability.span_count >= expected
}
```

### Acceptance Criteria

* Single trace ID across all layers.
* Parent-child relationships are correct.
* Metadata contains trace summary.
* Test fails if any layer drops trace context.

### Definition of Done

```text
cargo test test_trace_propagates_across_orchestrator_flow_node_memory_and_llm_or_tool
```

passes.

---

# 9. Final Acceptance Checklist

V1.5.1 is complete only when all are true:

```text
[ ] Oversized context integration test passes.
[ ] Impossible budget negative test passes.
[ ] Model calls cannot bypass budget enforcement.
[ ] Memory pruning integration test passes.
[ ] Memory summarization integration test passes.
[ ] Summarizer failure does not bypass budget.
[ ] ExecutionMetadata persists after successful execution.
[ ] ExecutionMetadata persists after failed execution where possible.
[ ] WorkContext repository roundtrip preserves metadata.
[ ] GET /work-contexts/:id exposes metadata.
[ ] GET /work-contexts/:id/metadata exists.
[ ] CLI work metadata <id> exists.
[ ] CLI work show <id> --metadata exists.
[ ] TraceContext exists or equivalent propagation contract exists.
[ ] Orchestrator creates root span.
[ ] Flow/node layers create child spans.
[ ] Memory/budget layers create spans.
[ ] LLM/tool layers create spans.
[ ] End-to-end trace propagation test passes.
[ ] No new TODO/stub/mock/placeholder implementation added.
```

---

# 10. Required Test Commands

Before marking complete:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Specific V1.5.1 tests:

```bash
cargo test context_budget -- --nocapture
cargo test memory_control -- --nocapture
cargo test execution_metadata -- --nocapture
cargo test observability -- --nocapture
cargo test trace -- --nocapture
```

Repo hygiene:

```bash
rg -n "TODO|FIXME|stub|placeholder|mock|dummy|fake|hardcoded|not implemented|unimplemented|todo!|panic!" src tests
```

This command should return either nothing or only explicitly approved test cases where the word appears in test fixture content.

---

# 11. Implementation Order

Do it in this order, because chaos is not a project management style no matter how many startups pretend otherwise:

```text
1. Define/harden ExecutionMetadata structs.
2. Ensure WorkContext persistence supports nested metadata.
3. Add context budget integration tests.
4. Add memory pruning/summarization integration tests.
5. Wire metadata persistence after execution.
6. Expose metadata through API.
7. Expose metadata through CLI.
8. Add TraceContext propagation contract.
9. Instrument orchestrator/flow/node/memory/LLM/tool.
10. Add end-to-end trace propagation test.
11. Run full test suite and grep for garbage markers.
```

---

# 12. Future Version Alignment

## V1.6 — Runtime Reliability

After V1.5.1, move into:

```text
- idempotency hardening
- retry policy
- tool side-effect tracking
- resumable execution
- failed-node recovery
```

## V1.7 — Import/Plugin Layer

Then:

```text
- Skill importer
- OpenClaw/Hermes-style skill conversion
- tool manifest validation
- plugin registration
```

## V2 — Agents

Only after V1.x is hard:

```text
- real agent roles
- agent assignment
- multi-agent task ownership
- agent-to-agent handoff
```

## V3 — Swarm Orchestration

```text
- parallel execution
- consensus
- conflict resolution
- shared work state
```

## V4 — Production Runtime

```text
- async DB strategy
- queue workers
- distributed execution
- sandbox isolation
```

## V5 — Ecosystem

```text
- community skills
- marketplace
- import/export packs
- remote tool networks
```

---

# 13. Final V1.5.1 Definition of Done

V1.5.1 is done when we can say:

```text
The system proves that context is budgeted before execution.
The system proves memory pruning/summarization is used in real runtime.
The system persists and exposes execution metadata.
The system proves trace propagation from orchestration to LLM/tool execution.
```

Not “the modules exist.”

Not “the docs say it.”

Not “the developer felt emotionally complete.”

The tests prove it. The API exposes it. The CLI shows it. The metadata persists.

That is V1.5.1.
