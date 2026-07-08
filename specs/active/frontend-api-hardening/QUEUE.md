# Frontend/API Experimental Surface Hardening Queue

## Epic

Frontend/API Experimental Surface Hardening

## Mode

Epic Completion Mode

## Status

Approved queue, not yet executed.

## Owner

Operator: Diego Rhoger

## Purpose

This queue defines the next bounded work items for hardening the experimental frontend and API surfaces without promoting them to stable alpha.

The goal is to let coding agents continue through approved tasks under the Loop Engineering Protocol instead of requiring the operator to manually prompt every next PR.

## Source of truth

Agents must read these before executing this queue:

- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `specs/loop-engineering/PR_TEMPLATE.md`
- `specs/loop-engineering/HANDOFF_TEMPLATE.md`
- `docs/guides/frontend-alpha-status.md`
- `docs/guides/serve-api-status.md`
- `docs/guides/local-frontend-demo.md`
- `docs/guides/product-surface-inventory.md`
- `docs/research/autonomous-loop-graduation-criteria.md`

## Approved scope

Allowed:

- documentation updates related to frontend/API experimental hardening
- frontend build/typecheck/lint decision work
- narrow frontend smoke/E2E design work
- narrow frontend/API compatibility planning
- narrow CI additions only when they are already proven locally
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
- adding production deployment claims
- adding dependencies without explicit review
- weakening CI
- skipping or narrowing tests only to pass

## Queue

### Task 1 — Frontend lint/typecheck decision

Scope:

- Inspect current frontend lint/typecheck status.
- Decide whether lint can be safely enabled now.
- If the fix is small and dependency-neutral, add or update the relevant script/CI.
- If lint/typecheck requires dependency or broader config changes, document the blocker and leave it as a follow-up.

Boundaries:

- Do not add dependencies without explicit approval.
- Do not redesign frontend components.
- Do not promote frontend.
- Do not add E2E/Playwright in this task.

Verification:

- `cd frontend && npm ci`
- `cd frontend && npm run build`
- any lint/typecheck command only if added or changed
- Rust baseline if docs or root files are touched

Stop if:

- lint requires dependency changes
- lint reveals broad unrelated frontend failures
- CI changes would be speculative

### Task 2 — Minimal frontend smoke/E2E design

Scope:

- Decide the smallest useful smoke/E2E strategy for the experimental frontend.
- Prefer docs-first design unless implementation is obviously tiny and dependency-neutral.
- Define what the smoke test should prove and what it should not prove.

Boundaries:

- Do not add Playwright unless explicitly approved.
- Do not add dependencies without explicit approval.
- Do not claim E2E coverage exists unless implemented and passing.
- Do not promote frontend.

Verification:

- Rust baseline for docs-only changes
- frontend build if frontend docs or paths are touched

Stop if:

- implementation requires new test dependencies
- test design requires API behavior changes
- test design expands into full frontend coverage

### Task 3 — Frontend/API compatibility smoke plan

Scope:

- Define the narrowest frontend/API compatibility smoke path.
- Use existing API smoke coverage as context.
- Identify which route or behavior should be tested first.
- Prefer plan/docs unless the implementation is tiny and already supported.

Boundaries:

- Do not add broad route coverage.
- Do not change API semantics.
- Do not add auth.
- Do not promote API server.
- Do not promote frontend.
- Do not add WebSocket tests unless explicitly scoped.

Verification:

- Rust baseline
- frontend build if frontend docs or paths are touched

Stop if:

- route behavior is unclear
- implementation needs runtime/API changes
- compatibility test requires E2E infrastructure

### Task 4 — Queue handoff and next-loop recommendation

Scope:

- Update `PROGRESS.md`.
- Write final handoff.
- Recommend whether the next queue should focus on:
  - loop structure validator
  - GitHub labels/comments
  - frontend/API smoke implementation
  - autonomous loop prerequisite work

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
