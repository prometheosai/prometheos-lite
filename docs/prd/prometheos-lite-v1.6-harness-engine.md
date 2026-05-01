# PRD V1.6 — Harness Engine for PrometheOS Lite

## Purpose

Build the **PrometheOS Lite Harness Engine**: a deterministic, verifiable, autonomous coding execution system integrated with:

```text
WorkContext
Playbooks
WorkOrchestrator
WorkExecutionService
FlowExecutionService
Memory
Execution metadata
API/CLI execution paths
```

The Harness Engine must turn coding tasks into **real, tested, reviewable code changes**, not vague code generation. Because apparently “the model said it worked” is still considered software engineering in some neighborhoods. We will not be joining them.

---

## Current repo context

PrometheOS Lite already has:

```text
src/work/
  orchestrator.rs
  execution_service.rs
  service.rs
  types.rs
  playbook.rs
  playbook_resolver.rs
  phase_controller.rs

src/flow/
  execution_service.rs
  factory/builtin_nodes.rs
  intelligence/router.rs
  memory/

src/api/
  state.rs
  work_contexts.rs

tests/
  work_orchestrator_e2e.rs
```

Current strengths:

```text
WorkContext exists
WorkOrchestrator exists
API critical endpoints now route through WorkOrchestrator
Flow nodes fail explicitly instead of returning placeholders
Execution metadata pipeline exists at least partially
Playbook/domain flow selection exists conceptually
```

Current V1.6 gaps:

```text
No serious repo intelligence layer
No structured patch protocol
No patch selection
No attempt pool
No review harness
No sandbox runtime
No adversarial validation
No benchmark anti-overfitting system
No confidence/evidence-driven completion policy
```

---

## Non-negotiable engineering standard

Every issue must ship with:

```text
production-ready code
tests
documentation
no TODOs
no placeholders
no mock production logic
no silent failures
clear acceptance criteria
```

A task is not complete unless it works end-to-end.

---

# Target architecture

```text
User/API/CLI
→ WorkOrchestrator
→ WorkContext
→ Coding Playbook
→ Harness Engine

Harness Engine:
  1. Repo Intelligence
  2. File Control
  3. Edit Protocol
  4. Patch Application
  5. Execution Loop
  6. Validation
  7. Repair
  8. Review
  9. Selection
  10. Scaling
  11. Trajectory
  12. Sandbox
  13. Git Checkpoints
  14. WorkContext Evidence
  15. Observability
```

Recommended module structure:

```text
src/harness/
  mod.rs
  repo_intelligence.rs
  file_control.rs
  edit_protocol.rs
  patch_applier.rs
  execution_loop.rs
  validation.rs
  repair_loop.rs
  review.rs
  selection.rs
  scaling.rs
  trajectory.rs
  sandbox.rs
  git_checkpoint.rs
  observability.rs
  confidence.rs
  risk.rs
  semantic_diff.rs
  acceptance.rs
  regression_memory.rs
  environment.rs
  benchmark.rs
  artifacts.rs
```

---

# Core execution flow

```text
User request
→ WorkContext created or resumed
→ classify as coding task
→ load coding playbook
→ inspect repo
→ build repo context
→ select editable/read-only files
→ plan patch
→ generate structured edit
→ apply patch
→ validate
→ repair if needed
→ review diff
→ select best attempt if multiple
→ final validation
→ git checkpoint/commit
→ write artifacts + metadata
→ update WorkContext status
```

---

# Issue 1 — Repo Intelligence Engine

## Goal

Build a precise repo intelligence layer that identifies relevant files, symbols, and dependencies without dumping the entire repo into context like an animal.

## Files

```text
src/harness/repo_intelligence.rs
src/harness/mod.rs
tests/harness_repo_intelligence.rs
```

## Requirements

Implement:

```rust
pub struct RepoContext {
    pub root: PathBuf,
    pub ranked_files: Vec<RankedFile>,
    pub symbols: Vec<CodeSymbol>,
    pub relationships: Vec<SymbolEdge>,
    pub compressed_context: String,
    pub token_estimate: usize,
}

pub struct RankedFile {
    pub path: PathBuf,
    pub score: f32,
    pub reason: String,
}

pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}

pub enum SymbolKind {
    Function,
    Struct,
    Class,
    Enum,
    Trait,
    Interface,
    Module,
    Constant,
    Unknown,
}

pub struct SymbolEdge {
    pub from: String,
    pub to: String,
    pub relation: SymbolRelation,
}

pub enum SymbolRelation {
    Imports,
    Calls,
    Implements,
    Extends,
    References,
    Defines,
}
```

