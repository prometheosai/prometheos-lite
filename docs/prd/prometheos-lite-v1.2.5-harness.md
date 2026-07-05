## Executive verdict

PrometheOS Lite is going in the **right strategic direction**, but right now it is still more **execution framework** than **personal AI operating system**.

The good news: the foundations are not stupid. Shocking, really. You already have Rust, flow execution, WorkContext, SQLite, CLI/API structure, model routing, memory primitives, budgets, tracing, and guardrails. The README explicitly positions v1.2 as flow-centric, local-first, with WorkContext lifecycle management, domain profiles, artifacts, autonomy levels, policy enforcement, memory, and model routing. 

The bad news: the current harness is still too **static**, too **flow-file driven**, and not yet enough like:

```text
goal → persistent work context → autonomous plan → tool execution → verification → continuation → learning
```

Right now it is closer to:

```text
intent → select flow → run nodes → produce output
```

That is useful. It is not Jarvis yet. It is Jarvis’s clipboard.

---

## 1. Current architecture state

The repo is a Rust 2024 project using `tokio`, `axum`, `rusqlite`, `reqwest`, `clap`, `serde`, `uuid`, and local OpenAI-compatible LLM APIs. That’s a sane local-first stack. No bloated circus tent, thank heavens. 

The public module surface is already clean-ish:

```rust
pub mod api;
pub mod config;
pub mod control;
pub mod db;
pub mod flow;
pub mod fs;
pub mod intent;
pub mod llm;
pub mod logger;
pub mod personality;
pub mod tools;
pub mod utils;
pub mod work;
```

Legacy agent/core modules are hidden behind a `legacy` feature flag, which is exactly the kind of adult decision software occasionally makes when unsupervised. 

CLI entry is simple:

```text
Run
Flow
Serve
Bench
Work
Templates
```

So the product already has the bones of a local agent runtime plus WorkContext management. 

---

## 2. The best thing in the repo: WorkContext is correctly shaped

This is the strongest part.

Your `WorkContext` already includes:

```text
id
user_id
title
domain
domain_profile_id
context_type
project_id
conversation_id
parent_context_id
priority
due_at
goal
requirements
constraints
status
phase
plan
approved_plan
artifacts
memory_refs
decisions
flow_runs
tool_trace
open_questions
autonomy_level
approval_policy
completion_criteria
metadata
timestamps
```

That is not just a task. That is the beginning of a persistent operational object. 

Even better: `WorkDomain` already supports:

```rust
Custom(String)
```

So you are not trapped in the “Software, Business, Marketing, Personal” kindergarten enum forever. Good. The machine has not yet swallowed its own legs. 

This aligns with your uploaded V1.2 PRD: “Flow = how work is executed; WorkContext = what work is being done over time.” 

That concept is correct.

---

## 3. The WorkContext service is useful but still primitive

`WorkContextService` does CRUD, status updates, phase updates, artifacts, decisions, completion criteria, blocking, list, active context lookup, and routing by explicit/context conversation ID. 

That is good plumbing.

But it is not yet an intelligence layer.

What is missing:

```text
- no WorkContext scoring
- no automatic next-action selection
- no playbook evolution
- no context merge/split logic
- no dependency graph between contexts
- no stale-context detection
- no "resume until completion" execution loop
- no domain-specific evaluator
```

Right now `route_context()` basically says:

```text
explicit context > active conversation context > none
```

That is not enough for Jarvis. Jarvis should infer:

```text
“This message belongs to the book project.”
“This is a new legal-document context.”
“This coding task is a child context of the SaaS project.”
“This context is blocked until approval.”
```

Current routing is a doorman. You need an air-traffic controller.

---

## 4. Flow engine: solid primitive, not yet a coding harness

The flow module is strong as a generic execution engine. It has nodes, transitions, state, budget guards, tracing, loop detection, nested flows, and builder DSL. 

`SharedState` has explicit buckets:

```text
input
working
output
context
meta
```

This is a good pattern. It prevents the usual agent-framework crime of stuffing everything into one JSON blob and calling it “memory,” as if naming garbage makes it architecture. 

`Flow::run()` handles:

```text
start node
prep
exec
post
transition
retry
budget check
tracing
loop detection
completion
```

