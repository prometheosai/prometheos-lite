# PrometheOS Lite V1.6 PRD — Harness Engine

## Executive Summary

PrometheOS Lite V1.6 ships the **Harness Engine**: the missing “hands, eyes, spine, and courtroom” of the system. The existing product already has the high-level architecture: `WorkContext`, `WorkOrchestrator`, `FlowExecutionService`, playbooks, memory, execution metadata, API/CLI paths, tracing, and local-first Rust infrastructure. The previous V1.6 plans correctly define the target as a deterministic coding execution system that turns coding tasks into **real, tested, reviewable code changes**, not “the model wrote something and everyone clapped like civilization hasn’t learned anything.” 

This PRD keeps the original 35-issue structure, preserves all major functionality from the previous V1.6 plan, and folds in the consensus from the architecture analysis: **structured patching, RepoMap, mandatory validation, evidence logs, sandboxing, attempt pools, review/risk gates, OpenTelemetry, WorkContext integration, and evidence-based completion**. The original implementation plan already organizes V1.6 into 5 epics and 35 issues, with modules under `src/harness/`, API endpoints, CLI commands, tests, docs, dependencies, and definition of done. 

---

# Product Goal

Build a production-grade Harness Engine that allows PrometheOS Lite to safely execute software engineering work inside real repositories.

The Harness Engine must:

```text
Inspect a repo
Understand task context
Select relevant files
Generate structured edits
Apply patches safely
Rollback on failure
Run validation
Repair failed attempts
Review risk
Compare multiple attempts
Record full trajectory
Attach evidence to WorkContext
Finalize only when completion policy passes
```

The core positioning:

> PrometheOS Lite is not merely a coding assistant. It is a **WorkContext-native, local-first execution OS** where coding tasks are validated, reviewable, replayable, and reversible.

---

# Non-Negotiable Engineering Standard

Every issue must ship with:

```text
Production-ready code
Tests
Documentation
No TODOs
No placeholders
No mock production logic
No silent failures
Clear acceptance criteria
Integration into WorkContext where relevant
```

A V1.6 feature is not complete unless it works through the real execution path.

No “we’ll wire it later.” That phrase belongs in a museum of preventable bugs.

---

# Current Repo Context

V1.6 assumes these systems already exist and must be integrated, not duplicated:

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
  factory/
  intelligence/
  memory/
  tracing/
  budget/
  policy/

src/api/
  state.rs
  work_contexts.rs

src/cli/
  commands/
  runtime_builder.rs
```

Existing strengths:

```text
WorkContext exists
WorkOrchestrator exists
FlowExecutionService exists
Playbook/domain routing exists conceptually
Execution metadata exists partially
Memory and tracing exist
Policy/rate limiting exists
API and CLI paths exist
```

V1.6 gaps:

```text
No serious RepoMap / repo intelligence layer
No structured patch protocol
No safe patch applier with rollback
No mandatory validation gate
No review/risk engine
No attempt pool / patch selection
No adversarial validation
No evidence-based completion policy
No full trajectory replay
No tool permission ledger
No runtime task-local tools
No benchmark anti-overfitting protocol
```

---

# Target Architecture

```text
User/API/CLI
  ↓
WorkOrchestrator
  ↓
WorkContext
  ↓
Coding Playbook
  ↓
Harness Engine
  ↓
Repo Intelligence
  ↓
File Control
  ↓
Edit Protocol
  ↓
Patch Applier
  ↓
Git Checkpoint
  ↓
Sandboxed Validation
  ↓
Repair Loop
  ↓
Review Layer
  ↓
Risk Gate
  ↓
Selection Engine
  ↓
Evidence Log
  ↓
Completion Policy
  ↓