Required functions:

```rust
pub async fn build_repo_context(
    repo_root: &Path,
    task: &str,
    mentioned_files: &[PathBuf],
    mentioned_symbols: &[String],
    token_budget: usize,
) -> anyhow::Result<RepoContext>;

pub fn search_symbol(context: &RepoContext, name: &str) -> Vec<CodeSymbol>;

pub fn find_references(context: &RepoContext, symbol: &str) -> Vec<SymbolEdge>;

pub fn rank_files_by_relevance(
    files: Vec<PathBuf>,
    task: &str,
    symbols: &[CodeSymbol],
) -> Vec<RankedFile>;
```

## Implementation notes

Start with regex/light parser support for Rust, TypeScript, JavaScript, Python, Go, and generic fallback. Tree-sitter can come later, but the interface must allow pluggable parsers.

## Acceptance criteria

```text
Given a repo and task, produces ranked files
Extracts symbols from at least Rust/TS/Python
Fits compressed context into token budget
No panic on binary/large files
Tests cover empty repo, mixed-language repo, and large file skip
```

---

# Issue 2 — File Control System

## Goal

Separate files into editable, read-only, generated, and artifact sets.

## Files

```text
src/harness/file_control.rs
tests/harness_file_control.rs
```

## Requirements

Implement:

```rust
pub struct FileSet {
    pub editable: Vec<PathBuf>,
    pub readonly: Vec<PathBuf>,
    pub generated: Vec<PathBuf>,
    pub artifacts: Vec<PathBuf>,
}

pub struct FilePolicy {
    pub repo_root: PathBuf,
    pub allowed_write_paths: Vec<PathBuf>,
    pub denied_paths: Vec<PathBuf>,
    pub max_file_size_bytes: u64,
}
```

Functions:

```rust
pub fn build_file_set(
    repo_context: &RepoContext,
    explicit_files: &[PathBuf],
    policy: &FilePolicy,
) -> anyhow::Result<FileSet>;

pub fn assert_edit_allowed(path: &Path, file_set: &FileSet, policy: &FilePolicy) -> anyhow::Result<()>;
```

## Rules

```text
Only editable files can be changed
Read-only files can be used as context only
Generated files must be tracked separately
No writes outside repo root
No writes to denied paths
```

## Acceptance criteria

```text
Rejects edits outside repo root
Rejects denied paths
Allows explicit editable files
Marks dependencies as read-only
```

---

# Issue 3 — Edit Protocol

## Goal

Replace raw code generation with structured edit operations.

## Files

```text
src/harness/edit_protocol.rs
tests/harness_edit_protocol.rs
```

## Requirements

Implement:

```rust
pub enum EditOperation {
    SearchReplace(SearchReplaceEdit),
    UnifiedDiff(UnifiedDiffEdit),
    WholeFile(WholeFileEdit),
    CreateFile(CreateFileEdit),
    DeleteFile(DeleteFileEdit),
    RenameFile(RenameFileEdit),
}

pub struct SearchReplaceEdit {
    pub file: PathBuf,
    pub search: String,
    pub replace: String,
}

pub struct WholeFileEdit {
    pub file: PathBuf,
    pub content: String,
}

pub struct CreateFileEdit {
    pub file: PathBuf,
    pub content: String,
}

pub struct DeleteFileEdit {
    pub file: PathBuf,
}

pub struct RenameFileEdit {
    pub from: PathBuf,
    pub to: PathBuf,
}
```

Functions:

```rust
pub fn parse_edit_response(raw: &str) -> anyhow::Result<Vec<EditOperation>>;

pub fn validate_edit_operations(
    edits: &[EditOperation],
    file_set: &FileSet,
    policy: &FilePolicy,
) -> anyhow::Result<()>;
```

## Supported formats

```text
SEARCH_REPLACE
UNIFIED_DIFF
WHOLE_FILE
CREATE_FILE
DELETE_FILE
RENAME_FILE
```

## Acceptance criteria

```text
Rejects malformed edits
Rejects unknown files unless CREATE_FILE
Rejects DELETE/RENAME unless explicitly allowed
Parses multiple edits in one response
```

---

# Issue 4 — Patch Applier

## Goal

Apply edits safely and produce structured patch results.

## Files

```text
src/harness/patch_applier.rs
tests/harness_patch_applier.rs
```