That is the right skeleton. 

But for coding tasks, this is not enough.

Claude Code’s strength is not “it has nodes.” It is:

```text
inspect repo
search files
read relevant files
edit exact files
run tests
parse failures
patch again
verify
summarize diff
```

PrometheOS Lite currently has flow execution. It does **not yet have a ruthless coding loop**.

That is the big gap.

---

## 5. Built-in nodes are too naive right now

Your built-in `PlannerNode`, `CoderNode`, and `ReviewerNode` are placeholder-style prompt wrappers:

```text
Planner: “Create a structured plan”
Coder: “Generate code”
Reviewer: “Review generated code”
```

They call `ModelRouter.generate()` when available. 

This is fine for v1. It is not enough for SOTA coding.

The `CoderNode` currently asks the model to “provide generated code only.” That is the exact kind of prompt that creates useless floating code not grounded in the repo. Coding agents should not “generate code.” They should:

```text
read repo → identify files → patch files → run tests → verify
```

So the node taxonomy needs to evolve from:

```text
planner / coder / reviewer
```

to:

```text
RepoInspector
ContextRetriever
PatchPlanner
FileEditor
TestRunner
FailureInterpreter
Verifier
DiffSummarizer
CompletionEvaluator
```

Otherwise you will build a well-typed hallucination machine. Rust will make it safe. It will still be wrong.

---

## 6. There is a bug-shaped smell in `ContextLoaderNode`

In `ContextLoaderNode::prep()`, it builds:

```json
{ "task": task }
```

But in `exec()`, it reads:

```rust
let query = input["query"].as_str().context("Missing query...")
```

That means the node prepares `task` but executes expecting `query`. Unless another wrapper mutates it, this node will fail when actually used. 

That is not a philosophical issue. That is a brick on the floor in the hallway.

Fix:

```rust
Ok(serde_json::json!({ "query": task }))
```

or make `exec()` accept both.

---

## 7. Model router exists, but it is not a real router yet

`ModelRouter` has providers, fallback chain, current provider, `generate`, and `generate_stream`. 

Good start.

But it does not yet route by:

```text
task type
cost
latency
local vs cloud
coding vs summarization
context size
failure history
confidence
available hardware
```

It is currently a fallback wrapper, not an intelligent router.

Also, the OpenAI-compatible provider wraps `LlmClient`, but `model()` returns `"unknown"` because `LlmClient` doesn’t expose the model. 

That is a small detail, but details like this become observability rot. If you cannot log which model produced an artifact, you cannot evaluate the system. If you cannot evaluate the system, you are just roleplaying progress with extra JSON.

---

## 8. LLM client is simple and local-first, but too thin

`LlmClient` posts to:

```text
/v1/chat/completions
```

with a single user message and supports retries plus streaming parsing. 

This is good for LM Studio compatibility.

But for your target, it lacks:

```text
system/developer prompt layers
tool call support
structured output enforcement
response format / schema mode
usage extraction
token accounting from provider responses
conversation message history
model metadata
timeout profiles per provider
backoff classification by error type
```

This is the difference between:

```text
LLM client
```

and:

```text
agent model runtime
```

You need the second.

---

## 9. Memory layer is promising, but not yet Hermes-grade

The memory module has:

```text
MemoryDb
EmbeddingProvider
FallbackEmbeddingProvider
LocalEmbeddingProvider
MemoryService
MemoryKind
MemoryRelationship
VectorSearchBackend
BruteForceBackend
ContextLoaderNode
MemoryExtractorNode
MemoryWriteNode
```

This is directionally strong. 

`MemoryService` supports episodic/semantic memory, background write queue, deduplication, importance/confidence scores, vector search, relationships, delete, rebuild index. 

But there are serious gaps:

```text
- vector index is in-memory brute force
- semantic memories written in background often have embedding = None
- no durable vector index reload unless explicitly rebuilt
- no memory provider abstraction like Hermes
- no skill/procedural memory lifecycle
- no memory poisoning scanner
- no user/project/work-context retrieval policy
- no fenced recalled-memory context
```

Hermes has a cleaner distinction:

