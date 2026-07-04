So now you want to give your system hands. Brave. Let’s make sure it doesn’t start slapping itself in production.

Here’s a **real V1.4 PRD (Hands / Coding Harness)** aligned with your current repo (which finally stopped pretending in V1.2–V1.3 and actually works).

---

# ## V1.4 PRD — “Hands” (Coding Harness & Repo Execution)

## Executive Context

Current system (after V1.3):

```txt
✔ WorkContext lifecycle (real)
✔ Flow execution engine (real)
✔ Orchestrator (real)
✔ Playbooks + Evolution (functional, not magical)
✔ Metadata propagation (real)
✔ No fake nodes (finally)
```

What is missing:

```txt
✘ No real interaction with codebases
✘ No repo awareness
✘ No patching / editing loop
✘ No verification loop
✘ No safe execution boundary
```

Right now:

```txt
PrometheOS = thinks
```

V1.4:

```txt
PrometheOS = works
```

---

# ## Core Design Principle

```txt
Flows orchestrate
Tools act
Harness verifies
```

---

# ## Architecture (aligned with your repo)

You already have:

```txt
- ToolRuntime
- Node system
- WorkExecutionService
- Orchestrator
- Strict execution behavior
```

So we DO NOT invent a new system.

We extend:

```txt
ToolRuntime → becomes real tool execution layer
Flow Nodes → call tools
WorkContext → stores artifacts/results
```

---

# ## EPIC 1 — Repo Tooling Layer

## Goal

Give the system safe, deterministic access to the filesystem + repo.

---

## Issue: RepoToolRegistry

**File:** `src/tools/repo.rs`

```rust
pub trait RepoTool {
    fn name(&self) -> &'static str;
    fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value>;
}
```

---

## Required Tools

### 1. list_tree

```rust
list_tree(root: String, depth: Option<u32>)
```

Output:

```json
{
  "files": ["src/main.rs", "src/lib.rs"],
  "dirs": ["src/", "tests/"]
}
```

---

### 2. read_file

```rust
read_file(path: String)
```

---

### 3. search_files

```rust
search_files(query: String, glob: Option<String>)
```

---

### 4. write_file

```rust
write_file(path: String, content: String)
```

---

### 5. patch_file (CRITICAL)

```rust
patch_file(path: String, diff: String)
```

Rules:

```txt
- Must validate diff applies cleanly
- Must reject partial/invalid patches
- Must produce artifact: diff + result
```

---

### 6. git_diff

```rust
git_diff()
```

---

# ## EPIC 2 — Command Harness

## Issue: CommandTool

**File:** `src/tools/command.rs`

```rust
run_command(command: String, args: Vec<String>, cwd: String)
```

Must return:

```json
{
  "stdout": "...",
  "stderr": "...",
  "exit_code": 0,
  "duration_ms": 1200
}
```

---

## Issue: run_tests

Wrapper around `run_command`.

---

## Requirements

```txt
✔ timeout enforced
✔ max output size
✔ no interactive prompts
✔ deterministic execution
```

---

# ## EPIC 3 — ToolRuntime Upgrade

## Modify: `src/flow/tool_runtime.rs`

Add:

```txt
- tool registry injection
- tool whitelist per WorkContext
- strict mode enforcement
```

---

## Add ToolPolicy

```rust
pub struct ToolPolicy {
    pub allowed_paths: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allow_commands: Vec<String>,
}
```

---

# ## EPIC 4 — Coding Flow Template

## File: `flows/software_dev.yaml`

```yaml
nodes:
  - id: inspect_repo
    type: tool
    tool: list_tree

  - id: read_context
    type: tool
    tool: read_file

  - id: plan
    type: planner

  - id: implement
    type: coder

  - id: apply_patch
    type: tool
    tool: patch_file

  - id: run_tests
    type: tool
    tool: run_tests

  - id: review
    type: reviewer

transitions:
  - from: plan
    to: implement

  - from: implement
    to: apply_patch

  - from: apply_patch
    to: run_tests

  - from: run_tests
    to: review
```

---

# ## EPIC 5 — Verification Loop

## Modify: Orchestrator Loop

Add retry loop:

```txt
plan → patch → test → failure → re-plan → patch → test
```

Bounded by:

```txt
max_iterations = 5
max_failures = 3
```

---

# ## EPIC 6 — Artifact System Extension

Add artifact types:

```txt
- code_patch
- test_result
- diff
- command_output
- repo_snapshot
```

---

# ## EPIC 7 — Safety Layer

## Path Guard

```txt
- no writes outside workspace root
- no system paths (/etc, /usr, etc.)
```

## Command Guard

```txt
- block dangerous commands
- whitelist only safe binaries
```

---

# ## EPIC 8 — Strict Mode Enforcement

Extend your existing strict behavior:

```txt
✔ tool failure = stop
✔ invalid patch = stop
✔ test failure = retry loop
✔ missing output = error
```

---

# ## EPIC 9 — Testing (non-negotiable)

## Create fixture repo

```txt
tests/fixtures/sample_repo/
```

---

## Required tests

```txt
✔ read_file works
✔ patch_file applies valid diff
✔ patch_file rejects invalid diff
✔ run_tests returns failure correctly
✔ full loop: failing test → fix → pass
✔ forbidden path rejection
```

---

# ## EPIC 10 — Observability (minimal)

Add:

```txt
- tool_call logs
- command logs
- execution trace per WorkContext
```

---

# ## Definition of Done

```txt
✔ System can read repo
✔ System can modify files safely
✔ System can run tests
✔ System can retry on failure
✔ System produces real diffs
✔ System never silently succeeds with fake outputs
```

---

# ## What NOT to do

```txt
✘ Do NOT allow unrestricted shell
✘ Do NOT skip patch validation
✘ Do NOT auto-commit changes silently
✘ Do NOT allow tools without policy guard
```

---

# ## V1.5 — Stabilization & Context Control

After V1.4:

```txt
- ContextBudgeter (token control)
- Memory pruning / summarization
- Better evaluation signals
- Observability (real tracing)
```

---

# ## V2 — Multi-Agent System

```txt
Planner Agent
Coder Agent
Reviewer Agent
Research Agent
```

Now meaningful because:

```txt
they operate on real code, not text blobs
```

---

# ## V3 — Advanced Coding System

```txt
- semantic code search
- AST-aware patching
- dependency graph understanding
- test generation
```

---

# ## V4 — Swarm Execution

```txt
- parallel WorkContexts
- agent coordination
- shared memory graph
```

---

# ## V5 — Self-Improving System

```txt
- automatic skill extraction
- flow synthesis
- meta-learning
```

---

# Final reality check

Right now:

```txt
You built a brain
```

V1.4:

```txt
You give it hands
```

After that:

```txt
You find out if it actually knows what it’s doing
```

And that’s where most systems collapse.