## Requirements

Implement:

```rust
pub struct PatchResult {
    pub applied: bool,
    pub changed_files: Vec<PathBuf>,
    pub failures: Vec<PatchFailure>,
    pub diff: String,
}

pub struct PatchFailure {
    pub file: PathBuf,
    pub operation: String,
    pub reason: String,
    pub nearby_context: Option<String>,
}
```

Function:

```rust
pub async fn apply_patch(
    edits: &[EditOperation],
    file_set: &FileSet,
    policy: &FilePolicy,
) -> anyhow::Result<PatchResult>;
```

## Acceptance criteria

```text
Applies valid search/replace edits
Fails if search block not found
Returns nearby context on failure
Does not partially apply invalid batch unless transaction mode is disabled
Produces diff
```

---

# Issue 5 — Execution Loop

## Goal

Create the central harness loop.

## Files

```text
src/harness/execution_loop.rs
tests/harness_execution_loop.rs
```

## Requirements

Implement:

```rust
pub struct HarnessExecutionRequest {
    pub work_context_id: String,
    pub repo_root: PathBuf,
    pub task: String,
    pub requirements: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub mode: HarnessMode,
    pub limits: HarnessLimits,
}

pub enum HarnessMode {
    Review,
    Autonomous,
    Benchmark,
}

pub struct HarnessLimits {
    pub max_steps: u32,
    pub max_time_ms: u64,
    pub max_cost_usd: f64,
    pub max_patch_attempts: u32,
}
```

Function:

```rust
pub async fn execute_harness_task(
    req: HarnessExecutionRequest,
) -> anyhow::Result<HarnessExecutionResult>;
```

## Acceptance criteria

```text
Stops at max steps
Stops at max runtime
Fails if no patch created for coding task
Records trajectory
Returns structured result
```

---

# Issue 6 — Validation Layer

## Goal

Run format, lint, tests, and reproduction commands.

## Files

```text
src/harness/validation.rs
tests/harness_validation.rs
```

## Requirements

Implement:

```rust
pub struct ValidationPlan {
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub repro_commands: Vec<String>,
}

pub struct ValidationResult {
    pub passed: bool,
    pub command_results: Vec<CommandResult>,
    pub errors: Vec<String>,
}

pub struct CommandResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}
```

Function:

```rust
pub async fn run_validation(
    repo_root: &Path,
    plan: &ValidationPlan,
    sandbox: &dyn SandboxRuntime,
) -> anyhow::Result<ValidationResult>;
```

## Acceptance criteria

```text
Captures stdout/stderr
Marks failure on non-zero exit
Supports no-op validation plan but reports VerificationStrength::None
Times commands
```

---

# Issue 7 — Repair Loop

## Goal

Repair failed patches or failed validation automatically.

## Files

```text
src/harness/repair_loop.rs
tests/harness_repair_loop.rs
```

## Requirements

Implement:

```rust
pub struct RepairRequest {
    pub original_task: String,
    pub patch_result: Option<PatchResult>,
    pub validation_result: Option<ValidationResult>,
    pub repo_context: RepoContext,
    pub file_set: FileSet,
    pub attempt_number: u32,
}

pub struct RepairResult {
    pub edits: Vec<EditOperation>,
    pub reason: String,
}
```

Function:

```rust
pub async fn repair_failure(req: RepairRequest) -> anyhow::Result<RepairResult>;
```

## Acceptance criteria

```text
Uses structured failure info
Does not retry more than configured limit
Produces structured edits only
Fails explicitly if model returns malformed repair
```

---

# Issue 8 — Review Layer

## Goal

Analyze diffs before commit and produce risk-tagged review issues.

## Files

```text
src/harness/review.rs
tests/harness_review.rs
```

## Requirements

Implement:

```rust
pub enum IssueType {
    Bug,
    Security,
    Performance,
    Maintainability,
    Style,
    TestCoverage,
    RegressionRisk,
}

pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

pub struct ReviewIssue {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub category: IssueType,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}
```

Function:

```rust
pub async fn review_diff(
    repo_root: &Path,
    diff: &str,
    task: &str,
    acceptance_criteria: &[AcceptanceCriterion],
) -> anyhow::Result<Vec<ReviewIssue>>;
```

## Acceptance criteria

```text
Finds obvious risky changes
Blocks Critical issues
Returns file/line when available
Produces machine-readable output
```

---

# Issue 9 — Selection Engine