```text
memory = durable facts
session_search = episodic recall
skills = reusable procedures
external provider = optional semantic/dialectic memory
```

PrometheOS currently has “memory” as one system with types, but the behavioral separation is not strong enough yet.

You need:

```text
MemoryKernel
SkillKernel
SessionSearch
WorkContextPlaybook
EvolutionEngine
```

Not one memory bucket wearing five hats.

---

## 10. WorkContext docs already mention playbooks, but code seems behind

The docs include a `work_context_playbooks` table with preferred flows, approval defaults, research depth, creativity level, evaluation rules, confidence, and usage count. 

That is exactly the right direction.

But from the code inspected, WorkContext service does not yet appear to actually run:

```text
playbook retrieval
playbook scoring
playbook update
flow preference learning
skill evolution
domain-specific evaluation
```

So the docs are ahead of the engine.

Not fatal. But dangerous if you start believing the docs. Documentation is where future code goes to cosplay as present code.

---

## 11. The real architectural gap: no always-on operating layer yet

Your stated vision is:

```text
Jarvis/Samantha-like personal AI OS
always on
multi-channel
local-first
continuation semantics
coding/apps/books/design/business/personal tasks
YOLO mode with guardrails
```

Current repo has:

```text
CLI
Serve command
FlowExecutionService
WorkContext
templates
memory
basic model routing
```

It does not yet have the true “OS layer”:

```text
Gateway daemon
Channel adapters
Event bus
Job queue
Scheduler
Long-running task runner
Task cancellation
User notification system
Approval inbox
Workspace isolation
Tool permission ledger
Agent activity stream
Background continuation
```

OpenClaw’s best idea is the always-on gateway. Hermes’ best idea is memory/skills/evolution. Claude Code’s best idea is brutal execution discipline.

PrometheOS Lite currently has the beginning of Claude-ish flow discipline and the beginning of Hermes-ish memory. It does not yet have OpenClaw’s always-on body.

That needs to become a first-class layer, not an afterthought.

---

## 12. What you are doing right

### A. Rust is the correct choice

For local-first, long-running, safe execution, Rust is a strong choice. Not because Rust is magic, but because it makes sloppy state mutation more annoying. Annoying is good. Software needs more friction before stupidity.

### B. WorkContext is the right centerpiece

This is the biggest strategic win. WorkContext is the thing that can make PrometheOS feel like a system instead of a chat window. 

### C. Flow-centric execution is a good primitive

Flows are composable, traceable, testable, and reusable. Good. 

### D. Legacy modules are isolated

Keeping deprecated agents/core behind a feature flag prevents architecture rot from spreading. 

### E. Local-first stack is lean

`rusqlite`, `axum`, `tokio`, OpenAI-compatible API, no giant distributed nonsense yet. Good. 

---

## 13. What is weak / worrying

### 1. Too much “flow runner,” not enough “agent harness”

A flow runner executes known graphs.

An agent harness decides what graph/tools/actions are needed when the world is messy.

You need a layer above FlowExecutionService:

```text
WorkOrchestrator
```

It should own:

```text
intent → context routing → playbook selection → plan → execution loop → verification → memory/skill update
```

### 2. Built-in nodes are too generic

The node library needs domain-operational nodes, not just LLM wrappers.

### 3. Tool runtime needs to become a real registry

Hermes and Claude Code both win because tools are precise, schema-bound, filtered, permissioned, and observable.

Your current direction has `ToolRuntime`, `ToolPolicy`, and `PathGuard`, which is good, but the coding harness needs tools like:

```text
read_file
write_file
patch_file
search_files
run_command
run_tests
git_diff
git_status
list_tree
inspect_package
parse_errors
```

And they need structured outputs.

### 4. WorkContext is not yet driving execution deeply enough

The README says Flow → WorkContext integration exists, docs describe `WorkExecutionService`, and the PRD expects continuation. But the core inspected path still centers on `execute_message()` and flow selection by `Intent`. 

WorkContext should become the default execution substrate.

### 5. Memory is not yet skill evolution

You need the Hermes pattern:

```text
after complex task → extract reusable procedure → create skill
after failed/revised skill → patch skill
after repeated domain work → update playbook
```

Right now memory exists. Evolution does not.

