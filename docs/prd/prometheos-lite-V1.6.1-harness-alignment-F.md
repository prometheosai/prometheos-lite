# PrometheOS Lite V1.6.1 PRD

## Harness Alignment, Enforcement & Token-Lean Correctness

## 0. Executive Summary

PrometheOS Lite V1.6 already has major harness-shaped pieces: WorkContext, Flow execution, repo tools, patching, tracing, model routing, memory, and software-domain workflows. The problem is no longer “build the harness from nothing.” The problem is uglier and more realistic: **the harness pieces are not yet consolidated into one mandatory, evidence-gated execution path**.

So V1.6.1 is not a new feature parade. It is the alignment release.

**V1.6.1 must make this path non-optional:**

```text
WorkContext
→ Intent + Acceptance Criteria
→ RepoMap
→ Token Budgeter
→ Patch Protocol
→ Git Checkpoint
→ Validation
→ Review/Risk
→ Attempt Selection
→ Evidence Log
→ Completion Policy
```

The uploaded PRD is structurally sound: it keeps WorkContext-first architecture, preserves the Flow engine, adds Node Registry, Patch Protocol, RepoMap, Review Gate, Attempt Pool, and Observability, then adds token budgeting, context distillation, hallucination metrics, and local-first cost routing. Those are valid and should be absorbed. The parts that need correction are the stale assumptions about “missing everything,” the dangerous `git add -A` checkpoint idea, and the “zero hallucination” language, which should become **measurable hallucination-risk reduction with validation evidence**, because reality remains rudely attached to physics. 

---

# 1. Product Goal

## Goal

Make PrometheOS Lite a **local-first, WorkContext-native coding execution system** that can safely inspect, modify, validate, review, and explain changes to a real repository with minimal token waste.

## The One-Line Product Promise

> PrometheOS Lite does not just generate code. It proves what it changed, why it changed it, whether it passed validation, how much it cost, and how to roll it back.

## Strategic Positioning

PrometheOS Lite should not compete as “another chat coding assistant.” That battlefield is already crowded with tools politely burning money through token fog.

PrometheOS Lite competes as:

```text
A local-first AI work harness with durable context, repo grounding, patch safety,
validation gates, token discipline, and auditable evidence.
```

---

# 2. Core Principles

## 2.1 WorkContext-first

Everything belongs to a WorkContext:

```text
task goal
repo evidence
patches
attempts
validation logs
review reports
risk reports
token/cost metrics
completion decision
rollback instructions
```

No orphaned tool calls. No “the model did something somewhere.” We are not raising feral processes in the woods.

## 2.2 Harness over LLM

The LLM proposes. The harness verifies.

```text
LLM = reasoning worker
Harness = execution authority
WorkContext = durable mission ledger
Compiler/tests = reality check
```

## 2.3 No evidence, no progress

For software-domain work:

```text
No RepoMap = no confident patch generation.
No patch = no coding completion.
No checkpoint = no write.
No validation = no review transition.
No risk report = no autonomous finalization.
No completion evidence = no Completed phase.
```

## 2.4 Token efficiency is a hard constraint

Token discipline is not a “nice optimization.” It is part of correctness.

The system must:

```text
avoid full-file dumping
prefer diffs over whole files
retrieve only relevant symbols/files
summarize old WorkContext history
cap input/output tokens
track cost per node
show cost to the user
```

## 2.5 “Zero hallucination” becomes measurable enforcement

Absolute “zero hallucination” is not technically honest. The system should instead enforce:

```text
zero unvalidated completion
zero ungrounded code edits
zero silent patch failures
zero fake test-pass claims
zero unchecked phase advancement
```

That is stronger than marketing poetry and less likely to embarrass us in front of a compiler.

---

# 3. Current Codebase Reality

Based on the repo audit and the uploaded PRD context, V1.6.1 should treat these as existing foundations:

```text
src/flow/
  flow lifecycle
  Node trait
  SharedState
  NodeFactory
  RuntimeContext
  ToolRuntime
  ModelRouter
  tracing / OpenTelemetry pieces
  coding analysis nodes

src/work/
  WorkContext
  WorkOrchestrator
  WorkExecutionService
  PhaseController
  PlaybookResolver
  WorkPhase
  WorkDomain
  AutonomyLevel
  ApprovalPolicy

src/tools/
  repo tools
  read_file
  write_file
  search_files
  list_tree
  patch_file
  git_diff
  run_tests
  command

src/db/
  SQLite persistence
  run DB / WorkContext persistence

src/api/
  Axum API
  WorkContext routes

src/cli/
  work commands
  flow commands
```

## Current Gaps V1.6.1 Must Fix

```text
NodeFactory still too hardcoded
Patch tool exists but is not full patch lifecycle
write_file can bypass patch discipline
Repo intelligence is heuristic, not a proper RepoMap
Validation is not mandatory for Execution → Review
Review/risk is not a hard gate
Attempt pool is not fully isolated/selective
OpenTelemetry exists but is not queryable product evidence
Token budgets are not hard invariants everywhere
Context distillation is not strict enough
Hallucination/correction metrics are not first-class
Cost transparency is not visible enough
```

---

# 4. Non-Goals

V1.6.1 will not:

