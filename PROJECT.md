# PrometheOS Lite - Project Context

## Project Name
PrometheOS Lite

## Version
v1.2.1

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
v1.2.1 - Intent Classification Layer

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
