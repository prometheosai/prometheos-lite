# PrometheOS Lite MVP PRD

## Product Name

PrometheOS Lite: Repo Workbench

## Product Summary

PrometheOS Lite is a local-first AI workbench for software developers. It helps a developer understand a local codebase, create a WorkContext, generate a safe plan, analyze files, propose changes, require approval before writes, save memory, and continue work later.

## Core Promise

A developer can point PrometheOS Lite at a repo and get useful, safe, resumable AI-assisted software work in under 10 minutes.

## Primary User

Individual developer or technical founder working inside a local codebase.

## Initial Use Case

A developer wants to identify risky code and receive safe improvement suggestions without giving an autonomous agent unrestricted write access.

## MVP Goal

Deliver one reliable local workflow:

```text
Local repository → WorkContext → read-only analysis → plan → staged artifact → approval gate → memory write → continuation.
```

## Non-Goals

The MVP will not include:

- Cloud sync
- Team workspaces
- Billing
- Voice interface
- Text-to-speech
- Trading workflows
- Self-improving model training
- Full Mnemosyne integration
- Full PrometheOS service deployment
- Autonomous unrestricted writes
- Multi-agent marketplace
- Browser IDE

## Core Features

### 1. Local Repo Selection

The user can run PrometheOS Lite inside a local repository or pass a repo path.

Target command:

```bash
prometheos work create --repo . --goal "Find risky code and suggest safe improvements"
```

### 2. WorkContext Creation

The system creates a persistent WorkContext with:

- `work_id`
- repo path
- goal
- mode
- created_at
- status
- current phase
- memory references
- artifacts

### 3. Repo Scan

The system scans the repo and identifies:

- project type
- important files
- source files
- config files
- tests
- likely entrypoints

### 4. Planning

The system generates a short plan:

- objective
- files to inspect
- risks
- expected artifacts
- next action

### 5. Read-Only Analysis

The system performs read-only file analysis. It must not write to disk during the first analysis phase.

### 6. Artifact Generation

The system generates at least one artifact:

- risk report
- suggested patch
- follow-up task list
- implementation plan

### 7. Approval Gate

The system must require approval before modifying files.

Target command:

```bash
prometheos work approve <artifact_id>
```

### 8. Memory Write

The system stores:

- goal
- repo summary
- files inspected
- plan
- findings
- artifacts
- user approvals or rejections
- next recommended action

### 9. Continue Work

The user can resume a previous WorkContext.

Target command:

```bash
prometheos work continue <work_id>
```

The system restores context and suggests the next action.

## Target CLI

```bash
prometheos work create --repo . --goal "<goal>" --mode review
prometheos work run <work_id>
prometheos work status <work_id>
prometheos work artifacts <work_id>
prometheos work approve <artifact_id>
prometheos work continue <work_id>
prometheos memory show <work_id>
```

## MVP Workflow

1. User creates WorkContext.
2. PrometheOS Lite scans the repo.
3. System generates a plan.
4. System performs read-only analysis.
5. System creates a risk report and suggested patch artifact.
6. User reviews artifact.
7. User approves or rejects.
8. System records the decision.
9. System saves memory.
10. User can continue later.

## Acceptance Criteria

The MVP is complete when:

- A fresh user can clone, build, and run PrometheOS Lite.
- The user can create a WorkContext for a local repo.
- The system can scan and summarize the repo.
- The system can generate a useful plan.
- The system can inspect files without writing.
- The system can produce a readable risk report.
- The system can produce a staged suggested patch artifact.
- The system does not write changes without approval.
- The system saves memory for the run.
- The user can continue the WorkContext later.
- The full demo works in under 10 minutes.

## Quality Bar

The MVP should feel boringly reliable.

Prefer:

- clear logs
- predictable output
- safe defaults
- simple CLI
- readable artifacts
- explicit approval gates

Avoid:

- vague agent claims
- hidden autonomy
- huge architecture changes
- new model training
- experimental voice features
- unnecessary abstractions

## Success Metric

Primary metric:

A developer can get one useful repo analysis result within 10 minutes of installation.

Secondary metrics:

- WorkContext resumes correctly.
- No file write occurs without approval.
- Memory contains useful summary.
- User understands what happened.
- User knows the next action.

## First Demo Script

```bash
git clone <sample_repo>
cd <sample_repo>

prometheos work create \
  --repo . \
  --goal "Find risky code and suggest safe improvements" \
  --mode review

prometheos work run <work_id>

prometheos work artifacts <work_id>

prometheos work approve <artifact_id>

prometheos memory show <work_id>

prometheos work continue <work_id>
```

## Product Positioning

PrometheOS Lite is a local AI workbench for safe autonomous software workflows. It helps developers understand codebases, plan work, inspect files, propose changes, require approvals, remember decisions, and continue later.

## Implementation Notes

### Suggested Golden Path

```text
CLI → WorkContext Service → Repo Scanner → Planner → Read-only Tool Runtime → Artifact Store → Approval Gate → Memory Store → Continuation
```

### Suggested Data Model

```rust
struct WorkContext {
    id: String,
    repo_path: String,
    goal: String,
    mode: WorkMode,
    phase: WorkPhase,
    status: WorkStatus,
    created_at: DateTime,
    updated_at: DateTime,
    artifacts: Vec<ArtifactRef>,
    memory_refs: Vec<String>,
    next_action: Option<String>,
}
```

```rust
struct Artifact {
    id: String,
    work_id: String,
    kind: ArtifactKind,
    title: String,
    content: String,
    status: ArtifactStatus,
    requires_approval: bool,
}
```

```rust
struct MemoryRecord {
    id: String,
    work_id: String,
    kind: MemoryKind,
    summary: String,
    content: String,
    created_at: DateTime,
}
```

## First Workflow: Risky Code Review

Inputs:

- repo path
- goal
- optional file filters

Steps:

1. Detect project type.
2. Find important files.
3. Inspect source files.
4. Detect risky patterns.
5. Generate risk report.
6. Suggest safe patch.
7. Require approval.
8. Save memory.

Example output:

```markdown
# Risk Review

## Summary

PrometheOS Lite reviewed 12 files and found 3 medium-risk issues.

## Findings

### 1. Unsafe unwrap in config parser

File: `src/config.rs`  
Risk: Medium  
Why it matters: Can panic at runtime if config is malformed.  
Suggested fix: Replace unwrap with structured error handling.

### 2. Missing test for API error path

File: `src/api.rs`  
Risk: Medium  
Why it matters: Error behavior can regress silently.  
Suggested fix: Add test for invalid request body.

### 3. Hardcoded timeout

File: `src/client.rs`  
Risk: Low  
Why it matters: Timeout should be configurable.  
Suggested fix: Move timeout into config.
```

## Follow-Up Milestone

After the MVP works locally, PrometheOS Lite may export completed WorkContexts into Mnemosyne and later provide traces to Brain for learning. These are not MVP requirements.