```text
Build a full IDE
Replace SQLite with Postgres
Chase SWE-bench leaderboard as the main goal
Rewrite the Flow engine
Rewrite the WorkContext system
Make Docker sandbox mandatory
Promise literal zero hallucination
Add cloud-hosted sandbox infrastructure
Add WhatsApp/Telegram approval flows
Build a plugin marketplace
```

Lovely future monsters. Not this release.

---

# 5. Target Architecture

```text
User / CLI / API
  ↓
WorkContext
  ↓
Intent + Acceptance Criteria
  ↓
RepoMap Engine
  ↓
Context Budgeter + Distiller
  ↓
ModelRouter
  ↓
Patch Proposal
  ↓
Patch Protocol
  ↓
File Control + Permission Ledger
  ↓
Git Checkpoint
  ↓
Patch Apply
  ↓
Validation Runner
  ↓
Review + Risk Gate
  ↓
Attempt Pool / Selector
  ↓
Evidence Log
  ↓
Completion Policy
  ↓
WorkContext Artifact + Trace + Cost Summary
```

---

# 6. Canonical V1.6.1 Module Map

```text
src/harness/
  mod.rs
  contract.rs
  evidence.rs
  completion.rs
  acceptance.rs

  repo_map/
    mod.rs
    scanner.rs
    parser.rs
    tree_sitter.rs
    heuristic.rs
    symbols.rs
    dependency_graph.rs
    context_pack.rs
    cache.rs

  patch/
    mod.rs
    model.rs
    parser.rs
    validator.rs
    applier.rs
    dry_run.rs
    rollback.rs
    history.rs

  git/
    mod.rs
    checkpoint.rs
    dirty_state.rs
    rollback.rs
    diff.rs

  validation/
    mod.rs
    environment.rs
    runner.rs
    commands.rs
    result.rs

  review/
    mod.rs
    semantic_diff.rs
    risk.rs
    report.rs
    confidence.rs

  attempts/
    mod.rs
    pool.rs
    workspace.rs
    selector.rs
    consensus.rs

  budget/
    mod.rs
    token_budgeter.rs
    context_distiller.rs
    cost_tracker.rs

  observability/
    mod.rs
    spans.rs
    trace_summary.rs
    metrics.rs

src/flow/factory/
  registry.rs
  register_builtin.rs
  register_harness.rs

src/tools/
  repo.rs
  patch_apply.rs
```

Important: `src/tools/patch_apply.rs` should expose the tool, but the actual patch lifecycle belongs in `src/harness/patch/`. Otherwise patching becomes “just another tool,” which is how we get bypasses, chaos, and then a very expensive debugging séance.

---

# 7. Core Data Models

## 7.1 Harness Run

```rust
pub struct HarnessRun {
    pub id: String,
    pub work_context_id: String,
    pub repo_root: PathBuf,
    pub goal: String,
    pub domain: WorkDomain,
    pub mode: HarnessMode,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: HarnessStatus,
}

pub enum HarnessMode {
    ReviewOnly,
    Assisted,
    Autonomous,
    Benchmark,
}

pub enum HarnessStatus {
    Running,
    Completed,
    NeedsRepair,
    NeedsApproval,
    Blocked,
    Failed,
}
```

## 7.2 Evidence Log

```rust
pub struct EvidenceLog {
    pub id: String,
    pub work_context_id: String,
    pub harness_run_id: String,
    pub repo_map_id: Option<String>,
    pub patch_ids: Vec<String>,
    pub selected_attempt_id: Option<String>,
    pub validation_id: Option<String>,
    pub review_id: Option<String>,
    pub risk_id: Option<String>,
    pub completion_id: Option<String>,
    pub cost_id: Option<String>,
}
```

## 7.3 Completion Evidence

```rust
pub struct CompletionEvidence {
    pub patch_exists: bool,
    pub validation_ran: bool,
    pub validation_passed: bool,
    pub review_ran: bool,
    pub critical_issues: usize,
    pub risk_level: RiskLevel,
    pub confidence: ConfidenceScore,
    pub verification_strength: VerificationStrength,
}

pub enum CompletionDecision {
    Complete,
    NeedsRepair(String),
    NeedsApproval(String),
    Blocked(String),
}
```

## 7.4 Token Budget

```rust
pub struct WorkContextBudget {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_total_tokens: u32,
    pub max_cost_cents: u32,
}

pub enum BudgetDecision {
    Allow,
    Trimmed { tokens_removed: u32 },
    Blocked { reason: String },
}
```

---

# 8. Epic Overview

V1.6.1 should be implemented as **5 epics**.

```text
Epic 1 — Harness Contract, Node Registry & WorkContext Enforcement
Epic 2 — RepoMap, Context Discipline & Token Budgeting
Epic 3 — Patch Protocol, File Control & Git Safety
Epic 4 — Validation, Review, Risk & Completion Evidence
Epic 5 — Attempt Pool, Observability, Metrics & Cost Transparency
```

---

# EPIC 1 — Harness Contract, Node Registry & WorkContext Enforcement

## Objective

Make the harness an explicit execution contract, not a pile of optional utilities. Refactor node creation so harness nodes can be added without editing the central factory like it’s 2009 and extensibility is a forbidden art.

---

## Issue 1.1 — Add `HarnessContract`

