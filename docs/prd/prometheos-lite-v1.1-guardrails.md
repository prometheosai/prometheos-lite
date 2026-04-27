Yes. Because V1 Core ended stronger than the original plan, **V1.1 priorities changed**.

You already pulled some V1.1 foundations into V1:

* `ToolPermission` / `ToolPolicy` exists. 
* `ToolMetadata` with `schema_hash` exists. 
* `FinalOutput`, `trace_id`, `evaluation`, `budget_report`, `events_count` exist. 
* `TraceEvent` already includes guardrail-adjacent events like `BudgetExceeded`, `ToolRequested`, `ToolCompleted`, `MemoryRead`, `MemoryWrite`, `EvaluationCompleted`, and `OutputGenerated`. 

So V1.1 should no longer be “create guardrail concepts.”
It should be:

```txt
turn guardrail concepts into enforced runtime guarantees
```

Tiny distinction. Also the difference between “secure” and “decorated.”

# PrometheOS Lite V1.1 Guardrails PRD

## Codename

**V1.1 — Enforced Runtime Guardrails**

## Objective

Make V1 Core safe enough for V2 Agents by enforcing:

```txt
ToolContext
Permission checks
Path safety
Flow snapshots
Idempotency
Outbox
Interrupts
Approval policy
Trust policy
Loop detection
Guardrail tests
```

## Non-goals

```txt
No AgentProfile
No SwarmRunner
No Learning loop
No Meta optimizer
No marketplace
No full Harness dashboard
```

V1.1 protects V1. It does not become V2 wearing a fake mustache.

---

# Updated Priority Order

Because V1 already added trace/budget/tool metadata, do V1.1 in this order:

```txt
1. ToolContext enforcement
2. PathGuard + FileWriter hardening
3. ApprovalPolicy + InterruptContext
4. Flow snapshot/versioning
5. Idempotency + Outbox
6. TrustPolicy
7. Loop detection
8. Guardrail event emission audit
9. CLI guardrail commands
10. Tests + docs
```

---

# Issue #1 — Enforced ToolContext

## Goal

No tool or side-effect node runs without execution context.

## Current state

You have `ToolPolicy`, but it is not yet clearly enforced at every tool boundary. 

## Add

```rust
pub struct ToolContext {
    pub run_id: String,
    pub trace_id: String,
    pub node_id: String,
    pub tool_name: String,
    pub policy: ToolPolicy,
    pub trust_level: TrustLevel,
    pub approval_policy: ApprovalPolicy,
    pub idempotency_key: Option<String>,
}
```

## Acceptance criteria

* `ToolRuntime.execute_*` requires `ToolContext`.
* `ToolNode` cannot call tools without context.
* `FileWriterNode` uses `ToolContext`.
* Denied calls emit `PermissionDenied`.
* Approved calls emit `PermissionChecked`.

---

# Issue #2 — PathGuard + FileWriter Hardening

## Goal

Stop arbitrary file writes.

## Current problem

Your `FileWriterNode` still allows absolute paths and Windows-style drive paths. That is not a guardrail. That is a polite suggestion to chaos.

## Add

```txt
src/tools/path_guard.rs
```

## Rules

```txt
No absolute paths
No ..
No symlink escape
Canonical final path must remain inside prometheos-output/
```

## Acceptance criteria

* `file_path = "/etc/passwd"` fails.
* `file_path = "../../secret"` fails.
* `file_path = "safe/output.txt"` succeeds.
* Failure emits `PermissionDenied`.
* Tests cover Unix + Windows-style paths.

---

# Issue #3 — ApprovalPolicy

## Goal

Centralize when execution needs approval.

## Add

```rust
pub enum ApprovalPolicy {
    Auto,
    RequireForTools,
    RequireForSideEffects,
    RequireForUntrusted,
    ManualAll,
}
```

## Acceptance criteria

* Approval policy attaches to `ExecutionOptions`.
* Tool calls consult approval policy.
* Side-effecting tools pause when approval is required.
* Approval events are traced.

---

# Issue #4 — InterruptContext

## Goal

Make human approval resumable.

## Add

```rust
pub struct InterruptContext {
    pub interrupt_id: String,
    pub run_id: String,
    pub trace_id: String,
    pub node_id: String,
    pub reason: String,
    pub expected_schema: serde_json::Value,
    pub expires_at: Option<DateTime<Utc>>,
}
```

## Acceptance criteria

* Interrupts persist to SQLite.
* Resume validates decision schema.
* Expired interrupts fail safely.
* Invalid decision does not mutate `SharedState`.

---

# Issue #5 — Flow Snapshot / Versioning

## Goal

A resumed run must use the same flow definition it started with.

## Add

```rust
pub struct FlowSnapshot {
    pub flow_name: String,
    pub flow_version: String,
    pub source_hash: String,
    pub source_text: String,
    pub created_at: DateTime<Utc>,
}
```

## Acceptance criteria

* Every run stores exact flow source.
* Resume uses stored snapshot, not current YAML file.
* Flow hash mismatch is visible.
* Missing flow version fails in strict mode.

---

# Issue #6 — Idempotency Keys

## Goal

Prevent duplicate side effects on retry/resume.

## Add

```rust
pub struct IdempotencyKey {
    pub key: String,
    pub run_id: String,
    pub node_id: String,
    pub operation_hash: String,
}
```

## Acceptance criteria

* File writes generate deterministic operation hash.
* Repeated side effect checks prior execution.
* Duplicate side effect is blocked or skipped.
* Trace emits `IdempotencyChecked`.