WorkContext Artifacts + Trajectory
```

The previous PRD already names the key integration points: `WorkContext`, `Playbooks`, `WorkOrchestrator`, `WorkExecutionService`, `FlowExecutionService`, memory, execution metadata, and API/CLI execution paths. 

---

# Module Structure

Recommended final structure:

```text
src/harness/
  mod.rs

  repo_intelligence.rs
  file_control.rs
  edit_protocol.rs
  patch_applier.rs
  environment.rs

  execution_loop.rs
  validation.rs
  repair_loop.rs
  failure.rs
  reproduction.rs
  task_cache.rs

  review.rs
  selection.rs
  scaling.rs
  adversarial_validation.rs
  confidence.rs
  semantic_diff.rs
  risk.rs
  minimality.rs
  verification.rs

  trajectory.rs
  sandbox.rs
  git_checkpoint.rs
  observability.rs
  runtime_tools.rs
  permissions.rs
  time_travel.rs

  model_strategy.rs
  acceptance.rs
  regression_memory.rs
  golden_paths.rs
  benchmark.rs
  artifacts.rs
  completion.rs
```

This preserves the original module plan from the uploaded V1.6 implementation plan. 

---

# Epic 1 — Foundation: Repo Intelligence, File Control & Patch Safety

## Objective

Give the harness the ability to understand repositories, control what can be touched, and apply edits safely.

This epic builds the “eyes” and “hands” of the system. Without it, the agent is just a confident text generator wandering around a codebase with scissors.

---

## Issue 1 — Repo Intelligence Engine

**Module:** `src/harness/repo_intelligence.rs`
**Priority:** P0
**Dependencies:** None

### Goal

Build a repository intelligence layer that identifies files, symbols, dependencies, entrypoints, test files, and relevant context for a coding task.

### Requirements

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

### Implementation Notes

Start with:

```text
walkdir
ignore
regex parsers
language-specific extractors for Rust, TypeScript, JavaScript, Python, Go
generic fallback parser
large/binary file skip
```

Tree-sitter should be supported through a pluggable parser interface, but V1.6 must not depend on perfect AST parsing to work.

### Acceptance Criteria

```text
Produces ranked files for a repo/task
Extracts symbols from Rust, TS/JS, Python
Skips binary and huge files safely
Builds compressed context within token budget
Can find symbol references
Can detect likely test files and entrypoints
No panic on empty repo
```

---

## Issue 2 — Environment Fingerprinting

**Module:** `src/harness/environment.rs`
**Priority:** P0
**Dependencies:** None

### Goal

Detect how the repository builds, tests, and runs before execution begins.

### Requirements

```rust
pub struct EnvironmentProfile {
    pub languages: Vec<String>,
    pub package_manager: Option<String>,
    pub build_commands: Vec<String>,
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
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

### Detection Targets

```text
Cargo.toml
package.json
pnpm-lock.yaml
yarn.lock
package-lock.json
pyproject.toml
requirements.txt
go.mod
Makefile
Dockerfile
docker-compose.yml
```

### Acceptance Criteria

```text
Detects Rust/Cargo projects
Detects npm/pnpm/yarn projects
Detects Python projects
Detects Go projects
Finds likely format/lint/test commands
Stores EnvironmentProfile as harness artifact
```

---

## Issue 3 — File Control System

**Module:** `src/harness/file_control.rs`
**Priority:** P0
**Dependencies:** Issues 1, 2

### Goal

Separate editable, read-only, generated, denied, and artifact files.

### Requirements

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
    pub allow_delete: bool,
    pub allow_rename: bool,
    pub allow_generated_edits: bool,
}
```

Functions:

```rust
pub fn build_file_set(
    repo_context: &RepoContext,
    explicit_files: &[PathBuf],
    policy: &FilePolicy,
) -> anyhow::Result<FileSet>;