**Priority:** P0
**Files:**

```text
src/harness/mod.rs
src/harness/contract.rs
src/work/types.rs
src/work/execution_service.rs
```

### Goal

Define a canonical contract for all software-domain harness runs.

### Requirements

Create:

```rust
pub struct HarnessRequest {
    pub work_context_id: String,
    pub repo_root: PathBuf,
    pub task: String,
    pub acceptance_criteria: Vec<String>,
    pub mode: HarnessMode,
    pub budget: WorkContextBudget,
}

pub struct HarnessResult {
    pub run_id: String,
    pub evidence_log_id: String,
    pub completion_decision: CompletionDecision,
    pub artifact_summary: String,
}
```

### Acceptance Criteria

```text
All software-domain execution can be represented as HarnessRequest
HarnessResult can update WorkContext
No harness run can complete without EvidenceLog
Unit tests cover request/result serialization
```

---

## Issue 1.2 — Replace `DefaultNodeFactory` Match with `NodeRegistry`

**Priority:** P0
**Files:**

```text
src/flow/factory/node_factory.rs
src/flow/factory/registry.rs
src/flow/factory/register_builtin.rs
src/flow/factory/register_harness.rs
```

### Goal

Make nodes pluggable.

### Requirements

Create:

```rust
pub trait NodeFactoryPlugin: Send + Sync {
    fn node_type(&self) -> &'static str;
    fn create(
        &self,
        config: NodeConfig,
        runtime: RuntimeContext,
    ) -> anyhow::Result<Arc<dyn Node>>;
}

pub struct NodeRegistry {
    plugins: HashMap<String, Arc<dyn NodeFactoryPlugin>>,
}
```

### Acceptance Criteria

```text
Existing node names still work
Existing flows run unchanged
Adding a new node requires zero edits to node_factory.rs
Registry returns clear error for unknown node type
cargo test passes
```

---

## Issue 1.3 — Register Harness Nodes

**Priority:** P0
**Depends on:** Issue 1.2
**Files:**

```text
src/flow/factory/register_harness.rs
src/harness/mod.rs
```

### Harness Node Names

```text
harness.repo_map
harness.patch_apply
harness.validate
harness.review
harness.risk
harness.completion
harness.attempt_pool
harness.context_distill
```

### Acceptance Criteria

```text
All harness nodes registered via NodeRegistry
Flow JSON can reference harness.* node types
No edits required to core flow loop
```

---

## Issue 1.4 — WorkContext Harness Fields

**Priority:** P0
**Files:**

```text
src/work/types.rs
src/db/migrations/001_harness_161.sql
```

### Add WorkContext Metadata

```rust
pub struct HarnessMetadata {
    pub latest_run_id: Option<String>,
    pub evidence_log_id: Option<String>,
    pub completion_decision: Option<CompletionDecision>,
    pub risk_level: Option<RiskLevel>,
    pub verification_strength: Option<VerificationStrength>,
    pub token_usage: Option<TokenUsageSummary>,
}
```

### Acceptance Criteria

```text
WorkContext can store latest harness state
Migration adds required persistence fields/tables
Old WorkContexts load safely
```

---

## Issue 1.5 — Phase Transition Enforcement

**Priority:** P0
**Files:**

```text
src/work/phase_controller.rs
src/work/execution_service.rs
src/harness/completion.rs
```

### Goal

Make WorkPhase transitions evidence-gated.

### Rules

```text
Software Planning → Execution requires accepted plan or autonomy allowance
Software Execution → Review requires patch evidence or no-code justification
Software Execution → Review requires validation evidence
Software Review → Finalization requires ReviewReport + RiskReport
Software Finalization → Completed requires CompletionDecision::Complete
Failed validation routes to Iteration or Blocked
High/Critical risk routes to NeedsApproval unless explicitly allowed
```

### Acceptance Criteria

```text
WorkContext cannot move to Review without validation evidence
WorkContext cannot move to Completed without CompletionEvidence
Failed validation causes Iteration/Blocked
Tests cover allowed and blocked transitions
```

---

# EPIC 2 — RepoMap, Context Discipline & Token Budgeting

## Objective

Give PrometheOS repo “vision” without dumping the whole repo into the model like a barbarian. Build a RepoMap, enforce context budgets, distill old WorkContext history, and make hallucination prevention measurable.

---

## Issue 2.1 — RepoMap Engine

**Priority:** P0
**Files:**

```text
src/harness/repo_map/mod.rs
src/harness/repo_map/scanner.rs
src/harness/repo_map/parser.rs
src/harness/repo_map/tree_sitter.rs
src/harness/repo_map/heuristic.rs
src/harness/repo_map/symbols.rs
```

### Goal

Build a symbol-aware repo index.

### Requirements

```rust
pub struct RepoMap {
    pub id: String,
    pub repo_root: PathBuf,
    pub files: Vec<FileInfo>,
    pub symbols: Vec<SymbolInfo>,
    pub dependencies: Vec<DependencyEdge>,
    pub entrypoints: Vec<PathBuf>,
    pub test_files: Vec<PathBuf>,
    pub generated_files: Vec<PathBuf>,
}

pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}
```

### Languages

```text
Rust
TypeScript
JavaScript
Python
fallback heuristic parser
```

