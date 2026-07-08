# Phase 2 Full-Stack Compatibility Smoke Progress

## Mode

Epic Completion Mode

## Status

Queue defined. Not yet executed. This PR defines the queue; execution is a separate approved run.

## Approved scope

See `QUEUE.md`.

## Current queue

- [ ] Task 1 — Full-stack smoke script
- [ ] Task 2 — CI integration for full-stack smoke
- [ ] Task 3 — Queue handoff and next-loop recommendation

## Completed tasks

| Task | Commit | Files | Verification | Notes |
|---|---|---|---|---|
| Queue definition | (this PR) | `QUEUE.md`, `PROGRESS.md`, `HANDOFF.md` | docs-only | Phase 2 full-stack compatibility smoke queue defined, not executed |

## Current task

None. Queue defined, awaiting execution approval.

## Blockers

None.

## Scope notes

This is a planning/queue-definition PR only. It adds no runtime behavior, no dependencies, and does not touch `OVERVIEW.md`, `README.md`, `AGENTS.md`, or any source file. It defines the Phase 2 follow-on to #67 (Phase 1 project CRUD integration tests) and the `frontend-api-compatibility-plan.md` Phase 2 section.

## Verification evidence

Docs-only. No `cargo` / `npm` commands required; no Rust or frontend behavior changed.

Cross-checked the queue scope against source-of-truth docs:

| Queue claim | Source match |
|---|---|
| Phase 2 builds binary, starts `prometheos serve`, curls frontend-reachable routes | `frontend-api-compatibility-plan.md` Phase 2 section |
| Routes verified: health + project CRUD | matches Phase 1 routes in `tests/api_frontend_compatibility.rs` and compatibility plan Step 1–2 |
| No Playwright / no new deps / no frontend source change | matches `frontend-smoke-strategy.md` (Level 4 excluded) and plan "what this does NOT do" |
| API server / frontend remain experimental | `docs/guides/product-surface-inventory.md` (serve = experimental, frontend = experimental) |

## Stop / continue decision

Stop. Queue defined. Execution is a separate Epic Completion run to be approved after merge.

## Next recommended action

Merge this PR to register the queue. Then execute under Epic Completion Mode in a follow-up PR (head into Task 1).
