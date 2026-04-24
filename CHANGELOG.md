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
- [x] 1.5 Flow as Node (FlowNode wrapper for nested flows)
- [x] 1.6 Migration Adapter (AgentNode wrapper for existing agents)
- [x] Parity tests (Flow vs SequentialOrchestrator)

### Phase 2 - Orchestration
- [x] 2.1 Maestro (flow scheduling, multi-flow orchestration)
- [x] 2.2 Continuation Engine (checkpointing, resume, replay)
- [x] 2.3 Flow Types (branching, looping, batch flows)

### Phase 3 - Intelligence
- [x] 3.1 Model Router (provider abstraction, fallback chains)
- [x] 3.2 Tool Runtime (Tool trait, sandbox profiles, process isolation)
- [x] 3.3 LLM Utilities (call_llm, streaming, token limiter)

### Phase 4 - Memory
- [x] 4.1 SQLite Schema (memories, embeddings, relationships)
- [x] 4.2 Embedding Provider (local HTTP, external API fallback)
- [x] 4.3 Memory Service (CRUD, semantic retrieval, episodic logging)
- [x] 4.4 Flow Integration (ContextLoaderNode, MemoryWriteNode)

### Phase 5 - Advanced Execution
- [x] 5.1 Parallel Flows (async parallel execution, concurrency limits)
- [x] 5.2 Self-Reflection Loops (looping nodes, reflection nodes)
- [x] 5.3 Batch Processing (iterable inputs, progress tracking)

### Phase 6 - Developer Experience
- [x] 6.1 CLI Runner (flow execution, flow file loading)
- [x] 6.2 Flow Builder DSL (simplified API, builder pattern)
- [x] 6.3 Debug Mode (step-by-step, state snapshots, breakpoints)
- [x] 6.4 Logging & Tracing (structured logs, event timeline)

### Phase 7 - Safety & Control
- [x] 7.1 Policy Hooks (pre/post validation, constitution-policy)
- [x] 7.2 Tool Sandbox Enforcement (permission model, capability checking)
- [x] 7.3 Rate Limiting (token budgeting, execution guardrails)

### Migration & Cleanup
- [x] 8.1 Remove Legacy Code (SequentialOrchestrator, old Agent trait)
- [x] 8.2 Documentation (README update, examples, migration guide)
- [x] 8.3 Final Testing (full test suite, performance benchmark, E2E tests)

**Status:** v1.1 complete - All phases implemented and tested
**See:** `docs/prd/prometheos-lite-prd-v1.1.md` for full specification

---

## v1.2 Roadmap - Future Improvements

### Refactor & Enhancements
- [x] Issue #22: Implement real NodeFactory in CLI runner to map node_type to concrete nodes (LLM, tool, memory, conditional nodes)
- [x] Issue #23: Unify debug and production execution by adding lifecycle hooks to Flow::run
- [x] Issue #24: Persist run registry and flow events to SQLite for Maestro process restart survival
- [x] Issue #25: Replace brute-force semantic search with indexed vector retrieval (SQLite extension or pluggable backend)
- [x] Issue #26: Retire or isolate legacy agents/core modules after parity is proven

**Status:** v1.2 refactor complete - All enhancement items implemented

---

## v1.1.2 PRD - From Architecture → Execution

**Codename:** "From Architecture → Execution"

**Objective:** Transform PrometheOS Lite from modular flow runtime into fully operational Flow execution system with real nodes, real outputs, and real workflows.

### Phase 1 — NodeFactory & Flow Execution
- [x] Issue #1: Implement NodeFactory trait
- [x] Issue #2: Replace PlaceholderNode with real nodes
- [x] Issue #3: Define Flow JSON Schema v1
- [x] Issue #4: Upgrade Flow Validation

### Phase 2 — Native Flow Nodes
- [x] Issue #5: Implement PlannerNode
- [x] Issue #6: Implement CoderNode
- [x] Issue #7: Implement ReviewerNode
- [x] Issue #8: Implement Generic LLMNode

### Phase 3 — File & Tool Integration
- [x] Issue #9: Implement FileWriterNode
- [x] Issue #10: Implement ToolNode
- [x] Issue #11: Harden Tool Sandbox

### Phase 4 — Memory Integration
- [x] Issue #12: Implement ContextLoaderNode
- [x] Issue #13: Implement MemoryWriteNode
- [x] Issue #14: Finalize Memory Schema

### Phase 5 — Tracing & Observability
- [x] Issue #15: Integrate Tracer into Flow::run
- [x] Issue #16: Implement Timeline Export
- [x] Issue #17: Integrate Debug Mode

### Phase 6 — CLI Integration
- [x] Issue #18: Upgrade Flow CLI
- [x] Issue #19: Implement Input Injection
- [x] Issue #20: Implement Output Rendering

### Phase 7 — Migration Completion
- [x] Issue #21: Replace SequentialOrchestrator
- [x] Issue #22: Convert Agents to Nodes
- [x] Issue #23: Remove Legacy Dependencies

### Phase 8 — Example Flows
- [x] Issue #24: Create Code Generation Flow
- [x] Issue #25: Create Tool Execution Flow
- [x] Issue #26: Create Memory-Augmented Flow

### Phase 9 — Testing
- [x] Issue #27: Flow Execution Tests
- [x] Issue #28: Node Tests
- [x] Issue #29: Memory Tests
- [x] Issue #30: Tool Sandbox Tests
- [x] Issue #31: CLI Tests

**Status:** v1.1.2 complete
**See:** `docs/prd/prometheos-lite-prd-v1.1.2.md` for full specification

---

## v1.1.3 Enhancement - RuntimeContext & Service Injection

**Objective:** Implement proper service injection to eliminate placeholder fallback behavior in nodes.

### Implementation
- [x] Create RuntimeContext struct for centralized service registry
- [x] Add DefaultNodeFactory::from_runtime method
- [x] Fix FileWriterNode to actually write files
- [x] Update FlowRunner to use RuntimeContext
- [x] Update CLI to create and inject RuntimeContext with real services

### Changes
- Added `src/flow/runtime.rs` - RuntimeContext with ModelRouter, ToolRuntime, MemoryService
- Updated `src/cli/runner.rs` - RuntimeContext integration in FlowRunner and DefaultNodeFactory
- Updated `src/cli/mod.rs` - CLI now creates RuntimeContext with real services
- Fixed FileWriterNode to use `std::fs::write` instead of placeholder

**Status:** v1.1.3 complete