### Acceptance Criteria

```text
Indexes prometheos-lite repo in under 5 seconds cold
Warm symbol query under 200ms for 50k LOC target
Respects .gitignore
Skips target, node_modules, .git, dist, build
Skips binary/huge files
Does not panic on parser failure
Falls back to heuristic parser when tree-sitter fails
```

---

## Issue 2.2 — RepoMap SQLite Cache

**Priority:** P0
**Files:**

```text
src/harness/repo_map/cache.rs
src/db/migrations/001_harness_161.sql
```

### Tables

```sql
repo_maps(
  id TEXT PRIMARY KEY,
  repo_root TEXT NOT NULL,
  repo_hash TEXT NOT NULL,
  created_at TEXT NOT NULL
);

repo_symbols(
  id TEXT PRIMARY KEY,
  repo_map_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  file_path TEXT NOT NULL,
  line_start INTEGER NOT NULL,
  line_end INTEGER NOT NULL
);

repo_dependencies(
  id TEXT PRIMARY KEY,
  repo_map_id TEXT NOT NULL,
  from_file TEXT NOT NULL,
  to_file TEXT,
  dependency TEXT NOT NULL
);
```

### Acceptance Criteria

```text
RepoMap persists to SQLite
Cache invalidates when repo hash changes
Symbol query reads from cache
Migration is idempotent
```

---

## Issue 2.3 — Ranked Context Pack

**Priority:** P0
**Files:**

```text
src/harness/repo_map/context_pack.rs
src/context/builder.rs
```

### Goal

Build minimal context for an LLM call.

### Requirements

```rust
pub struct RepoContextPack {
    pub task: String,
    pub ranked_files: Vec<RankedFile>,
    pub symbols: Vec<SymbolInfo>,
    pub snippets: Vec<CodeSnippet>,
    pub token_estimate: u32,
}
```

### Priority Order

```text
explicitly mentioned files
symbols mentioned in task
entrypoints
related test files
recently changed files
dependency-neighbor files
fallback ranked files
```

### Acceptance Criteria

```text
Never sends full repo
Never sends full large file by default
Context pack respects token budget
Prompt includes why each file/snippet was selected
```

---

## Issue 2.4 — Token Budgeter with Hard Caps

**Priority:** P0
**Files:**

```text
src/harness/budget/token_budgeter.rs
src/control/budget.rs
src/work/types.rs
src/flow/types.rs
src/flow/intelligence.rs
```

### Goal

No LLM call exceeds budget.

### Requirements

```rust
pub struct TokenBudgeter {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_cost_cents: u32,
}

pub fn enforce_budget(
    prompt: PromptParts,
    budget: &TokenBudgeter,
) -> anyhow::Result<BudgetedPrompt>;
```

### Context Priority

```text
User goal
Acceptance criteria
Current patch/diff
Relevant RepoMap snippets
Recent decisions
Milestones
Older raw history last
```

### Acceptance Criteria

```text
Flow pauses with BudgetExceeded instead of calling model
No LLM call exceeds configured input cap
No output request exceeds configured output cap
Test with 4k cap and 20k context trims correctly
Budget decision recorded in trace
```

---

## Issue 2.5 — Context Distillation

**Priority:** P0
**Files:**

```text
src/harness/budget/context_distiller.rs
src/context/builder.rs
src/flow/memory.rs
```

### Goal

Prevent chat-history snowballing.

### Requirements

```rust
pub struct Milestone {
    pub id: String,
    pub work_context_id: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub artifacts: Vec<String>,
    pub created_at: DateTime<Utc>,
}
```

### Rules

```text
After 10 turns, summarize older raw events into milestones
Retrieve top memories instead of appending all history
Retrieve top RepoMap symbols instead of full files
Do not send raw tool logs unless needed
```

### Acceptance Criteria

```text
50-turn WorkContext prompt stays under configured cap
Milestones preserve decisions and constraints
No full file included unless explicitly selected
Distillation event recorded in EvidenceLog
```

---

## Issue 2.6 — Grounding Guard

**Priority:** P0
**Files:**

```text
src/harness/repo_map/context_pack.rs
src/harness/review/report.rs
src/flow/intelligence.rs
```

### Goal

Prevent ungrounded code claims.

### Rule

Every code-editing prompt must include:

```text
If the required implementation detail is not present in the provided repo evidence,
call the appropriate repo tool instead of assuming.
```

### Acceptance Criteria

```text
Patch prompt contains grounding clause
Patch output must cite selected files/symbols
Patch rejected if it references unknown file/symbol without tool evidence
Review report flags ungrounded references
```

---

# EPIC 3 — Patch Protocol, File Control & Git Safety

## Objective

Make code mutation safe, reversible, reviewable, and impossible to bypass in normal software workflows.

---

## Issue 3.1 — File Control Policy

**Priority:** P0
**Files:**

```text
src/harness/patch/file_control.rs
src/tools/repo.rs
src/work/types.rs
```

### Requirements

```rust
pub struct FilePolicy {
    pub repo_root: PathBuf,
    pub allowed_write_paths: Vec<PathBuf>,
    pub denied_paths: Vec<PathBuf>,
    pub allow_delete: bool,
    pub allow_rename: bool,
    pub allow_dependency_changes: bool,
}
```

