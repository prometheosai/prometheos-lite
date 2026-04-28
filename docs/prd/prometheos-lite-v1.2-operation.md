Alright. You asked for **everything**—not a cute outline, not vibes, not “you got this champ.”
So here’s the **full V1.2 PRD** written like something another AI (or a very serious human) can actually implement without improvising nonsense.

This is the layer that turns your system from:

```txt
“runs flows”
```

into:

```txt
“runs WORK over time”
```

---

# 🧠 PrometheOS Lite V1.2 PRD

## Codename: **Operation Layer (WorkContext Engine)**

---

# 0. 🧨 Executive Summary

## Goal

Introduce a **persistent operational layer** that manages real-world work across time.

## Core Shift

```txt
V1:
User → Flow → Output

V1.2:
User → WorkContext → Flow → Artifacts → Continue → Complete
```

## Non-Negotiable Principle

```txt
Flow executes work
WorkContext governs work
```

---

# 1. 🧱 System Architecture

## High-Level

```txt
Interface (CLI/API)
  ↓
Gateway (session/context routing)
  ↓
WorkContext Engine (NEW)
  ↓
FlowExecutionService (existing V1)
  ↓
FlowRunner + Nodes (existing V1)
  ↓
Artifacts + Decisions + Memory
```

---

# 2. 🧠 Core Concepts

## 2.1 WorkContext (PRIMARY OBJECT)

```rust
pub struct WorkContext {
    pub id: String,
    pub user_id: String,

    // Identity
    pub title: String,
    pub domain: WorkDomain,
    pub domain_profile_id: Option<String>,
    pub context_type: String,

    // Intent
    pub goal: String,
    pub requirements: Vec<String>,
    pub constraints: Vec<String>,

    // Execution state
    pub status: WorkStatus,
    pub current_phase: WorkPhase,

    // Planning
    pub plan: Option<ExecutionPlan>,
    pub approved_plan: Option<ExecutionPlan>,

    // Artifacts
    pub artifacts: Vec<Artifact>,

    // Memory
    pub memory_refs: Vec<String>,

    // Decisions
    pub decisions: Vec<DecisionRecord>,

    // Execution tracking
    pub flow_runs: Vec<String>,
    pub tool_trace: Vec<String>,

    // Questions / blockers
    pub open_questions: Vec<String>,

    // Control
    pub autonomy_level: AutonomyLevel,
    pub approval_policy: ApprovalPolicy,

    // Extensibility
    pub metadata: serde_json::Value,

    // Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

---

## 2.2 WorkDomain

```rust
pub enum WorkDomain {
    Software,
    Business,
    Marketing,
    Personal,
    Creative,
    Research,
    Operations,
    General,
    Custom(String),
}
```

---

## 2.3 WorkStatus

```rust
pub enum WorkStatus {
    Draft,
    Planning,
    AwaitingApproval,
    InProgress,
    Blocked,
    Completed,
    Failed,
    Archived,
}
```

---

## 2.4 WorkPhase

```rust
pub enum WorkPhase {
    Intake,
    Planning,
    Execution,
    Review,
    Iteration,
    Finalization,
}
```

---

## 2.5 AutonomyLevel

```rust
pub enum AutonomyLevel {
    Chat,
    Review,
    Autonomous,
}
```

---

# 3. 🧩 Domain Profiles

## WorkDomainProfile

```rust
pub struct WorkDomainProfile {
    pub id: String,
    pub name: String,
    pub parent_domain: Option<String>,

    pub default_flows: Vec<String>,
    pub artifact_kinds: Vec<String>,

    pub approval_defaults: ApprovalPolicy,
    pub lifecycle_template: LifecycleTemplate,
}
```

---

## LifecycleTemplate

```rust
pub struct LifecycleTemplate {
    pub phases: Vec<WorkPhase>,
    pub transitions: Vec<(WorkPhase, WorkPhase)>,
}
```

---

# 4. 🧠 Playbook (FOUNDATION ONLY)

## WorkContextPlaybook

```rust
pub struct WorkContextPlaybook {
    pub id: String,
    pub user_id: String,
    pub domain_profile_id: String,

    pub name: String,
    pub description: String,

    pub preferred_flows: Vec<String>,

    pub default_approval_policy: ApprovalPolicy,
    pub default_research_depth: ResearchDepth,
    pub default_creativity_level: CreativityLevel,

    pub evaluation_rules: Vec<String>,

    pub confidence: f32,
    pub usage_count: u32,