pub fn assert_edit_allowed(
    path: &Path,
    file_set: &FileSet,
    policy: &FilePolicy,
) -> anyhow::Result<()>;
```

### Rules

```text
No writes outside repo root
No writes to denied paths
Generated files are not editable unless explicitly allowed
Secrets/env files are denied by default
Lockfiles require risk assessment
Delete/rename require explicit permission
```

### Acceptance Criteria

```text
Rejects path traversal
Rejects writes outside repo
Rejects denied paths
Marks dependencies/read-only files
Marks generated files
Supports explicit editable file allowlist
```

---

## Issue 4 — Edit Protocol

**Module:** `src/harness/edit_protocol.rs`
**Priority:** P0
**Dependencies:** Issue 3

### Goal

Replace raw code generation with structured edits.

### Requirements

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

pub struct UnifiedDiffEdit {
    pub diff: String,
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

### Supported Formats

```text
SEARCH_REPLACE
UNIFIED_DIFF
WHOLE_FILE
CREATE_FILE
DELETE_FILE
RENAME_FILE
```

### Acceptance Criteria

```text
Parses multiple edits from one model response
Rejects malformed edits
Rejects unknown files unless CREATE_FILE
Rejects DELETE/RENAME unless allowed
Rejects ambiguous search blocks
Produces structured parse errors
```

---

## Issue 5 — Patch Applier + Transaction Safety

**Module:** `src/harness/patch_applier.rs`
**Priority:** P0
**Dependencies:** Issue 4

### Goal

Apply edits safely with dry-run, transaction semantics, structured failure reporting, and diff output.

### Requirements

```rust
pub struct PatchResult {
    pub applied: bool,
    pub changed_files: Vec<PathBuf>,
    pub failures: Vec<PatchFailure>,
    pub diff: String,
    pub dry_run: bool,
}

pub struct PatchFailure {
    pub file: PathBuf,
    pub operation: String,
    pub reason: String,
    pub nearby_context: Option<String>,
}
```

Functions:

```rust
pub async fn dry_run_patch(
    edits: &[EditOperation],
    file_set: &FileSet,
    policy: &FilePolicy,
) -> anyhow::Result<PatchResult>;

pub async fn apply_patch(
    edits: &[EditOperation],
    file_set: &FileSet,
    policy: &FilePolicy,
) -> anyhow::Result<PatchResult>;
```

### Acceptance Criteria

```text
Dry-run validates without writing
Search/replace requires unique match
Fails if search block not found
Returns nearby context on failure
Applies batch atomically by default
Produces unified diff
Never partially applies invalid batch unless explicitly configured
```

---

# Epic 2 — Execution: Harness Loop, Validation, Repair & Reproduction

## Objective

Create the central execution loop that turns WorkContext coding tasks into controlled harness runs with validation, repair, reproduction, failure classification, and task-local memory.

This epic turns scattered tools into an actual engine. Civilization may yet continue.

---

## Issue 6 — Harness Execution Loop

**Module:** `src/harness/execution_loop.rs`
**Priority:** P0
**Dependencies:** Epic 1

### Requirements

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

Main function:

```rust
pub async fn execute_harness_task(
    req: HarnessExecutionRequest,
) -> anyhow::Result<HarnessExecutionResult>;
```

### Execution Flow

```text
Start trajectory
Fingerprint environment
Build RepoContext
Build FileSet
Compile acceptance criteria
Create attempt(s)
Generate structured edits
Dry-run patch
Create git checkpoint
Apply patch
Run validation
Repair if needed
Review diff
Assess risk
Compute confidence
Generate artifacts
Evaluate completion
Update WorkContext
```

### Acceptance Criteria

```text
Stops at max steps
Stops at max runtime
Stops at max attempt count
Fails if no patch is produced for coding task
Records trajectory
Returns structured result
Does not swallow errors
```

---

## Issue 7 — Validation Layer

**Module:** `src/harness/validation.rs`
**Priority:** P0
**Dependencies:** Issue 6

### Requirements

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

### Acceptance Criteria

```text
Runs format/lint/test/repro commands
Captures stdout/stderr
Marks failure on non-zero exit
Supports timeout
Supports no-op plan but reports VerificationStrength::None
Attaches command evidence to trajectory
```

---

## Issue 8 — Failure Taxonomy

**Module:** `src/harness/failure.rs`
**Priority:** P0
**Dependencies:** Issues 6, 7

### Requirements

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

Functions:

```rust
pub fn classify_patch_failure(failure: &PatchFailure) -> FailureKind;