### Default Deny

```text
.env
secrets
private keys
.git
target
node_modules
lockfiles unless approved
production config unless approved
migrations unless approved
```

### Acceptance Criteria

```text
Rejects path traversal
Rejects writes outside repo root
Rejects denied paths
Delete/rename require explicit approval
Dependency/config changes require risk review
```

---

## Issue 3.2 — Extract Patch Tool into Harness Patch Module

**Priority:** P0
**Files:**

```text
src/harness/patch/mod.rs
src/harness/patch/model.rs
src/harness/patch/parser.rs
src/harness/patch/validator.rs
src/tools/patch_apply.rs
src/tools/repo.rs
```

### Goal

Keep existing `patch_file`, but make `patch_apply` canonical.

### Requirements

```rust
pub struct Patch {
    pub id: String,
    pub work_context_id: String,
    pub attempt_id: Option<String>,
    pub operations: Vec<PatchOperation>,
}

pub enum PatchOperation {
    SearchReplace { file: PathBuf, search: String, replace: String },
    UnifiedDiff { diff: String },
    CreateFile { file: PathBuf, content: String },
    DeleteFile { file: PathBuf },
    RenameFile { from: PathBuf, to: PathBuf },
}
```

### Acceptance Criteria

```text
patch_file remains backwards-compatible alias
patch_apply is canonical
Patch parser supports unified diff and search/replace
Malformed patch returns structured error
No production TODO/stub
```

---

## Issue 3.3 — Patch Dry-Run

**Priority:** P0
**Files:**

```text
src/harness/patch/dry_run.rs
src/harness/patch/applier.rs
```

### Acceptance Criteria

```text
Dry-run performs no writes
Search/replace requires unique match
Unified diff must apply cleanly
Failure returns nearby context
Dry-run result attached to EvidenceLog
```

---

## Issue 3.4 — Disable Raw Write Bypass for Software WorkContexts

**Priority:** P0
**Files:**

```text
src/tools/repo.rs
src/flow/intelligence/tool.rs
src/work/execution_service.rs
```

### Rule

For `WorkDomain::Software`:

```text
write_file disabled by default
patch_apply required for modifications
create_file allowed only through PatchOperation::CreateFile
delete/rename require approval
```

### Acceptance Criteria

```text
Software WorkContext cannot use write_file directly unless approved
Non-software domains can still use write_file if policy allows
Attempt to bypass patching is logged as denied tool action
```

---

## Issue 3.5 — Safe Git Checkpoint

**Priority:** P0
**Files:**

```text
src/harness/git/checkpoint.rs
src/harness/git/dirty_state.rs
src/harness/git/rollback.rs
```

### Important Correction

Do **not** use:

```bash
git add -A && git commit -m "checkpoint:{trace_id}"
```

That can commit unrelated user changes. Very efficient, if the goal is betrayal.

### Requirements

```rust
pub struct GitCheckpoint {
    pub id: String,
    pub work_context_id: String,
    pub before_head: Option<String>,
    pub dirty_files_before: Vec<PathBuf>,
    pub touched_files: Vec<PathBuf>,
    pub diff_before: String,
}
```

### Rules

```text
Detect dirty state before patch
Refuse overlapping dirty touched files unless approved
Checkpoint only harness-touched files
Support non-git fallback via file snapshots
Never stage unrelated files
```

### Acceptance Criteria

```text
Rollback restores harness changes only
Unrelated dirty files are preserved
Non-git repo uses snapshot fallback
Checkpoint ID stored with patch history
```

---

## Issue 3.6 — Patch History

**Priority:** P0
**Files:**

```text
src/harness/patch/history.rs
src/db/migrations/001_harness_161.sql
```

### Table

```sql
patches(
  id TEXT PRIMARY KEY,
  work_context_id TEXT NOT NULL,
  attempt_id TEXT,
  diff TEXT NOT NULL,
  status TEXT NOT NULL,
  dry_run_passed INTEGER NOT NULL,
  applied_at TEXT,
  rolled_back_at TEXT,
  created_at TEXT NOT NULL
);
```

### Acceptance Criteria

```text
Every patch attempt stored
Patch status queryable by WorkContext
Rollback updates patch status
work show displays patch history
```

---

# EPIC 4 — Validation, Review, Risk & Completion Evidence

## Objective

Turn “the agent says it worked” into “the compiler, tests, review, and risk gate agree enough to proceed.” Civilization inches forward.

---

## Issue 4.1 — Environment Fingerprint

**Priority:** P0
**Files:**

```text
src/harness/validation/environment.rs
```

### Detect

```text
Cargo.toml
package.json
pnpm-lock.yaml
yarn.lock
pyproject.toml
requirements.txt
go.mod
Makefile
Dockerfile
```

### Output

```rust
pub struct EnvironmentProfile {
    pub languages: Vec<String>,
    pub build_commands: Vec<String>,
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
}
```

### Acceptance Criteria

```text
Rust projects detect cargo check/test/fmt/clippy
Node projects detect build/test/lint where available
Python projects detect pytest/ruff/mypy where available
Profile stored as artifact
```

---

## Issue 4.2 — Validation Runner

**Priority:** P0
**Files:**

```text
src/harness/validation/runner.rs
src/harness/validation/result.rs
```

