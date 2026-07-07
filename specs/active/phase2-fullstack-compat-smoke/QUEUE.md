# Phase 2 Full-Stack Compatibility Smoke Queue

## Epic

Phase 2 Full-Stack Compatibility Smoke

## Mode

Epic Completion Mode

## Status

Approved queue, not yet executed.

## Owner

Operator: Diego Rhoger

## Purpose

This queue defines the next bounded work item for hardening the experimental frontend/API surfaces without promoting them to stable alpha: a full-stack compatibility smoke.

Phase 1 (implemented in #67) added Rust integration tests for project CRUD against the in-memory router. Phase 2 exercises the same frontend-reachable CRUD cycle the way a real deployment would: build the Rust binary, start `prometheos serve`, and verify the routes over HTTP with `curl`. This catches wiring breakage that in-memory router tests cannot (binary build, server bootstrap, config, port binding, route registration through the real server).

This queue is the definition only. Execution is a separate Epic Completion run approved after this PR merges.

## Source of truth

Agents must read these before executing this queue:

- `AGENTS.md`
- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `specs/loop-engineering/AGENT_BUDGETS.md`
- `specs/loop-engineering/PR_TEMPLATE.md`
- `specs/loop-engineering/HANDOFF_TEMPLATE.md`
- `docs/guides/frontend-api-compatibility-plan.md` (Phase 2 section)
- `docs/guides/frontend-smoke-strategy.md` (Level 3 section)
- `docs/guides/product-surface-inventory.md`
- `docs/guides/serve-api-status.md`

## Approved scope

Allowed:

- a shell-based full-stack smoke script that builds the Rust binary, starts `prometheos serve`, verifies frontend-reachable routes over HTTP with `curl`, and tears the server down
- narrow CI additions to run the full-stack smoke after the Rust build, when proven locally
- documentation updates related to the smoke implementation
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
- adding Playwright or browser automation (Level 4)
- adding production deployment claims
- adding dependencies without explicit review
- weakening CI
- skipping or narrowing tests only to pass
- changing API semantics

## Queue

### Task 1 — Full-stack smoke script

Scope:

- Implement Phase 2 of the compatibility plan: a shell script (POSIX `sh` or bash) that:
  1. builds the Rust binary (`cargo build --bin prometheos-lite` or equivalent),
  2. starts `prometheos serve` in the background on a known port,
  3. waits for readiness (poll `GET /health` until 200),
  4. verifies the frontend-reachable routes over HTTP with `curl`: at minimum `GET /health` and the project CRUD cycle (`POST /projects` → `GET /projects` → `GET /projects/:id`) matching what Phase 1 already covers in-memory,
  5. tears the server down and exits non-zero on any failure.
- Reuse the safe endpoints and route shapes already covered by Phase 1 (`tests/api_frontend_compatibility.rs`) so the script and in-memory tests assert the same contract.
- Use only `curl`, `cargo`, and shell built-ins. No new dependencies.

Boundaries:

- Do not add npm test dependencies.
- Do not add Playwright or browser automation.
- Do not change API semantics or route shapes.
- Do not change frontend source code.
- Do not promote the API server or frontend.
- Do not add model/flow execution to the smoke.

Verification:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- run the smoke script locally and confirm it builds, starts the server, passes the curls, and tears down
- frontend checks only if frontend docs or paths are touched

Stop if:

- `prometheos serve` cannot be started in the local/CI environment for environmental reasons unrelated to the script
- the binary build or server bootstrap requires config changes outside the smoke script's responsibility
- verification reveals the server does not expose the Phase 1 routes as expected (treat as a real finding, report it, do not mask)

### Task 2 — CI integration for full-stack smoke

Scope:

- Add a CI job/step that runs the full-stack smoke script after the Rust build.
- The job builds the binary, runs the script, and fails the build on non-zero exit.
- Keep it in a separate job from the unit/test matrix so a full-stack environment issue does not block the core Rust checks.

Boundaries:

- Do not weaken existing CI.
- Do not add dependencies to the workflow beyond what already exists.
- Do not promote surfaces.

Verification:

- `cargo test` still green
- the new CI job passes locally-equivalent run if runnable locally, otherwise verified in CI only (document as partial verification in handoff)
- Rust baseline

Stop if:

- CI changes conflict with existing workflow structure
- the job cannot run in the current CI runner environment
- the job would require secrets or external services

### Task 3 — Queue handoff and next-loop recommendation

Scope:

- Update `PROGRESS.md`.
- Write final handoff.
- Recommend whether the next queue should focus on:
  - Level 3 API connectivity smoke (frontend reaches API server)
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
- a task requires API behavior changes outside the smoke's verification purpose
- a task requires frontend runtime changes beyond narrow scope
- a task requires autonomous execution changes
- verification fails
- source-of-truth docs conflict
- human approval is unclear

## Human approval gate

Agents must not merge PRs.

The operator reviews and approves final merges.
