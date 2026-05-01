You finally reached the phase where systems usually fall apart quietly while everyone pretends it’s “just scaling.” Cute.

V1.5 is where you stop the system from **collapsing under its own intelligence theater**.

---

# ## V1.5 PRD — Stabilization & Context Control

## Executive Context

After V1.4:

```txt
✔ System can act (repo tools, commands, patching)
✔ System can plan (flows, orchestrator)
✔ System can learn (playbooks, evolution)
```

What will now break:

```txt
✘ Context explosion (tokens → garbage performance)
✘ Memory bloat (irrelevant junk dominates prompts)
✘ Weak evaluation (fast garbage gets rewarded)
✘ Zero observability (debugging becomes guesswork)
```

V1.5 exists to prevent:

```txt
“Smart system → slow → confused → wrong → expensive”
```

---

# ## Core Principles

```txt
1. Context is a budget, not a dump
2. Memory must compete to exist
3. Evaluation must punish bad work
4. Every execution must be traceable
```

---

# ## EPIC 1 — Context Budgeter

## Goal

Control token usage across all LLM calls.

---

## Issue: Define ContextBudgeter

**File:** `src/context/budgeter.rs`

```rust
pub struct ContextBudgeter {
    pub max_tokens: usize,
    pub reserved_output_tokens: usize,
}
```

---

## Issue: Budget Allocation Strategy

```txt
priority:
1. system prompt
2. task
3. plan
4. critical memory
5. recent artifacts
6. long-tail memory
```

---

## Issue: Token Estimation

Add utility:

```rust
pub fn estimate_tokens(text: &str) -> usize
```

---

## Issue: Context Trimming

```rust
pub fn build_context(inputs: ContextInputs) -> TrimmedContext
```

Rules:

```txt
- truncate lowest priority first
- preserve structural integrity
- never cut mid-JSON / mid-code block
```

---

## Issue: Integration

Inject into:

```txt
PlannerNode
CoderNode
ReviewerNode
LlmNode
```

---

## Definition of Done

```txt
✔ No LLM call exceeds token limit
✔ Context is deterministic under overflow
✔ Logs show what was dropped
```

---

# ## EPIC 2 — Memory Pruning & Summarization

## Goal

Prevent memory from becoming a junkyard.

---

## Issue: Memory Scoring

Add:

```rust
pub struct MemoryScore {
    pub relevance: f32,
    pub recency: f32,
    pub usage: f32,
}
```

---

## Issue: Memory Ranking

```rust
fn rank_memories(memories: Vec<Memory>) -> Vec<Memory>
```

---

## Issue: Memory Pruning

```rust
fn prune(memories: Vec<Memory>, max: usize)
```

---

## Issue: Memory Summarizer

**File:** `src/memory/summarizer.rs`

```rust
pub fn summarize(memories: Vec<Memory>) -> Memory
```

---

## Issue: Compression Trigger

Trigger when:

```txt
- memory count > threshold
- token size > threshold
```

---

## Issue: Integration

Hook into:

```txt
MemoryService
ContextBuilder
```

---

## Definition of Done

```txt
✔ Memory size bounded
✔ Old/low-value memory removed
✔ Summaries replace clusters
✔ No uncontrolled growth
```

---

# ## EPIC 3 — Evaluation System Upgrade

## Goal

Make system stop rewarding garbage.

---

## Issue: Evaluation Engine Refactor

**File:** `src/work/evaluation.rs`

Expand:

```rust
pub struct EvaluationResult {
    pub score: f32,
    pub dimensions: EvaluationDimensions,
}
```

---

## Issue: Add Dimensions

```txt
- correctness
- completeness
- efficiency
- reliability
```

---

## Issue: Structural Validation

```txt
✔ patch validity
✔ test pass/fail
✔ artifact schema compliance
```

---

## Issue: Semantic Evaluation (LLM-based)

```rust
fn evaluate_semantic(output: &str, task: &str) -> f32
```

---

## Issue: Penalization Rules

```txt
- high retries → penalty
- failed tests → strong penalty
- hallucinated output → critical penalty
```

---

## Issue: Integration

Feed into:

```txt
EvolutionEngine
FlowPerformanceRecord
```

---

## Definition of Done

```txt
✔ Bad outputs reduce playbook confidence
✔ Good outputs reinforce flows
✔ No blind success scoring
```