### Requirements

```rust
pub struct ValidationEvidence {
    pub id: String,
    pub work_context_id: String,
    pub commands: Vec<CommandEvidence>,
    pub passed: bool,
    pub verification_strength: VerificationStrength,
}
```

### Acceptance Criteria

```text
Runs detected validation commands
Captures stdout/stderr/exit code/duration
Timeout enforced
No validation reports VerificationStrength::None
Failed command causes validation failure
Evidence stored and linked to WorkContext
```

---

## Issue 4.3 — Review Report

**Priority:** P0
**Files:**

```text
src/harness/review/report.rs
```

### Requirements

```rust
pub struct ReviewReport {
    pub id: String,
    pub work_context_id: String,
    pub issues: Vec<ReviewIssue>,
    pub summary: String,
    pub passed: bool,
}

pub struct ReviewIssue {
    pub severity: Severity,
    pub category: IssueCategory,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
}
```

### Acceptance Criteria

```text
Review runs after patch and validation
Critical issues block completion
Review report stored as artifact
Report available through CLI/API
```

---

## Issue 4.4 — Semantic Diff & Risk Report

**Priority:** P0
**Files:**

```text
src/harness/review/semantic_diff.rs
src/harness/review/risk.rs
```

### Risk Triggers

```text
auth
payments
secrets
.env
database migrations
dependency upgrades
production config
large deletions
network side effects
git push
```

### Requirements

```rust
pub struct RiskReport {
    pub id: String,
    pub level: RiskLevel,
    pub reasons: Vec<String>,
    pub requires_approval: bool,
}
```

### Acceptance Criteria

```text
High/Critical risk requires approval
Risk report linked to WorkContext
Risk shown in work show
Risk affects CompletionDecision
```

---

## Issue 4.5 — Completion Policy

**Priority:** P0
**Files:**

```text
src/harness/completion.rs
src/work/phase_controller.rs
```

### Rules

```text
No patch = not complete for coding task
No validation = not complete
Failed validation = NeedsRepair
Critical review issue = Blocked
High/Critical risk = NeedsApproval
Budget exceeded = Blocked or NeedsApproval
Low confidence = NeedsApproval
Manual/no-code task must explicitly mark patch not applicable
```

### Acceptance Criteria

```text
CompletionDecision controls WorkContext final status
Fake completion impossible through harness path
CLI/API show completion evidence
Tests cover Complete, NeedsRepair, NeedsApproval, Blocked
```

---

## Issue 4.6 — Hallucination / Rejection Metrics

**Priority:** P0
**Files:**

```text
src/harness/evidence.rs
src/db/migrations/002_metrics.sql
src/api/routes.rs
```

### Rename

Use **rejection rate** or **hallucination-risk rate**, not absolute hallucination certainty.

### Table

```sql
patch_metrics(
  work_context_id TEXT PRIMARY KEY,
  patches_generated INTEGER NOT NULL,
  patches_rejected INTEGER NOT NULL,
  patches_failed_tests INTEGER NOT NULL,
  patches_ungrounded INTEGER NOT NULL
);
```

### Metric

```text
hallucination_risk_rate =
  (patches_rejected + patches_ungrounded + patches_failed_tests) / patches_generated
```

### Acceptance Criteria

```text
Every rejected patch has reason
API exposes quality metrics
CLI warns if hallucination_risk_rate > 0.2
Metrics visible per WorkContext
```

---

# EPIC 5 — Attempt Pool, Observability, Metrics & Cost Transparency

## Objective

Run multiple attempts when useful, isolate them, select the best one, record why, expose traces, and show users tokens/costs saved. Finally, a system that tells the truth instead of hiding behind a spinner.

---

## Issue 5.1 — Attempt Pool

**Priority:** P1
**Files:**

```text
src/harness/attempts/pool.rs
src/harness/attempts/workspace.rs
src/work/execution_service.rs
```

### Requirements

```rust
pub struct AttemptPool {
    pub id: String,
    pub work_context_id: String,
    pub attempts: Vec<HarnessAttempt>,
    pub selected_attempt_id: Option<String>,
}

pub struct HarnessAttempt {
    pub id: String,
    pub workspace_path: PathBuf,
    pub patch_id: Option<String>,
    pub validation_id: Option<String>,
    pub review_id: Option<String>,
    pub risk_id: Option<String>,
    pub cost: Option<TokenCost>,
}
```

### Isolation

```text
Preferred: git worktree
Fallback: temporary repo copy
Only selected attempt applies to final workspace
Rejected attempts preserve artifacts
```

### Acceptance Criteria

```text
Balanced mode creates 3 isolated attempts
Rejected attempts do not modify final repo
All attempts logged under parent run ID
```

---

## Issue 5.2 — Consensus / Selection Node

**Priority:** P1
**Files:**

```text
src/harness/attempts/selector.rs
src/harness/attempts/consensus.rs
```

### Selection Criteria

```text
validation passed
no critical review issue
lower risk
smaller diff
lower cost
higher confidence
acceptance criteria satisfied
```

### Acceptance Criteria

```text
Selector explains winner
Selector explains rejected attempts
Failed validation cannot win unless all failed
Selection stored in EvidenceLog
```

---

## Issue 5.3 — OpenTelemetry Harness Spans