pub fn classify_validation_failure(result: &ValidationResult) -> FailureKind;
```

### Acceptance Criteria

```text
Every failed harness result has FailureKind
Repair strategy changes based on FailureKind
FailureKind stored in trajectory and WorkContext metadata
```

---

## Issue 9 — Reproduction-First Mode

**Module:** `src/harness/reproduction.rs`
**Priority:** P1
**Dependencies:** Issue 7

### Goal

Bug-fix tasks should attempt to reproduce the failure before patching.

### Requirements

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

### Acceptance Criteria

```text
Bug-fix tasks attempt reproduction
Feature/refactor tasks may skip with explicit reason
failed_before=true and passed_after=true boosts confidence
Reproduction logs attach to EvidenceLog
```

---

## Issue 10 — Repair Loop

**Module:** `src/harness/repair_loop.rs`
**Priority:** P0
**Dependencies:** Issues 6, 7, 8

### Requirements

```rust
pub struct RepairRequest {
    pub original_task: String,
    pub patch_result: Option<PatchResult>,
    pub validation_result: Option<ValidationResult>,
    pub failure_kind: FailureKind,
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

### Acceptance Criteria

```text
Uses structured failure info
Does not retry beyond configured limit
Produces structured edits only
Fails explicitly on malformed repair output
Records repair attempts in trajectory
```

---

## Issue 11 — Task-Local Knowledge Cache

**Module:** `src/harness/task_cache.rs`
**Priority:** P1
**Dependencies:** Issue 6

### Requirements

```rust
pub struct TaskLocalCache {
    pub work_context_id: String,
    pub facts: Vec<TaskFact>,
    pub discovered_files: Vec<PathBuf>,
    pub temporary_tools: Vec<PathBuf>,
}

pub struct TaskFact {
    pub key: String,
    pub value: String,
    pub source: String,
}
```

### Acceptance Criteria

```text
Stores discoveries during task
Can be archived after task
Does not pollute global memory automatically
Feeds repair/review/selection context
```

---

## Issue 12 — Acceptance Criteria Compiler

**Module:** `src/harness/acceptance.rs`
**Priority:** P0
**Dependencies:** Issue 6

### Requirements

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

### Acceptance Criteria

```text
Generates criteria from WorkContext requirements
Maps criteria to validation/review/manual approval
Updates status after validation/review
Completion requires criteria resolved
```

---

# Epic 3 — Quality: Review, Risk, Selection, Attempts & Verification

## Objective

Make PrometheOS choose safe, minimal, validated patches instead of accepting the first patch-shaped object a model emits.

This epic adds the judgment layer: review, risk, confidence, semantic diffing, adversarial validation, patch selection, and attempt pools.

---

## Issue 13 — Review Layer

**Module:** `src/harness/review.rs`
**Priority:** P0
**Dependencies:** Issues 5, 7, 12

### Requirements

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

### Acceptance Criteria

```text
Finds obvious risky changes
Blocks Critical issues
Returns file/line when available
Produces machine-readable issues
Attaches review report to WorkContext
```

---

## Issue 14 — Semantic Diff Analyzer

**Module:** `src/harness/semantic_diff.rs`
**Priority:** P0
**Dependencies:** Issue 5

### Requirements

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

### Acceptance Criteria

```text
Detects dependency file changes
Detects DB/migration changes
Detects auth/security/config changes
Detects public API/type changes
Feeds risk gates and review
```

---

## Issue 15 — Patch Minimality Enforcement

**Module:** `src/harness/minimality.rs`
**Priority:** P0
**Dependencies:** Issues 1, 5

### Requirements

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

### Acceptance Criteria

```text
Rejects large unrelated changes
Allows broad changes only when task requires it
Feeds selection and confidence
Records minimality score as evidence
```

---

## Issue 16 — Risk-Based Approval Gates

**Module:** `src/harness/risk.rs`
**Priority:** P0
**Dependencies:** Issues 13, 14, 15

### Requirements

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

### Critical Triggers

```text
auth
payments
database migration
secrets
production config
large deletion
dependency upgrade
Git push
network side effects
```

### Acceptance Criteria

```text
High/Critical risk blocks autonomous commit
Approval requirement is recorded
Risk reasons stored in WorkContext metadata
```

---

## Issue 17 — Verification Strength Levels

**Module:** `src/harness/verification.rs`
**Priority:** P0
**Dependencies:** Issues 7, 9

### Requirements

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

### Acceptance Criteria

```text
Final result includes verification strength
No validation reports VerificationStrength::None
Confidence depends on verification strength
Completion policy uses verification strength
```

---

## Issue 18 — Adversarial Validation

**Module:** `src/harness/adversarial_validation.rs`
**Priority:** P1
**Dependencies:** Issues 7, 13, 17

### Requirements

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

### Acceptance Criteria

```text
Generates edge cases for changed behavior
Runs generated tests when safe
Does not permanently commit generated tests unless configured
Records generated tests as artifacts
```

---

## Issue 19 — Confidence Calibration

**Module:** `src/harness/confidence.rs`
**Priority:** P0
**Dependencies:** Issues 7, 13, 17, 18

### Requirements

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

### Acceptance Criteria

```text
No validation means low confidence
Critical review issue caps confidence
Failed tests cap confidence
Reproduction pass boosts confidence
Confidence included in API/CLI output
```

---

## Issue 20 — Selection Engine

**Module:** `src/harness/selection.rs`
**Priority:** P0
**Dependencies:** Issues 13–19

### Requirements

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

### Scoring Criteria

```text
Validation passed
Reproduction passed
No critical review issues
Minimal diff
Low semantic risk
High confidence
Acceptance criteria satisfied
```

### Acceptance Criteria

```text
Rejects failed validation unless no valid candidate exists
Prefers smaller safe patch over large risky patch
Explains selected and rejected candidates
Records selection result in trajectory
```

---

## Issue 21 — Scaling Engine / Attempt Pool

**Module:** `src/harness/scaling.rs`
**Priority:** P1
**Dependencies:** Issues 6, 20

### Requirements

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

### Acceptance Criteria

```text
Single runs 1 attempt
Balanced runs 3 attempts
Max runs configurable N attempts
Attempts are isolated
Only selected patch is applied to final workspace
Rejected attempts preserve artifacts for audit
```

---

# Epic 4 — Infrastructure: Sandbox, Permissions, Git, Trajectory & Observability

## Objective

Make every harness action isolated, auditable, reversible, measurable, and replayable.

This is the part where the system stops behaving like a chat window with ambition and starts behaving like infrastructure.

---

## Issue 22 — Sandbox Runtime

**Module:** `src/harness/sandbox.rs`
**Priority:** P0
**Dependencies:** Issue 7

### Requirements

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
DenoSandboxRuntime
DockerSandboxRuntime
```

### Runtime Rules

```text
Per-invocation process isolation
Timeout enforced
Working directory restricted
Environment allowlist
Output size limit
Command allowlist
No shared mutable process state across invocations
```

### Acceptance Criteria

```text
Local runtime works by default
Deno runtime supports task-local tools/scripts
Docker runtime behind config flag
Timeouts enforced
stdout/stderr captured
Denied commands fail loudly
```

---

## Issue 23 — Tool Permission Ledger

**Module:** `src/harness/permissions.rs`
**Priority:** P0
**Dependencies:** Issue 22

### Requirements

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

### Acceptance Criteria

```text
Read/write/shell/network/git actions checked before execution
Denied actions fail loudly
Approval-required actions return NeedsApproval
Ledger attached to trajectory
No silent permission fallback
```

---

## Issue 24 — Git Checkpoint System

**Module:** `src/harness/git_checkpoint.rs`
**Priority:** P0
**Dependencies:** Issues 5, 6, 23

### Requirements

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

### Acceptance Criteria

```text
Captures pre-task dirty state
Rejects commit of unrelated dirty files unless configured
Can rollback failed attempts
Can commit successful attempt
Stores before/after diff
```

---

## Issue 25 — Trajectory Recorder

**Module:** `src/harness/trajectory.rs`
**Priority:** P0
**Dependencies:** Issue 6

### Requirements

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

### Acceptance Criteria

```text
Records model calls
Records tool calls
Records patch operations
Records validation commands
Records review/risk/completion decisions
Serializes to JSON
Attaches trajectory ID to WorkContext metadata
```

---

## Issue 26 — Observability Layer / OpenTelemetry

**Module:** `src/harness/observability.rs`
**Priority:** P1
**Dependencies:** Issues 6, 22, 25

### Goal

Export harness metrics and traces to console/file by default, and OTLP/Jaeger when configured.

### Metrics

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

### Required Spans

```text
harness.execute
repo_context.build
environment.fingerprint
patch.parse
patch.dry_run
patch.apply
validation.run
repair.run
review.run
risk.assess
selection.run
completion.evaluate
```

### Acceptance Criteria

```text
Metrics emitted per harness run
Spans include work_context_id where safe
No secrets in traces or metrics
Jaeger/OTLP export behind config
Local trace output works without Jaeger
```

---

## Issue 27 — Runtime Tool Extension

**Module:** `src/harness/runtime_tools.rs`
**Priority:** P1
**Dependencies:** Issues 22, 23

### Requirements

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

### Acceptance Criteria

```text
Tools scoped to WorkContext
Tools cannot write outside allowed paths
Tools run through sandbox
Tools are deleted or archived after completion
Permission ledger records usage
```

---

## Issue 28 — Time-Travel Debugging

**Module:** `src/harness/time_travel.rs`
**Priority:** P1
**Dependencies:** Issues 24, 25

### Requirements

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

### Acceptance Criteria

```text
Can inspect task state after completion
Can replay major decisions without model calls
Can show what files/context the model saw
Can show why a patch was selected or rejected
```

---

# Epic 5 — Intelligence: WorkContext Integration, Memory, Models, Benchmarks & Completion

## Objective

Connect the harness to PrometheOS’s actual product layer: WorkContext, playbooks, memory, model strategy, benchmark mode, artifacts, and evidence-based completion.

This epic makes the harness part of PrometheOS Lite instead of a shiny sidecar living in the garage.

---

## Issue 29 — WorkContext Integration

**Module:** `src/work/execution_service.rs`, `src/work/orchestrator.rs`, `src/work/types.rs`, `src/harness/mod.rs`
**Priority:** P0
**Dependencies:** Issues 1–28

### Requirements

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
repo context summary
environment profile
selected files
patch diff
validation logs
review issues
risk assessment
confidence score
trajectory ID
git checkpoint
completion evidence
artifact summary
```

### Acceptance Criteria

```text
Coding WorkContext routes to Harness Engine
Harness success updates WorkContext artifacts
Harness failure updates WorkContext status/errors
API and CLI use same harness execution path
No duplicate execution logic
```

---

## Issue 30 — Multi-Model Strategy

**Module:** `src/harness/model_strategy.rs`, `src/flow/intelligence/router.rs`
**Priority:** P1
**Dependencies:** Issue 6

### Requirements

```rust
pub enum ModelRole {
    Draft,
    Patch,
    Review,
    Judge,
    Summarize,
}

pub enum TaskComplexity {
    Low,
    Medium,
    High,
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

### Acceptance Criteria

```text
Cheap model used for low-risk drafts
Stronger model used for patch/review when configured
Judge model separated from generator model where possible
Model choice recorded in trajectory
Budget policy respected
```

---

## Issue 31 — Golden Path Templates

**Module:** `src/harness/golden_paths.rs`
**Priority:** P1
**Dependencies:** Issues 6, 12

### Templates

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

### Requirements

```rust
pub struct GoldenPathTemplate {
    pub name: String,
    pub required_discovery: Vec<String>,
    pub required_validation: Vec<String>,
    pub risk_gates: Vec<String>,
    pub expected_artifacts: Vec<String>,
}
```

### Acceptance Criteria

```text
Task classifier maps task to template
Template affects validation plan
Template affects risk gates
Template affects expected artifacts
```

---

## Issue 32 — Regression Memory

**Module:** `src/harness/regression_memory.rs`
**Priority:** P1
**Dependencies:** Issues 7, 10

### Requirements

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

### Acceptance Criteria

```text
Failed post-validation can be stored
Relevant regressions influence validation/review
Regression memory does not pollute task-local cache automatically
```

---

## Issue 33 — Benchmark Anti-Overfitting Protocol

**Module:** `src/harness/benchmark.rs`
**Priority:** P1
**Dependencies:** Issues 20, 21

### Requirements

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

### Acceptance Criteria

```text
Supports private benchmark tasks
Supports synthetic bug generation hooks
Reports pass rate, cost, time, attempts, confidence
Prevents optimizing only for visible tests
Benchmark output includes trajectory/artifacts
```

---

## Issue 34 — PR / Release Artifact Generator

**Module:** `src/harness/artifacts.rs`
**Priority:** P0
**Dependencies:** Issues 6–20

### Output Must Include

```text
Summary
Changed files
Why changed
Patch diff
Tests run
Validation result
Review result
Risk remaining
Rollback instructions
Confidence
Verification strength
Completion decision
```

Function:

```rust
pub fn generate_completion_artifact(
    result: &HarnessExecutionResult,
) -> anyhow::Result<String>;
```

### Acceptance Criteria

```text
Artifact attached to WorkContext
Useful without reading raw logs
Never claims tests passed if they did not run
Includes rollback instructions
Available through API and CLI
```

---

## Issue 35 — Evidence-Based Completion Policy

**Module:** `src/harness/completion.rs`
**Priority:** P0
**Dependencies:** Issues 7, 13, 17, 19, 34

### Requirements

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

pub enum CompletionDecision {
    Complete,
    Blocked(String),
    NeedsRepair(String),
    NeedsApproval(String),
}
```

Function:

```rust
pub fn evaluate_completion(
    evidence: &CompletionEvidence,
    mode: HarnessMode,
) -> anyhow::Result<CompletionDecision>;
```

### Completion Rules

```text
No patch = not complete for coding task
No validation = not complete unless explicitly manual/research-only
Failed validation = NeedsRepair or Blocked
Critical review issue = Blocked
High/Critical risk = NeedsApproval
Low confidence = NeedsApproval
Benchmark mode requires verification result
```

### Acceptance Criteria

```text
WorkContext can only move to Completed through this policy
Final API/CLI response includes evidence summary
Completion decision is stored in WorkContext
No fake completion possible through harness path
```

---

# API Requirements

Add:

```text
POST /api/work-contexts/:id/harness/run
GET  /api/work-contexts/:id/harness/trajectory
GET  /api/work-contexts/:id/harness/artifacts
GET  /api/work-contexts/:id/harness/confidence
GET  /api/work-contexts/:id/harness/replay
GET  /api/work-contexts/:id/harness/risk
GET  /api/work-contexts/:id/harness/completion
```

Rules:

```text
All endpoints use WorkContext identity
No hardcoded api-user fallback
All harness runs go through WorkOrchestrator/WorkExecutionService
No duplicate API-only execution path
```

The previous plan already included the core four endpoints for run, trajectory, artifacts, and confidence. 

---

# CLI Requirements

Add:

```text
prometheos work harness run <context_id>
prometheos work harness replay <context_id>
prometheos work harness benchmark
prometheos work harness artifact <context_id>
prometheos work harness risk <context_id>
prometheos work harness completion <context_id>
```

CLI output must show:

```text
WorkContext ID
Selected files
Patch summary
Validation result
Review result
Risk level
Confidence
Verification strength
Completion decision
Artifact path
Rollback command
```

---

# Required Tests

Minimum test files:

```text
tests/harness_repo_intelligence.rs
tests/harness_environment.rs
tests/harness_file_control.rs
tests/harness_edit_protocol.rs
tests/harness_patch_applier.rs

tests/harness_execution_loop.rs
tests/harness_validation.rs
tests/harness_failure.rs
tests/harness_reproduction.rs
tests/harness_repair_loop.rs
tests/harness_acceptance.rs
tests/harness_task_cache.rs

tests/harness_review.rs
tests/harness_semantic_diff.rs
tests/harness_minimality.rs
tests/harness_risk.rs
tests/harness_verification.rs
tests/harness_adversarial_validation.rs
tests/harness_confidence.rs
tests/harness_selection.rs
tests/harness_scaling.rs

tests/harness_sandbox.rs
tests/harness_permissions.rs
tests/harness_git_checkpoint.rs
tests/harness_trajectory.rs
tests/harness_observability.rs
tests/harness_runtime_tools.rs
tests/harness_time_travel.rs

tests/harness_work_context_integration.rs
tests/harness_model_strategy.rs
tests/harness_golden_paths.rs
tests/harness_regression_memory.rs
tests/harness_benchmark.rs
tests/harness_artifacts.rs
tests/harness_completion.rs
```

The previous plan required 15 minimum suites; this expands coverage to match all 35 issues instead of pretending eight tests can supervise an entire execution engine. 

---

# Documentation Requirements

Add or update:

```text
docs/v1.6-harness-engine.md
docs/harness-execution-flow.md
docs/harness-benchmarking.md
docs/harness-patch-protocol.md
docs/harness-sandboxing.md
docs/harness-evidence-completion.md
docs/harness-api-cli.md
```

Docs must include:

```text
Architecture overview
Module map
Execution flow
Patch protocol format
Risk/approval rules
Validation strategy
Sandbox strategy
Trajectory/replay behavior
API examples
CLI examples
Definition of done
Known limitations
```

---

# Dependencies

Verify/add:

```toml
git2 = "0.20"
diffy = "0.4"
sha2 = "0.10"
walkdir = "2.5"
ignore = "0.4"
tiktoken-rs = "0.5"
async-trait = "0.1"
opentelemetry = "0.27"
opentelemetry_sdk = "0.27"
tracing-opentelemetry = "0.28"
```

Optional / feature-gated:

```toml
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.23"
bollard = "0.18" # Docker runtime, optional
```

The original implementation plan already proposed `git2`, `diffy`, `tiktoken-rs`, `sha2`, `walkdir`, and `ignore`. 

---

# Execution Order

Since you want everything in V1.6, the sequencing becomes dependency-driven inside one release.

```text
Wave 1 — Foundation
Issues 1, 2
Issues 3, 4, 5

Wave 2 — Execution Core
Issues 6, 7, 8, 12
Issues 9, 10, 11

Wave 3 — Quality Gates
Issues 13, 14, 15, 17
Issues 16, 18, 19, 20, 21

Wave 4 — Infrastructure
Issues 22, 23, 24, 25
Issues 26, 27, 28

Wave 5 — Intelligence + Integration
Issues 29, 30, 31, 32, 33, 34, 35

Wave 6 — E2E Hardening
API/CLI integration
Docs
No-stub audit
Clean clone test
Full lifecycle E2E
```

The previous plan estimated 11 weeks for all 35 issues. That is more realistic than a heroic 4-week sprint unless you have a suspiciously obedient army of Rust goblins. 

---

# Full Definition of Done for V1.6

V1.6 is complete only when:

```text
A coding WorkContext can run through the Harness Engine
Repo intelligence identifies relevant files and symbols
Environment fingerprinting detects test/build commands
File control blocks unsafe edits
Structured edits are parsed and validated
A real patch is produced
Patch dry-run works
Git checkpoint is created before patch application
Patch is applied safely
Validation runs format/lint/test/repro commands where available
Repair loop handles failed patch/validation
Review layer produces structured issues
Semantic diff and risk gates run
Patch minimality is scored
Attempt pool can run multiple isolated attempts
Selection engine chooses best candidate
Sandbox runtime enforces timeouts and permissions
Tool permission ledger records actions
Trajectory records every major step
OpenTelemetry/local observability emits traces/metrics
Runtime task-local tools are scoped and auditable
Time-travel replay can inspect decisions
WorkContext stores all harness artifacts
Regression memory records relevant failures
Golden path templates guide common task types
Benchmark mode supports private/synthetic/adversarial tasks
Completion artifact is generated
Evidence-based completion policy gates final status
API and CLI both execute the same code path
All 35 issues have tests
No stubs/placeholders/TODOs in production paths
Docs are complete
Changelog is updated
Clean clone passes cargo check/test
```

---

# Success Metrics

```text
Patch success rate: >90% on well-defined small tasks
Validation pass rate: >85% when existing test suite exists
Critical risk detection: >95% for obvious auth/db/secrets/config changes
Review false-positive rate: <10% for common safe diffs
Single-attempt execution time: <5 minutes for typical repo task
Rollback success rate: 100% in tested git repos
Harness module test coverage: >80%
No silent persistence failures in harness path
```

---

# Strategic Notes

The consensus direction is clear:

```text
Keep WorkContext as the moat.
Keep Flow as the substrate.
Make Harness the only path that can touch code.
Make Evidence the only path to completion.
Make Git rollback non-optional.
Make validation mandatory.
Make risk visible.
Make user trust the product because it shows proof.
```

This is the version where PrometheOS Lite stops being “a very sophisticated orchestrator” and becomes a real autonomous coding execution system.

The brutal truth: V1.6 is not about adding more intelligence. It is about adding **consequences, proof, and brakes**. Which, irritatingly enough, is how useful software gets born.