## Goal

Choose the best patch among multiple attempts.

## Files

```text
src/harness/selection.rs
tests/harness_selection.rs
```

## Requirements

Implement:

```rust
pub struct PatchCandidate {
    pub attempt_id: String,
    pub patch_result: PatchResult,
    pub validation_result: ValidationResult,
    pub review_issues: Vec<ReviewIssue>,
    pub confidence: ConfidenceScore,
}

pub struct SelectionResult {
    pub selected_attempt_id: String,
    pub score: f32,
    pub reason: String,
    pub rejected: Vec<RejectedCandidate>,
}
```

Function:

```rust
pub fn select_best_patch(candidates: &[PatchCandidate]) -> anyhow::Result<SelectionResult>;
```

## Scoring criteria

```text
tests passed
repro passed
no critical review issues
minimal diff
low regression risk
high confidence
```

## Acceptance criteria

```text
Rejects candidates with failed validation unless no valid candidate exists
Prefers smaller safe patch over large risky patch
Explains selection
```

---

# Issue 10 — Scaling Engine

## Goal

Support multiple independent patch attempts.

## Files

```text
src/harness/scaling.rs
tests/harness_scaling.rs
```

## Requirements

Implement:

```rust
pub enum ScalingMode {
    Single,
    Balanced,
    Max,
}

pub struct AttemptPool {
    pub attempts: Vec<HarnessAttempt>,
    pub selected: Option<String>,
}

pub struct HarnessAttempt {
    pub id: String,
    pub model_profile: String,
    pub result: Option<PatchCandidate>,
    pub trajectory_id: String,
}
```

Function:

```rust
pub async fn run_attempt_pool(
    req: HarnessExecutionRequest,
    mode: ScalingMode,
) -> anyhow::Result<AttemptPool>;
```

## Acceptance criteria

```text
Single mode runs 1 attempt
Balanced runs 3 attempts
Max runs configurable N attempts
Attempts are isolated from each other
```

---

# Issue 11 — Trajectory Recorder

## Goal

Record every execution step.

## Files

```text
src/harness/trajectory.rs
tests/harness_trajectory.rs
```

## Requirements

Implement:

```rust
pub struct Trajectory {
    pub id: String,
    pub work_context_id: String,
    pub steps: Vec<TrajectoryStep>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

pub struct TrajectoryStep {
    pub step_id: String,
    pub phase: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub tool_results: Vec<ToolResultRecord>,
    pub errors: Vec<String>,
    pub tokens: Option<u32>,
    pub duration_ms: u64,
}
```

## Acceptance criteria

```text
Records model calls
Records tool calls
Records validation commands
Can serialize to JSON
Can attach trajectory ID to WorkContext execution metadata
```

---

# Issue 12 — Sandbox Runtime

## Goal

Run commands safely in isolated environments.

## Files

```text
src/harness/sandbox.rs
tests/harness_sandbox.rs
```

## Requirements

Implement trait:

```rust
#[async_trait::async_trait]
pub trait SandboxRuntime: Send + Sync {
    async fn run_command(
        &self,
        repo_root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> anyhow::Result<CommandResult>;
}
```

Implement:

```text
LocalSandboxRuntime
DockerSandboxRuntime
```

## Acceptance criteria

```text
Local runtime works
Docker runtime is behind config flag
Timeouts are enforced
Command output is captured
```

---

# Issue 13 — Git Checkpoint System

## Goal

Make all changes reversible.

## Files

```text
src/harness/git_checkpoint.rs
tests/harness_git_checkpoint.rs
```

## Requirements

Implement:

```rust
pub struct GitCheckpoint {
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub diff: String,
    pub committed: bool,
    pub commit_message: Option<String>,
}
```

Functions:

```rust
pub async fn create_pre_task_checkpoint(repo_root: &Path) -> anyhow::Result<GitCheckpoint>;

pub async fn commit_success(
    repo_root: &Path,
    message: &str,
) -> anyhow::Result<GitCheckpoint>;

pub async fn rollback_to_checkpoint(
    repo_root: &Path,
    checkpoint: &GitCheckpoint,
) -> anyhow::Result<()>;
```

## Acceptance criteria

```text
Captures diff
Can rollback failed attempts
Can commit successful attempt
Does not commit dirty unrelated files unless configured
```

---

# Issue 14 — WorkContext Integration

## Goal

Bind Harness Engine execution into WorkContext lifecycle.

## Files