**Priority:** P0
**Files:**

```text
src/harness/observability/spans.rs
src/flow/opentelemetry.rs
src/flow/tracing.rs
```

### Required Spans

```text
harness.run
repo_map.build
context.distill
budget.enforce
patch.parse
patch.dry_run
git.checkpoint
patch.apply
validation.run
review.run
risk.assess
attempt.select
completion.evaluate
```

### Acceptance Criteria

```text
Spans include work_context_id/run_id where safe
No secrets in spans
OTLP behind feature flag
stdout/file tracing works locally by default
```

---

## Issue 5.4 — Queryable Trace Summary

**Priority:** P0
**Files:**

```text
src/harness/observability/trace_summary.rs
src/db/migrations/001_harness_161.sql
src/cli/commands/traces.rs
src/api/routes.rs
```

### Requirements

```rust
pub struct TraceSummary {
    pub run_id: String,
    pub work_context_id: String,
    pub duration_ms: u64,
    pub node_count: usize,
    pub tool_count: usize,
    pub error_count: usize,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_cents: u32,
}
```

### CLI

```bash
prometheos traces show <run_id>
prometheos traces list --work-context <id>
```

### API

```text
GET /api/traces/:run_id
GET /api/work-contexts/:id/traces
```

### Acceptance Criteria

```text
Can query tokens, duration, failure count per WorkContext
Trace summary persists after process exits
work show links latest trace
```

---

## Issue 5.5 — Cost Transparency & Local-First Routing

**Priority:** P0
**Files:**

```text
src/harness/budget/cost_tracker.rs
src/flow/intelligence.rs
src/config/mod.rs
src/cli/commands/work.rs
```

### Model Roles

```rust
pub enum ModelRole {
    Discovery,
    Planning,
    Patch,
    Review,
    Judge,
    Summarize,
}
```

### Routing Defaults

```text
Discovery → local/small model or deterministic tool
Planning → configured default model
Patch → local model by default where possible
Review → stronger model if configured
Judge → separate model from patch generator where possible
Summarize → cheap/local model
```

### CLI

```bash
prometheos work cost <id>
```

### Output

```text
tokens in/out
estimated cost
model used by node
local vs frontier ratio
tokens saved by RepoMap vs full-file baseline
```

### Acceptance Criteria

```text
Every model call records role/model/tokens/cost
CLI shows per-WorkContext cost
Default config prefers local for patch attempts where available
System never hides token spend from user
```

---

## Issue 5.6 — WorkContext Artifact Display

**Priority:** P0
**Files:**

```text
src/cli/commands/work.rs
src/api/work_contexts.rs
src/harness/evidence.rs
```

### `prometheos work show <id>` Must Display

```text
phase
goal
latest harness run
selected files
patch history
validation result
review issues
risk level
completion decision
token usage
cost estimate
rollback command
trace ID
```

### Acceptance Criteria

```text
User can understand what happened without reading raw logs
Never says tests passed if tests did not run
Shows validation strength explicitly
Shows risk approval status
```

---

# 9. API Requirements

Add or update:

```text
POST /api/work-contexts/:id/harness/run
GET  /api/work-contexts/:id/harness/evidence
GET  /api/work-contexts/:id/harness/patches
GET  /api/work-contexts/:id/harness/validation
GET  /api/work-contexts/:id/harness/review
GET  /api/work-contexts/:id/harness/risk
GET  /api/work-contexts/:id/harness/completion
GET  /api/work-contexts/:id/quality
GET  /api/work-contexts/:id/cost
GET  /api/work-contexts/:id/traces
GET  /api/traces/:run_id
```

## API Rules

```text
No hardcoded api-user fallback
All endpoints scoped by WorkContext identity
No duplicate API-only harness path
API calls use same WorkExecutionService/HarnessContract as CLI
Failures return structured error bodies
```

---

# 10. CLI Requirements

Add:

```bash
prometheos harness run <work_context_id>
prometheos harness evidence <work_context_id>
prometheos harness patches <work_context_id>
prometheos harness rollback <patch_id>
prometheos harness validate <work_context_id>
prometheos harness review <work_context_id>
prometheos harness risk <work_context_id>
prometheos work cost <work_context_id>
prometheos work quality <work_context_id>
prometheos traces show <run_id>
prometheos traces list --work-context <work_context_id>
```

CLI output should be concise by default, detailed with `--verbose`.

---

# 11. Database Migration Requirements

Create:

```text
src/db/migrations/001_harness_161.sql
src/db/migrations/002_metrics.sql
```

Minimum tables:

```text
harness_runs
evidence_logs
repo_maps
repo_symbols
repo_dependencies
patches
git_checkpoints
validation_results
review_reports
risk_reports
attempt_pools
attempts
trace_summaries
token_costs
patch_metrics
milestones
```

Migration rules:

```text
idempotent
safe on existing DB
rollback documented
tests cover migration from empty DB and existing DB
```

---

# 12. Test Plan

## Required Tests

