# PrometheOS Lite Issue Tracker

This document tracks all issues from the PRDs organized by milestone, with implementation status.

**Legend:**
- [ ] = Open (not started)
- [~] = In Progress
- [x] = Done (implemented)

---

## v1.0 PRD - Multi-Agent CLI (Complete)

### Phase 0 - Foundation
- [x] Issue #1: Initialize Rust workspace & CLI entrypoint
- [x] Issue #2: Project structure scaffolding

### Phase 1 - LLM Integration
- [x] LLM client with reqwest
- [x] Config loader with JSON support

### Phase 2 - Agent System
- [x] Agent trait
- [x] Planner agent
- [x] Coder agent
- [x] Reviewer agent

### Phase 3 - Orchestration
- [x] Sequential orchestrator

### Phase 4 - Real-Time Experience
- [x] Issue #11: Structured agent logger
- [x] Issue #12: Streaming output renderer
- [x] Issue #13: Execution timeline events

### Phase 5 - File System
- [x] Issue #14: File parser for generated output
- [x] Issue #15: Safe file writer

### Phase 6 - CLI Experience
- [x] Issue #16: CLI UX output improvements
- [x] Issue #17: Robust error handling and retries

### Phase 7 - Release Readiness
- [x] Issue #18: Optimize default prompts for demo quality
- [x] Issue #19: Finalize documentation and examples
- [x] Issue #20: Plugin interface (superseded by v1.1)
- [x] Issue #21: Web log viewer (superseded by v1.1)

**Status:** v1.0 production release complete (v0.5.0)

---

## v1.1 PRD - Flow-Centric Architecture

### Phase 1 - Flow Core (Foundation)
- [x] 1.1 Core Types (SharedState, NodeId, Action, Input, Output)
- [x] 1.2 Node Trait (prep, exec, post, config)
- [x] 1.3 Flow Engine (execution loop, transitions, validation)
- [x] 1.4 Retry System (exponential backoff, failure propagation)
- [~] 1.5 Flow as Node (FlowNode wrapper for nested flows)
- [ ] 1.6 Migration Adapter (AgentNode wrapper for existing agents)
- [ ] Parity tests (Flow vs SequentialOrchestrator)

### Phase 2 - Orchestration
- [ ] 2.1 Maestro (flow scheduling, multi-flow orchestration)
- [ ] 2.2 Continuation Engine (checkpointing, resume, replay)
- [ ] 2.3 Flow Types (branching, looping, batch flows)

### Phase 3 - Intelligence
- [ ] 3.1 Model Router (provider abstraction, fallback chains)
- [ ] 3.2 Tool Runtime (Tool trait, sandbox profiles, process isolation)
- [ ] 3.3 LLM Utilities (call_llm, streaming, token limiter)

### Phase 4 - Memory
- [ ] 4.1 SQLite Schema (memories, embeddings, relationships)
- [ ] 4.2 Embedding Provider (local HTTP, external API fallback)
- [ ] 4.3 Memory Service (CRUD, semantic retrieval, episodic logging)
- [ ] 4.4 Flow Integration (ContextLoaderNode, MemoryWriteNode)

### Phase 5 - Advanced Execution
- [ ] 5.1 Parallel Flows (async parallel execution, concurrency limits)
- [ ] 5.2 Self-Reflection Loops (looping nodes, reflection nodes)
- [ ] 5.3 Batch Processing (iterable inputs, progress tracking)

### Phase 6 - Developer Experience
- [ ] 6.1 CLI Runner (flow execution, flow file loading)
- [ ] 6.2 Flow Builder DSL (simplified API, builder pattern)
- [ ] 6.3 Debug Mode (step-by-step, state snapshots, breakpoints)
- [ ] 6.4 Logging & Tracing (structured logs, event timeline)

### Phase 7 - Safety & Control
- [ ] 7.1 Policy Hooks (pre/post validation, constitution-policy)
- [ ] 7.2 Tool Sandbox Enforcement (permission model, capability checking)
- [ ] 7.3 Rate Limiting (token budgeting, execution guardrails)

### Migration & Cleanup
- [ ] 8.1 Remove Legacy Code (SequentialOrchestrator, old Agent trait)
- [ ] 8.2 Documentation (README update, examples, migration guide)
- [ ] 8.3 Final Testing (full test suite, performance benchmark, E2E tests)

**Status:** Phase 1 in progress (~60% complete)
**See:** `docs/prd/prometheos-lite-prd-v1.1.md` for full specification