    pub updated_at: DateTime<Utc>,
}
```

⚠️ V1.2 uses playbooks but does NOT evolve them automatically.

---

# 5. 📦 Artifact System

```rust
pub struct Artifact {
    pub id: String,
    pub work_context_id: String,
    pub kind: ArtifactKind,
    pub name: String,
    pub content: serde_json::Value,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}
```

---

## ArtifactKind

```rust
pub enum ArtifactKind {
    Plan,
    Code,
    Document,
    Analysis,
    MarketingCopy,
    TaskList,
    EmailDraft,
    Report,
}
```

---

# 6. 🧠 Decision System

```rust
pub struct DecisionRecord {
    pub id: String,
    pub description: String,
    pub chosen_option: String,
    pub alternatives: Vec<String>,
    pub approved: bool,
    pub created_at: DateTime<Utc>,
}
```

---

# 7. 📋 Execution Plan

```rust
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
}

pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub flow_ref: String,
    pub status: StepStatus,
}
```

---

# 8. 🔁 Lifecycle

## WorkContext Lifecycle

```txt
Create
→ Intake
→ Planning
→ AwaitingApproval
→ Execution
→ Review
→ Iteration
→ Finalization
→ Completed
```

---

# 9. ⚙️ Services

---

## Issue #1 — WorkContext Storage

### DB Table

```sql
CREATE TABLE work_contexts (
  id TEXT PRIMARY KEY,
  user_id TEXT,
  title TEXT,
  domain TEXT,
  status TEXT,
  phase TEXT,
  autonomy_level TEXT,
  approval_policy TEXT,
  created_at TEXT,
  updated_at TEXT,
  data_json TEXT
);
```

---

## Issue #2 — WorkContextService

### API

```rust
create_context(...)
get_context(id)
update_context(...)
add_artifact(...)
add_decision(...)
update_status(...)
```

---

## Issue #3 — Context Routing

```txt
If context_id provided → use it
Else if active exists → reuse
Else → create new WorkContext
```

---

## Issue #4 — Flow Integration

FlowExecutionService must:

```rust
execute_message(...)
→ returns FinalOutput
→ apply to WorkContext
```

---

## Issue #5 — Artifact Injection

Every flow result must:

```txt
map to ArtifactKind
store artifact
attach to WorkContext
```

---

## Issue #6 — Continuation Engine

```rust
continue_context(context_id)
```

### Behavior

```txt
Load context
→ inspect phase
→ pick next flow
→ execute
→ update context
```

---

## Issue #7 — Phase Controller

```txt
No plan → Planning
Plan exists → AwaitingApproval
Approved → Execution
Execution done → Review
Review done → Iteration/Final
```

---

## Issue #8 — Approval Integration

```txt
Interrupt → pause context
Approve → resume
Deny → Blocked
```

---

## Issue #9 — Mode System

### Chat

```txt
no persistence
```

### Review

```txt
approval required
```

### Autonomous

```txt
run until:
- budget hit
- approval needed
- complete
```

---

## Issue #10 — CLI

```bash
prometheos context list
prometheos context show <id>
prometheos context continue <id>
prometheos context artifacts <id>
```

---

## Issue #11 — API

```txt
POST /contexts
GET /contexts/:id
POST /contexts/:id/continue
GET /contexts/:id/artifacts
```

---

## Issue #12 — Domain Templates

```
templates/
  software.yaml
  business.yaml
  marketing.yaml
  personal.yaml
  research.yaml
  creative.yaml
```

---

## Issue #13 — Context-Aware Flow Selection

FlowSelector now receives:

```txt
intent + WorkContext
```

---

## Issue #14 — Guardrails Integration

Must enforce:

```txt
ToolPolicy
TrustPolicy
ApprovalPolicy
Budget
LoopDetection
```

---

## Issue #15 — Testing

### Required

```txt
create → plan → artifact created
resume → continues correctly
approval blocks execution
autonomous respects guardrails
```

---

# 🧭 Final Behavior

## System becomes

```txt
WorkContext = operational truth
Flow = execution
Artifact = output
Decision = control
Playbook = personalization (static for now)
```

---

# 🚀 What this enables

After V1.2:

```txt
System remembers work
System continues work
System organizes work
System enforces work rules
```

---

# 🧠 Final Insight

You are building:

```txt
Not:
AI assistant

But:
AI Operating Layer
```

---

# ⚠️ Final Rule

```txt
LLM proposes
System enforces
WorkContext persists
```

---