```text
src/work/execution_service.rs
src/work/orchestrator.rs
src/work/types.rs
src/harness/mod.rs
tests/harness_work_context_integration.rs
```

## Requirements

Add or use:

```rust
pub struct HarnessArtifact {
    pub kind: String,
    pub path: Option<PathBuf>,
    pub content: Option<String>,
    pub metadata: serde_json::Value,
}
```

WorkContext must record:

```text
selected files
patch diff
validation logs
review issues
confidence score
trajectory ID
git checkpoint
completion evidence
```

## Acceptance criteria

```text
Coding WorkContext routes to Harness Engine
Harness result updates WorkContext artifacts
Harness failure updates WorkContext status/errors
```

---

# Issue 15 — Observability Layer

## Goal

Expose measurable harness metrics.

## Files

```text
src/harness/observability.rs
tests/harness_observability.rs
```

## Metrics

```text
harness_task_duration_ms
harness_patch_success_total
harness_patch_failure_total
harness_validation_failure_total
harness_review_critical_total
harness_tokens_used_total
harness_cost_usd_total
harness_attempts_total
```

## Acceptance criteria

```text
Metrics emitted per harness run
Metrics include work_context_id and mode where safe
No secrets in metrics
```

---

# Issue 16 — Adversarial Validation

## Goal

Generate and run additional tests to reduce false positives.

## Files

```text
src/harness/adversarial_validation.rs
tests/harness_adversarial_validation.rs
```

## Requirements

Implement:

```rust
pub struct AdversarialTestPlan {
    pub generated_tests: Vec<GeneratedTest>,
    pub edge_cases: Vec<String>,
}

pub struct GeneratedTest {
    pub file: PathBuf,
    pub content: String,
    pub command: String,
}
```

Function:

```rust
pub async fn generate_adversarial_tests(
    task: &str,
    diff: &str,
    repo_context: &RepoContext,
) -> anyhow::Result<AdversarialTestPlan>;
```

## Acceptance criteria

```text
Generates edge cases for changed behavior
Runs generated tests when safe
Does not permanently commit generated tests unless configured
```

---

# Issue 17 — Runtime Tool Extension

## Goal

Allow task-scoped temporary tools.

## Files

```text
src/harness/runtime_tools.rs
tests/harness_runtime_tools.rs
```

## Requirements

Implement:

```rust
pub struct TaskLocalTool {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub command: String,
}
```

Functions:

```rust
pub async fn create_task_local_tool(
    work_context_id: &str,
    name: &str,
    source: &str,
) -> anyhow::Result<TaskLocalTool>;

pub async fn run_task_local_tool(
    tool: &TaskLocalTool,
    args: &[String],
    sandbox: &dyn SandboxRuntime,
) -> anyhow::Result<CommandResult>;
```

## Acceptance criteria

```text
Tools are scoped to WorkContext
Tools cannot write outside allowed paths
Tools are deleted or archived after completion
```

---

# Issue 18 — Multi-Model Strategy

## Goal

Use local and stronger models intelligently.

## Files

```text
src/harness/model_strategy.rs
src/flow/intelligence/router.rs
tests/harness_model_strategy.rs
```

## Requirements

Implement model roles:

```rust
pub enum ModelRole {
    Draft,
    Patch,
    Review,
    Judge,
    Summarize,
}
```

Function:

```rust
pub fn select_model_for_role(
    role: ModelRole,
    task_complexity: TaskComplexity,
    budget: BudgetPolicy,
) -> anyhow::Result<String>;
```

## Acceptance criteria

```text
Cheap model used for low-risk drafts
Stronger model used for final patch/review when configured
Judge model separated from generator model where possible
```

---

# Issue 19 — Failure Taxonomy

## Goal

Classify failures so recovery is targeted.

## Files

```text
src/harness/failure.rs
tests/harness_failure.rs
```

## Requirements

```rust
pub enum FailureKind {
    LocalizationFailure,
    PatchParseFailure,
    PatchApplyFailure,
    CompileFailure,
    TestFailure,
    RegressionFailure,
    SemanticFailure,
    TimeoutFailure,
    PermissionFailure,
    ModelFailure,
    ToolFailure,
    SandboxFailure,
}
```

## Acceptance criteria

```text
Every failed harness result has FailureKind
Repair strategy changes based on FailureKind
```

---

# Issue 20 — Reproduction-First Mode

## Goal

Reproduce bugs before patching when possible.

## Files

