# Loop Engineering — Specs Index

This directory contains the specification files for the PrometheOS Lite Loop Engineering Protocol.

## Files

| File | Purpose |
|---|---|
| `AGENT_PROTOCOL.md` | Detailed agent protocol with lifecycle, roles, and operating modes |
| `SAFETY_GATES.md` | Hard blockers and soft warnings specific to PrometheOS Lite |
| `PR_TEMPLATE.md` | Required PR body format for loop-engineering PRs |
| `HANDOFF_TEMPLATE.md` | Handoff report format for agent transitions |
| `PROGRESS_SCHEMA.md` | Progress tracking schema for Epic Completion Mode |
| `TASK_QUEUE_TEMPLATE.md` | Queue definition format for approved task sequences |
| `COMMENT_TEMPLATES.md` | Standardized GitHub comment templates for loop operations |

## How to use

1. Read `AGENT_PROTOCOL.md` first — it defines the operating modes and lifecycle.
2. Consult `SAFETY_GATES.md` for stop conditions.
3. Use `TASK_QUEUE_TEMPLATE.md` to define approved queues.
4. Track progress using the schema in `PROGRESS_SCHEMA.md`.
5. Use `PR_TEMPLATE.md` when opening PRs.
6. Use `HANDOFF_TEMPLATE.md` when handing off between agents or tasks.
7. Use `COMMENT_TEMPLATES.md` for standardized issue/PR communication.

## Design principles

- Spec-driven delivery: the repository is the source of truth.
- Visible evidence: every task produces artifacts and verification results.
- Hard safety boundaries: stop rules prevent scope creep and CI weakening.
- Human final approval: no unattended merges.
- Tool-agnostic: no requirement for a specific coding agent or platform.
