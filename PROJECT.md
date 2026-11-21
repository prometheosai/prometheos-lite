# PrometheOS Lite - Project Context

## Project Name
PrometheOS Lite

## Version
1.6.1 (matches Cargo.toml and CHANGELOG.md)

## Stack
- Backend: Rust (Axum web framework)
- Frontend: Next.js (React)
- LLM: LM Studio (local)
- Database: SQLite
- Flow Runtime: Custom Rust implementation

## Goals
- Local-first AI assistant
- Flow-based code generation
- Intent-aware routing
- Privacy and data sovereignty
- Efficient resource usage

## Current Phase
1.6.1 - Stable alpha: Repo Workbench (`prometheos work`). Experimental: flow execution, harness, API server, frontend, local-model validation. See docs/guides/product-surface-inventory.md and AGENTS.md.

## Important Paths
- Backend: src/
- Frontend: frontend/
- Flows: flows/
- Config: prometheos.config.json
- Control Files: .prometheos/SOUL.md, .prometheos/SKILLS.md, .prometheos/FLOWS.md, .prometheos/TOOLS.md, .prometheos/MEMORY.md
- Database: prometheos.db

## Key Features
- Intent classification (conversation vs coding tasks)
- Code generation flow (planner → coder → reviewer → file_writer → memory_write)
- WebSocket event streaming
- CORS-enabled API
- LM Studio integration