```text
src/harness/reproduction.rs
tests/harness_reproduction.rs
```

## Requirements

```rust
pub struct ReproductionEvidence {
    pub command: String,
    pub failed_before: bool,
    pub passed_after: Option<bool>,
    pub logs: String,
}
```

Function:

```rust
pub async fn attempt_reproduction(
    task: &str,
    repo_root: &Path,
    sandbox: &dyn SandboxRuntime,
) -> anyhow::Result<Option<ReproductionEvidence>>;
```

## Acceptance criteria

```text
Bug-fix tasks attempt reproduction
Feature tasks may skip with reason
Confidence increases when failed_before and passed_after are true
```

---

# Issue 21 — Acceptance Criteria Compiler

## Goal

Convert requirements into verifiable checks.

## Files

```text
src/harness/acceptance.rs
tests/harness_acceptance.rs
```

## Requirements

```rust
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub verification_method: VerificationMethod,
    pub status: CriterionStatus,
}

pub enum VerificationMethod {
    TestCommand(String),
    StaticCheck(String),
    ReviewCheck(String),
    ManualApproval,
}

pub enum CriterionStatus {
    Pending,
    Passed,
    Failed,
    NotApplicable,
}
```

## Acceptance criteria

```text
Generates criteria from WorkContext requirements
Updates status after validation/review
Completion requires criteria resolved
```

---

# Issue 22 — Confidence Calibration

## Goal

Score confidence based on evidence.

## Files

```text
src/harness/confidence.rs
tests/harness_confidence.rs
```

## Requirements

```rust
pub struct ConfidenceScore {
    pub repo_localization: f32,
    pub patch_correctness: f32,
    pub validation_strength: f32,
    pub regression_safety: f32,
    pub review_cleanliness: f32,
    pub overall: f32,
}
```

Function:

```rust
pub fn compute_confidence(
    repo_context: &RepoContext,
    patch: &PatchResult,
    validation: &ValidationResult,
    review: &[ReviewIssue],
    verification_strength: VerificationStrength,
) -> ConfidenceScore;
```

## Acceptance criteria

```text
No validation means low confidence
Critical review issue caps confidence
Reproduction pass boosts confidence
```

---

# Issue 23 — Semantic Diff Analyzer

## Goal

Classify behavioral impact of changes.

## Files

```text
src/harness/semantic_diff.rs
tests/harness_semantic_diff.rs
```

## Requirements

```rust
pub struct SemanticDiff {
    pub api_surface_changed: bool,
    pub database_changed: bool,
    pub auth_changed: bool,
    pub dependency_changed: bool,
    pub config_changed: bool,
    pub public_types_changed: bool,
    pub risk_notes: Vec<String>,
}
```

Function:

```rust
pub fn analyze_semantic_diff(diff: &str) -> SemanticDiff;
```

## Acceptance criteria

```text
Detects dependency file changes
Detects migration/config/auth-like changes
Feeds risk gates and review
```

---

# Issue 24 — Risk-Based Approval Gates

## Goal

Require approval for risky operations.

## Files

```text
src/harness/risk.rs
tests/harness_risk.rs
```

## Requirements

```rust
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

pub struct RiskAssessment {
    pub level: RiskLevel,
    pub reasons: Vec<String>,
    pub requires_approval: bool,
}
```

Function:

```rust
pub fn assess_risk(
    semantic_diff: &SemanticDiff,
    review_issues: &[ReviewIssue],
    file_set: &FileSet,
) -> RiskAssessment;
```

## Critical triggers

```text
auth
payments
database migration
secrets
production config
large deletion
dependency upgrade
```

## Acceptance criteria

```text
High/Critical risk blocks autonomous commit
Risk reasons recorded in WorkContext
```

---

# Issue 25 — Patch Minimality Enforcement

## Goal

Prevent unrelated edits.

## Files

```text
src/harness/minimality.rs
tests/harness_minimality.rs
```

## Requirements

```rust
pub struct MinimalityScore {
    pub files_changed: usize,
    pub lines_changed: usize,
    pub unrelated_change_score: f32,
    pub acceptable: bool,
}
```

Function:

```rust
pub fn score_patch_minimality(
    diff: &str,
    task: &str,
    repo_context: &RepoContext,
) -> MinimalityScore;
```

## Acceptance criteria

```text
Rejects large unrelated changes
Allows broad changes only when task requires it
Score feeds PatchSelector
```

---

# Issue 26 — Regression Memory

## Goal