```text
tests/harness_161_node_registry.rs
tests/harness_161_repo_map.rs
tests/harness_161_context_budget.rs
tests/harness_161_context_distillation.rs
tests/harness_161_patch_protocol.rs
tests/harness_161_git_checkpoint.rs
tests/harness_161_validation_gate.rs
tests/harness_161_review_risk.rs
tests/harness_161_completion_policy.rs
tests/harness_161_attempt_pool.rs
tests/harness_161_observability.rs
tests/harness_161_cost_quality.rs
tests/harness_161_e2e.rs
```

## E2E Fixture

```text
tests/fixtures/rust_tiny_repo/
  Cargo.toml
  src/lib.rs
  tests/basic.rs
```

## E2E Scenario

```text
1. Create WorkContext in Software domain
2. Build RepoMap
3. Enforce token budget
4. Generate patch
5. Dry-run patch
6. Create checkpoint
7. Apply patch
8. Run cargo check/test
9. Generate review/risk
10. Evaluate completion
11. Show evidence
12. Rollback patch
```

Acceptance:

```text
cargo test passes
no raw write_file used
patch history exists
validation evidence exists
completion decision exists
trace summary exists
cost summary exists
```

---

# 13. Implementation Order

Do this in order. Not because order is glamorous, but because dependency graphs are one of the few mercies in software.

```text
1. HarnessContract + EvidenceLog
2. CompletionEvidence + phase guards
3. NodeRegistry
4. FileControl + raw write bypass block
5. Patch module extraction
6. Git checkpoint + rollback
7. RepoMap engine + cache
8. TokenBudgeter + ContextDistiller
9. ValidationRunner
10. Review/Risk reports
11. Hallucination-risk metrics
12. AttemptPool + Selector
13. OpenTelemetry spans + trace summaries
14. Cost transparency + local-first routing
15. CLI/API display
16. E2E hardening
```

---

# 14. Definition of Done

V1.6.1 is complete only when:

```text
Existing flows run unchanged
NodeRegistry replaces hardcoded factory matching
Harness nodes register without editing core factory
Software WorkContexts cannot bypass patch protocol with raw write_file
RepoMap indexes repo and produces ranked context
TokenBudgeter hard-stops over-budget prompts
ContextDistiller prevents history bloat
PatchProtocol supports dry-run/apply/rollback/history
GitCheckpoint does not capture unrelated dirty files
ValidationEvidence is required for Execution → Review
ReviewReport and RiskReport are required before finalization
CompletionDecision gates Completed phase
AttemptPool runs isolated attempts and selects one
TraceSummary is queryable after process exit
Cost and token usage visible in CLI/API
Hallucination-risk metrics are logged
work show displays patch/risk/validation/evidence/cost/rollback
E2E test passes on fixture repo
No TODO/stub/mock in production harness paths
Docs updated
Migration tested
```

---

# 15. Success Metrics

## Technical Metrics

```text
RepoMap cold index: <5s on 50k LOC
Warm symbol query: <200ms
Patch rollback success: 100% in test scenarios
Validation evidence coverage: 100% for software completion
Raw write bypass rate: 0 for software harness tasks
Trace summary persistence: 100% for harness runs
```

## Quality Metrics

```text
Patch rejection reasons recorded: 100%
Critical risk detection for obvious auth/secrets/config/db changes: >95%
Validation pass rate on small well-scoped tasks: >85%
First-attempt success rate on fixture tasks: >80%
```

## Token Metrics

```text
No LLM call exceeds budget: 100%
Prompt size reduction vs full-file baseline: >50%
Cost summary available per WorkContext: 100%
Local routing used where configured: visible and measured
```

---

# 16. Documentation Requirements

Add:

```text
docs/architecture/harness-161.md
docs/architecture/node-registry.md
docs/architecture/repo-map.md
docs/architecture/patch-protocol.md
docs/architecture/evidence-completion.md
docs/architecture/token-budgeting.md
docs/architecture/observability.md
docs/cli/harness.md
```

Docs must include:

```text
architecture overview
module map
data model
phase transition rules
patch protocol examples
rollback behavior
RepoMap behavior
token budget examples
hallucination-risk metric definition
CLI examples
API examples
known limitations
```

---

# 17. Known Limitations

Be honest in the docs, because nothing screams amateur hour like promising “zero hallucination” and then discovering compilers have opinions.

```text
Zero hallucination is treated as an enforcement goal, not an absolute guarantee.
RepoMap v0 may miss dynamic references.
Tree-sitter support varies by language.
Validation strength depends on repo test quality.
Local model routing depends on user configuration.
AttemptPool can increase short-term token use but reduce total correction loops.
Docker sandbox is optional, not mandatory.
```

---

# 18. Final Recommendation

The uploaded PRD is valid, but the final V1.6.1 should be reframed as:

```text
Harness Alignment & Enforcement
```

not:

```text
Build missing harness from scratch
```

Absorb the original six:

```text
Node Registry
Patch Protocol
RepoMap
Review Gate
Attempt Pool
Observability
```

Absorb the added four:

```text
Token Budgeter
Context Distillation
Hallucination-Risk / Evidence Metrics
Cost Transparency + Local-First Routing
```

Then add the missing enforcement spine:

```text
File Control
Safe Git Checkpoint
Completion Policy
Phase Guards
Raw Write Bypass Blocking
Trace Summary Persistence
```

That gives you the real V1.6.1.

Not a prettier architecture.
A stricter one.
Because the repo does not need more inspiration. It needs rules with teeth.