---

# Issue #7 — Outbox Pattern

## Goal

Track side-effecting tool execution safely.

## Table

```sql
CREATE TABLE IF NOT EXISTS tool_outbox (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  trace_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  input_hash TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  completed_at TEXT,
  result_json TEXT
);
```

## Acceptance criteria

* Outbox entry created before side effect.
* Completed side effects are not re-executed on resume.
* Failed side effects are inspectable.
* `FinalOutput` includes side-effect summary eventually.

---

# Issue #8 — TrustPolicy

## Goal

Classify tools and sources.

## Add

```rust
pub enum TrustLevel {
    Trusted,
    Local,
    Community,
    External,
    Untrusted,
}

pub struct TrustPolicy {
    pub source: String,
    pub level: TrustLevel,
    pub require_approval: bool,
}
```

## Defaults

```txt
Built-in tools → Local
Local YAML flows → Local
Downloaded/imported tools → External
Unknown tools → Untrusted
```

## Acceptance criteria

* Untrusted tools require approval.
* Trust level appears in trace.
* Trust can be listed/updated by CLI.

---

# Issue #9 — Loop Detection

## Goal

Stop runaway flows.

## Add

```rust
pub struct LoopDetectionConfig {
    pub max_repeated_node: usize,
    pub max_repeated_transition: usize,
    pub max_repeated_tool_call: usize,
}
```

## Acceptance criteria

* Same node repeated too often → stop.
* Same transition cycle repeated too often → stop.
* Same tool call same args too often → stop.
* Emits `LoopDetected`.

---

# Issue #10 — Guardrail Trace Events

## Add missing events

You already have many trace events, but V1.1 needs these too:

```txt
PermissionChecked
PermissionDenied
ApprovalRequested
ApprovalGranted
ApprovalDenied
InterruptCreated
InterruptResumed
FlowSnapshotStored
SchemaHashChecked
IdempotencyChecked
OutboxPending
OutboxCompleted
TrustPolicyApplied
LoopDetected
```

## Acceptance criteria

* Events are enum variants, not random strings.
* Events are emitted in runtime, not just declared.
* Replay shows them.

---

# Issue #11 — Guardrail CLI

## Commands

```bash
prometheos flow resume <run_id>
prometheos flow events <run_id>
prometheos flow replay <run_id>

prometheos interrupt list
prometheos interrupt approve <interrupt_id> --decision '{}'
prometheos interrupt deny <interrupt_id>

prometheos trust list
prometheos trust set <source> --level trusted

prometheos outbox list
```

## Acceptance criteria

* Commands work without server.
* JSON output by default.
* Human-readable errors.

---

# Issue #12 — Guardrail Test Suite

## Required tests

```txt
tool_without_context_fails
shell_denied_by_default
network_denied_by_default
absolute_file_write_denied
path_traversal_denied
flow_resume_uses_snapshot
schema_hash_change_detected
side_effect_not_reexecuted
interrupt_invalid_decision_rejected
untrusted_tool_requires_approval
loop_detection_stops_run
```

## Acceptance criteria

* CI runs guardrail tests.
* At least 3 integration tests cover resume/interrupt/outbox.
* Tests fail if direct side effects bypass guardrails.

---

# What changed from the old V1.1 plan

## Removed from V1.1 “create basic concept” scope

Already done or mostly done:

```txt
ToolPermission
ToolPolicy
ToolMetadata schema_hash
TraceEvent expansion
FinalOutput contract
Budget report in state/output
```

## Promoted to high priority

Because V1 now has enough runtime core to make these enforceable:

```txt
ToolContext
PathGuard
FlowSnapshot
Idempotency
Outbox
Approval/Interrupt
```

## Still deferred to V3

```txt
OpenTelemetry
full cost accounting dashboard
human approval web UI
advanced replay engine
distributed sandboxing
```

---

# Future scope trace

## V2 — Agent Runtime

V2 depends on V1.1 because agents need safe inherited boundaries:

```txt
AgentProfile.allowed_tools → ToolPolicy
AgentProfile.memory_scope → Memory guard
AgentExecutor → ApprovalPolicy
Skill execution → FlowSnapshot
```

## V3 — Harness

V3 turns V1.1 guardrails into a supervisor:

```txt
HarnessSession
ReplayEngine
Approval dashboard
Outbox recovery worker
Cost model
OpenTelemetry
```

## V4 — Swarm

V4 must use:

```txt
SwarmRunner → Harness → AgentExecutor → Skill → Flow
```

No swarm outside guardrails. We are not giving 7 agents shell access and hoping vibes do access control.

## V5 — Learning

V5 learns from:

```txt
TraceEvent
Evaluation
Outbox failures
Approval decisions
LoopDetected
BudgetExceeded
```

## V6 — Meta-Optimization

V6 proposes changes to:

```txt
flows
agents
tool policies
budgets
model routing
```

But cannot auto-apply without approval.

## V7 — Persistent Identity

V7 inherits personality/context but must respect:

```txt
TrustPolicy
ApprovalPolicy
Memory boundaries
```

---

# Final recommendation

Do **not** call V1.1 done until this is true:

```txt
No side effect can occur without ToolContext + ToolPolicy + trace.
No resumed run can use a changed flow definition.
No retry can duplicate a side effect silently.
No untrusted tool can execute without approval.
No flow can loop forever.
```

That is V1.1.

Not more intelligence.
Brakes. Locks. Receipts. The boring holy trinity of systems that don’t embarrass their creators.