---

# ## EPIC 4 — Observability (Real Tracing)

## Goal

Make the system debuggable.

---

## Issue: Trace Model

```rust
pub struct ExecutionTrace {
    pub trace_id: String,
    pub work_context_id: String,
    pub flow_id: String,
    pub node_runs: Vec<NodeRun>,
}
```

---

## Issue: NodeRun

```rust
pub struct NodeRun {
    pub node_id: String,
    pub start_time: u64,
    pub duration_ms: u64,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
}
```

---

## Issue: ToolCallLog

```rust
pub struct ToolCallLog {
    pub tool_name: String,
    pub input: Value,
    pub output: Value,
    pub duration_ms: u64,
}
```

---

## Issue: LLM Metrics

```txt
- latency
- token usage
- model used
```

---

## Issue: Storage

Persist traces in:

```txt
SQLite (initial)
future: OpenTelemetry
```

---

## Issue: Trace Hierarchy

```txt
WorkContext
  → FlowRun
    → NodeRun
      → ToolCall
```

---

## Issue: Integration

Hook into:

```txt
RuntimeContext
FlowExecutionService
ToolRuntime
```

---

## Definition of Done

```txt
✔ Every execution has trace_id
✔ Every node is logged
✔ Every tool call is logged
✔ Failures are traceable
```

---

# ## EPIC 5 — Context Builder Refactor

## Goal

Unify context creation across system.

---

## Issue: Create ContextBuilder

**File:** `src/context/builder.rs`

```rust
pub struct ContextBuilder {
    budgeter: ContextBudgeter,
    memory_service: MemoryService,
}
```

---

## Issue: Input Model

```rust
pub struct ContextInputs {
    pub task: String,
    pub plan: Option<String>,
    pub memory: Vec<Memory>,
    pub artifacts: Vec<Artifact>,
}
```

---

## Issue: Output

```rust
pub struct BuiltContext {
    pub prompt: String,
    pub dropped_items: Vec<String>,
}
```

---

## Definition of Done

```txt
✔ All nodes use ContextBuilder
✔ No direct prompt construction
✔ Consistent context shaping
```

---

# ## EPIC 6 — Strict Mode Hardening

## Goal

Eliminate silent failures.

---

## Issue: Enforce No-Silent-Fallback

```txt
✔ missing inputs → error
✔ missing memory → error
✔ empty outputs → error
✔ tool failure → error
```

---

## Issue: Ban unsafe patterns

```txt
✔ no unwrap() in runtime path
✔ no Option::None propagation
```

---

## Issue: Idempotency Check

```txt
✔ prevent duplicate tool execution
✔ check tool_outbox before re-run
```

---

## Definition of Done

```txt
✔ No silent success
✔ No hidden failure paths
✔ Deterministic failure behavior
```

---

# ## EPIC 7 — Testing & Validation

## Required tests

```txt
✔ context overflow trimming
✔ memory pruning correctness
✔ evaluation scoring correctness
✔ trace generation correctness
✔ failure trace visibility
✔ token budget enforcement
```

---

# ## Definition of Done (V1.5)

```txt
✔ Context is bounded and controlled
✔ Memory does not grow indefinitely
✔ Evaluation is multi-dimensional
✔ Every execution is traceable
✔ System remains stable under load
```

---

# ## What you absolutely should NOT do

```txt
✘ Let memory grow unbounded
✘ Let LLM decide context size
✘ Skip evaluation penalties
✘ Ignore observability
```

---

# ## What V1.5 actually unlocks

Before:

```txt
System works sometimes
```

After:

```txt
System works reliably
```

---

# ## Future Roadmap

## V2 — Agents

```txt
multi-agent specialization
planner/coder/reviewer split
tool-aware agents
```

---

## V3 — Advanced Coding System

```txt
AST-aware patching
semantic search
dependency graphs
test generation
```

---

## V4 — Swarm

```txt
parallel execution
shared memory graph
task coordination
```

---

## V5 — Self-Evolving System

```txt
auto-skill creation
flow synthesis
meta-optimization
```

---

# Final reality check

You are entering the stage where:

```txt
systems stop failing loudly
and start failing subtly
```

That’s worse.

V1.5 is what prevents that.

Don’t rush it.