### 6. Observability is partial

Tracing exists, but you need persistent, queryable, UI-visible execution traces:

```text
what did it do?
why?
which memory was used?
which tools ran?
what changed?
what failed?
what model made the decision?
what did it cost?
```

This is essential for trust.

---

## 14. Priority refactor: build the missing “harness spine”

Your next architecture should look like this:

```text
Channel Input
  ↓
Gateway / API / CLI
  ↓
Intent + Context Router
  ↓
WorkContextService
  ↓
PlaybookResolver
  ↓
WorkOrchestrator
  ↓
ExecutionHarness
  ↓
FlowEngine + Tools + Models
  ↓
Verifier
  ↓
ArtifactStore
  ↓
Memory/Skill Evolution
  ↓
Continuation Decision
```

Right now you have pieces of the lower half. You need the upper spine.

---

## 15. The next 10 issues I would create

### Issue 1 — Fix ContextLoaderNode input mismatch

`prep()` emits `task`, `exec()` expects `query`. Fix immediately. Tiny bug, big embarrassment potential. 

### Issue 2 — Add WorkOrchestrator

Central service:

```rust
submit_user_intent(...)
continue_context(...)
run_until_blocked_or_complete(...)
```

It should own the high-level loop.

### Issue 3 — Make WorkContext the default execution path

Stop treating WorkContext as optional. Every non-trivial task should become or attach to one.

### Issue 4 — Add WorkContextPlaybook service

Use the table already documented:

```text
preferred flows
research depth
creativity level
approval policy
evaluation rules
confidence
usage count
```

Docs already know this exists. Code needs to catch up. 

### Issue 5 — Add SkillKernel

Skills should be executable procedural memories:

```text
skill = metadata + trigger conditions + flow graph + examples + success/failure history
```

### Issue 6 — Add EvolutionEngine

After each completed context:

```text
evaluate → update playbook → propose skill create/patch → store learning event
```

### Issue 7 — Build coding harness nodes

Add repo-aware nodes:

```text
RepoInspectNode
FileSearchNode
ReadFileNode
PatchNode
CommandNode
TestNode
FailureReviewNode
DiffSummaryNode
VerificationNode
```

### Issue 8 — Add provider/runtime metadata

Every model output should know:

```text
provider
model
latency
tokens
cost estimate
fallback used
```

Right now `OpenAiProvider.model()` returning `"unknown"` is not acceptable long-term. 

### Issue 9 — Persistent event/job system

For always-on mode, you need:

```text
jobs
events
outbox
inbox
approvals
notifications
scheduled continuations
```

### Issue 10 — Build the Control Panel around trace + WorkContext

Not just pretty chat. Show:

```text
active contexts
phase
current plan
artifacts
tool trace
memory used
pending approvals
budget
next action
```

That is Mission Control.

---

## 16. Strategic direction: what PrometheOS Lite should become

The current repo says:

```text
Flow-centric AI agent orchestration system
```

That is too small.

The product should become:

```text
Local-first personal AI operating layer for persistent, autonomous work.
```

The architecture should optimize for:

```text
continuation
verification
local execution
memory safety
skill evolution
budget control
multi-channel interaction
user-specific playbooks
```

Not “agent chat.”

Not “flow runner.”

Not “cool Rust CLI.”

Those are ingredients. The product is persistent work.

---

## 17. My bad-mood final diagnosis

You are building the right thing, but you are still too close to the engine room.

You have:

```text
Flow engine: decent
WorkContext: promising
Memory: early but meaningful
CLI/API: basic
Model routing: thin
Tools: not yet brutal enough
Always-on OS layer: mostly missing
Skill evolution: missing
Coding harness: insufficient
```

The project is past toy stage, but not yet dangerous.

And you want dangerous.

Not unsafe-dangerous. Useful-dangerous. The kind where it can actually finish a book, build an app, continue tomorrow, remember the constraints, run tests, patch itself, and tell you exactly what it did.

Right now PrometheOS Lite can become that.

But only if the next phase stops adding “features” and starts building the **spine**:

```text
WorkContext → Playbook → Skill → Flow → Tool → Verification → Evolution
```

That is the system.

Everything else is furniture.
