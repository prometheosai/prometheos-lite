# Changelog

## [v0.1.0] - Initial Foundation (Issue #1 Complete)
*   **Added:** Basic CLI entrypoint using `clap`. The core command structure `prometheos run "<task>"` is now functional, providing the initial mechanism to execute tasks through the system.
*   **Impact:** Establishes the primary user interaction point for the PrometheOS Lite agent framework.

## [v0.2.0] - Project Structure Scaffolding (Issue #2 Complete)
The next major milestone is defining and scaffolding the modular project structure across `src/cli`, `src/agents`, `src/core`, etc., to ensure maintainability and scalability as more components are added. This step organizes the codebase according to best practices for a multi-agent system in Rust, ensuring clear boundaries between concerns like CLI handling, agent logic, core utilities, and configuration management.

## [v0.3.0] - Core Agent System Complete (Issues #1-#10)
*   **Phase 0 - Foundation:** Rust workspace initialized, CLI with clap, async runtime with tokio, modular project structure
*   **Phase 1 - LLM Integration:** Local-first LLM client (reqwest), config loader with JSON support
*   **Phase 2 - Agent System:** Agent trait, Planner agent, Coder agent, Reviewer agent
*   **Phase 3 - Orchestration:** Sequential orchestrator coordinating Planner → Coder → Reviewer pipeline
*   **Impact:** Fully functional multi-agent CLI that can plan, generate, and review code using local LLMs

## [v0.4.0] - Real-Time Experience & File System (Issues #11-#17 Complete)
*   **Phase 4 - Real-Time Experience:** Structured logger with agent-based logging, streaming renderer with callback support, execution timeline events in orchestrator
*   **Phase 5 - File System:** File parser extracting files from markdown code blocks, file writer with `/prometheos-output` directory and conflict handling
*   **Phase 6 - CLI Experience:** CLI output improvements with verbose flag, loading states, output directory printing, success/failure messages
*   **Error Handling:** Retry logic with exponential backoff in LLM client (3 retries by default)
*   **Impact:** Full file generation pipeline with real-time logging and robust error handling

## [v0.5.0] - Production Ready (Issues #18-#21 Complete)
*   **Phase 7 - Release Readiness:** LLM timeout increased to 300s for complex tasks, full pipeline testing with SaaS landing page generation
*   **Bug Fixes:** File parser regex fixed to correctly associate markdown headers with code blocks
*   **Testing:** Verified end-to-end pipeline with LM Studio (google/gemma-4-eb model)
*   **Impact:** Production-ready multi-agent CLI capable of handling complex generation tasks
*   **Status:** All core features from v1.0 PRD implemented and tested

## [v1.1.0] - Flow-Centric Architecture (Next Major)
*   **Phase 1 - Flow Core:** Node trait, Flow engine, SharedState, action routing
*   **Phase 2 - Orchestration:** Maestro, continuation engine, run registry
*   **Phase 3 - Intelligence:** Model router, tools runtime, LLM utilities
*   **Phase 4 - Memory:** Memory service integration, context nodes
*   **Phase 5 - Advanced Execution:** Batch flows, parallel execution, looping nodes
*   **Phase 6 - Developer Experience:** CLI runner, flow builder DSL, debug mode
*   **Phase 7 - Safety & Control:** Policy hooks, tool sandbox, rate limiting
*   **See:** `docs/prd/prometheos-lite-prd-v1.1.md` for full specification