Remember regressions caused by prior changes.

## Files

```text
src/harness/regression_memory.rs
tests/harness_regression_memory.rs
```

## Requirements

```rust
pub struct RegressionMemory {
    pub pattern: String,
    pub triggering_change: String,
    pub failed_test: String,
    pub prevention_rule: String,
}
```

Functions:

```rust
pub async fn record_regression(memory: RegressionMemory) -> anyhow::Result<()>;

pub async fn retrieve_relevant_regressions(
    task: &str,
    diff: &str,
) -> anyhow::Result<Vec<RegressionMemory>>;
```

## Acceptance criteria

```text
Failed post-validation can be stored
Relevant memories influence validation/review
```

---

# Issue 27 — Golden Path Templates

## Goal

Use strong templates for common coding tasks.

## Files

```text
src/harness/golden_paths.rs
tests/harness_golden_paths.rs
```

## Templates

```text
Add API endpoint
Fix failing test
Add DB migration
Refactor module
Add UI component
Fix security issue
Improve performance
Add unit tests
```

## Requirements

```rust
pub struct GoldenPathTemplate {
    pub name: String,
    pub required_discovery: Vec<String>,
    pub required_validation: Vec<String>,
    pub risk_gates: Vec<String>,
    pub expected_artifacts: Vec<String>,
}
```

## Acceptance criteria

```text
Task classifier maps task to template
Template affects validation and approval gates
```

---

# Issue 28 — Environment Fingerprinting

## Goal

Detect how the repo runs before executing.

## Files

```text
src/harness/environment.rs
tests/harness_environment.rs
```

## Requirements

```rust
pub struct EnvironmentProfile {
    pub languages: Vec<String>,
    pub package_manager: Option<String>,
    pub test_commands: Vec<String>,
    pub services: Vec<ServiceDependency>,
}

pub struct ServiceDependency {
    pub name: String,
    pub required: bool,
    pub startup_command: Option<String>,
}
```

Function:

```rust
pub async fn fingerprint_environment(repo_root: &Path) -> anyhow::Result<EnvironmentProfile>;
```

## Acceptance criteria

```text
Detects Cargo/npm/pnpm/yarn/pip/go
Finds likely test commands
Stores profile as artifact
```

---

# Issue 29 — Task-Local Knowledge Cache

## Goal

Separate task-local knowledge from long-term memory.

## Files

```text
src/harness/task_cache.rs
tests/harness_task_cache.rs
```

## Requirements

```rust
pub struct TaskLocalCache {
    pub work_context_id: String,
    pub facts: Vec<TaskFact>,
    pub discovered_files: Vec<PathBuf>,
    pub temporary_tools: Vec<PathBuf>,
}
```

## Acceptance criteria

```text
Stores discoveries during task
Cleared or archived after task
Does not pollute user/global memory automatically
```

---

# Issue 30 — Verification Strength Levels

## Goal

Report how strongly the task was verified.

## Files

```text
src/harness/verification.rs
tests/harness_verification.rs
```

## Requirements

```rust
pub enum VerificationStrength {
    None,
    StaticOnly,
    ExistingTests,
    ReproductionTest,
    GeneratedEdgeTests,
    AdversarialTests,
    ManualApproval,
}
```

Function:

```rust
pub fn determine_verification_strength(
    validation: &ValidationResult,
    reproduction: Option<&ReproductionEvidence>,
    adversarial: Option<&AdversarialTestPlan>,
) -> VerificationStrength;
```

## Acceptance criteria

```text
Final result includes verification strength
Completion confidence depends on strength
```

---

# Issue 31 — Tool Permission Ledger

## Goal

Authorize and record all tool actions.

## Files

```text
src/harness/permissions.rs
tests/harness_permissions.rs
```

## Requirements

```rust
pub struct ToolPermission {
    pub tool: String,
    pub scope: PermissionScope,
    pub allowed_paths: Vec<PathBuf>,
    pub requires_approval: bool,
}

pub enum PermissionScope {
    ReadOnly,
    RepoWrite,
    Shell,
    Network,
    GitCommit,
    GitPush,
}
```

## Acceptance criteria

```text
Shell/write/network actions checked before execution
Denied actions fail loudly
Ledger attached to trajectory
```

---

# Issue 32 — Benchmark Anti-Overfitting Protocol

## Goal

Prevent leaderboard-only optimization.

## Files

```text
src/harness/benchmark.rs
tests/harness_benchmark.rs
```

