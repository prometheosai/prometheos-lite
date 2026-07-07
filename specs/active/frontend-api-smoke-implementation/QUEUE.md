# Frontend/API Smoke Implementation Queue

## Epic

Frontend/API Smoke Implementation

## Mode

Epic Completion Mode

## Status

Approved queue, not yet executed.

## Owner

Operator: Diego Rhoger

## Purpose

This queue converts the frontend smoke strategy and frontend/API compatibility plan from design docs into enforced verification.

The previous queue (Frontend/API Experimental Surface Hardening) produced four deliverables, including design docs for smoke testing and compatibility testing. This queue implements the first actionable layer of those plans.

## Source of truth

Agents must read these before executing this queue:

- `AGENTS.md`
- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `specs/loop-engineering/AGENT_BUDGETS.md`
- `specs/loop-engineering/GITHUB_AUTOMATION_LEVELS.md`
- `specs/loop-engineering/PR_TEMPLATE.md`
- `specs/loop-engineering/HANDOFF_TEMPLATE.md`
- `docs/guides/frontend-smoke-strategy.md`
- `docs/guides/frontend-api-compatibility-plan.md`
- `docs/guides/product-surface-inventory.md`

## Approved scope

Allowed:

- Rust integration tests for project CRUD using in-memory router (Phase 1 of compatibility plan)
- Server smoke script that starts the frontend dev server and verifies it responds (Level 2 of smoke strategy)
- narrow CI additions for smoke steps when proven locally
- documentation updates related to smoke implementation
- progress/handoff updates for this queue

Not allowed:

- promoting frontend to stable alpha
- promoting API server to stable alpha
- changing stable alpha scope
- changing `prometheos work` behavior
- adding autonomous execution behavior
- modifying harness autonomous execution
- adding model/provider behavior
- adding Brain or Mnemosyne integration
- adding full E2E/Playwright tests (Level 4)
- adding production deployment claims
- adding dependencies without explicit review
- weakening CI
- skipping or narrowing tests only to pass

## Queue

### Task 1 — Project CRUD Rust integration tests

Scope:

- Implement Phase 1 of the compatibility smoke plan: Rust integration tests for project CRUD (`POST /projects` → `GET /projects` → `GET /projects/:id`).
- Use existing test infrastructure (in-memory router, no model provider needed).
- Tests should verify the API router handles project lifecycle correctly.

Boundaries:

- Do not add broad route coverage.
- Do not change API semantics.
- Do not add auth.
- Do not add dependencies.
- Do not promote API server.

Verification:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- frontend checks only if frontend docs or paths are touched

Stop if:

- route behavior is unclear
- implementation requires runtime/API changes
- tests require a model provider
- test infrastructure changes require dependencies

### Task 2 — Server smoke script

Scope:

- Implement Level 2 of the frontend smoke strategy: a server smoke script that starts the frontend dev server and verifies it responds on the expected port.
- Use Node.js with no additional dependencies (use built-in `http` module).
- Add a CI step to run the smoke script after the build step.

Boundaries:

- Do not add Playwright or any test framework dependency.
- Do not add E2E tests.
- Do not promote frontend.
- Do not modify frontend source code.

Verification:

- `cd frontend && npm ci`
- `cd frontend && npm run lint`
- `cd frontend && npm run build`
- Rust baseline if docs or root files are touched

Stop if:

- implementation requires additional npm dependencies
- smoke script requires API server to be running
- CI changes conflict with existing workflow structure

### Task 3 — Queue handoff and next-loop recommendation

Scope:

- Update `PROGRESS.md`.
- Write final handoff.
- Recommend whether the next queue should focus on:
  - Phase 2 of compatibility plan (full-stack smoke)
  - Level 3 of smoke strategy (API connectivity smoke)
  - ReviewOnly GitHub Action implementation
  - loop structure validator

Boundaries:

- Do not start the next queue.
- Do not modify runtime behavior.
- Do not claim completed work that was not verified.

Verification:

- Rust baseline
- frontend build if frontend docs or paths are touched

## Global stop conditions

Stop immediately and hand off if any hard blocker from `specs/loop-engineering/SAFETY_GATES.md` appears.

Also stop if:

- scope expands beyond this queue
- a task requires dependency changes
- a task requires API behavior changes
- a task requires frontend runtime changes beyond narrow scope
- a task requires autonomous execution changes
- verification fails
- source-of-truth docs conflict
- human approval is unclear

## Human approval gate

Agents must not merge PRs.

The operator reviews and approves final merges.