## Requirements

```rust
pub struct BenchmarkTask {
    pub id: String,
    pub repo: PathBuf,
    pub issue: String,
    pub hidden_tests: Vec<String>,
    pub category: BenchmarkCategory,
}

pub enum BenchmarkCategory {
    Public,
    Private,
    Synthetic,
    Adversarial,
    Multimodal,
}
```

## Acceptance criteria

```text
Supports private benchmark tasks
Supports synthetic bug generation hooks
Reports pass rate, cost, time, attempts, and confidence
```

---

# Issue 33 — PR / Release Artifact Generator

## Goal

Generate handoff artifacts for every completed coding task.

## Files

```text
src/harness/artifacts.rs
tests/harness_artifacts.rs
```

## Output

```text
summary
changed files
why changed
tests run
review result
risk remaining
rollback instructions
confidence
```

## Function

```rust
pub fn generate_completion_artifact(
    result: &HarnessExecutionResult,
) -> anyhow::Result<String>;
```

## Acceptance criteria

```text
Artifact attached to WorkContext
Useful without reading raw logs
No fake claims about tests not run
```

---

# Issue 34 — Time-Travel Debugging

## Goal

Replay what the harness knew and did.

## Files

```text
src/harness/time_travel.rs
tests/harness_time_travel.rs
```

## Requirements

```rust
pub struct ReplaySnapshot {
    pub work_context_id: String,
    pub trajectory_id: String,
    pub repo_context: RepoContext,
    pub file_set: FileSet,
    pub selected_attempt: Option<String>,
    pub final_diff: Option<String>,
}
```

Functions:

```rust
pub async fn create_replay_snapshot(
    result: &HarnessExecutionResult,
) -> anyhow::Result<ReplaySnapshot>;

pub async fn replay_trajectory(snapshot: &ReplaySnapshot) -> anyhow::Result<()>;
```

## Acceptance criteria

```text
Can inspect task state after completion
Can replay major decisions without rerunning model calls
```

---

# Issue 35 — Evidence-Based Completion Policy

## Goal

Prevent fake completion.

## Files

```text
src/harness/completion.rs
tests/harness_completion.rs
```

## Requirements

```rust
pub struct CompletionEvidence {
    pub patch_exists: bool,
    pub validation_ran: bool,
    pub validation_passed: bool,
    pub review_ran: bool,
    pub critical_issues: usize,
    pub confidence: ConfidenceScore,
    pub verification_strength: VerificationStrength,
}
```

Function:

```rust
pub fn evaluate_completion(
    evidence: &CompletionEvidence,
    mode: HarnessMode,
) -> anyhow::Result<CompletionDecision>;

pub enum CompletionDecision {
    Complete,
    Blocked(String),
    NeedsRepair(String),
    NeedsApproval(String),
}
```

## Rules

```text
No patch = not complete for coding task
No validation = not complete unless explicitly manual/research-only
Critical review issue = blocked
Low confidence = needs approval
```

## Acceptance criteria

```text
WorkContext can only move to Completed through this policy
Final API/CLI response includes evidence summary
```

---

# Final integration requirements

## API

Add endpoints:

```text
POST /api/work-contexts/:id/harness/run
GET  /api/work-contexts/:id/harness/trajectory
GET  /api/work-contexts/:id/harness/artifacts
GET  /api/work-contexts/:id/harness/confidence
```

## CLI

Add:

```text
prometheos work harness run <context_id>
prometheos work harness replay <context_id>
prometheos work harness benchmark
```

## Tests

Minimum required test suites:

```text
tests/harness_repo_intelligence.rs
tests/harness_edit_protocol.rs
tests/harness_patch_applier.rs
tests/harness_validation.rs
tests/harness_review.rs
tests/harness_selection.rs
tests/harness_work_context_integration.rs
tests/harness_completion.rs
```

## Documentation

Add:

```text
docs/v1.6-harness-engine.md
docs/harness-execution-flow.md
docs/harness-benchmarking.md
```

---

# Final definition of done for V1.6

V1.6 is complete only when:

```text
A coding WorkContext can run through the Harness Engine
A real patch is produced
The patch is applied safely
Validation runs
Review runs
Completion evidence is produced
Trajectory is recorded
Artifacts are attached to WorkContext
API and CLI both execute the same path
Tests cover the full lifecycle
No stubs/placeholders/TODOs exist in production paths
```

This is the standard. Anything less is just a very organized way to disappoint yourself later.
