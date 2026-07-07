# Phase 2 Full-Stack Compatibility Smoke Handoff

## Current state

Queue defined, not executed. This PR (`#69`) registers the Phase 2 full-stack compatibility smoke queue under `specs/active/phase2-fullstack-compat-smoke/`. Execution is a separate Epic Completion run to be approved after merge.

## Completed work

### Queue definition (this PR)

- Created `specs/active/phase2-fullstack-compat-smoke/QUEUE.md` defining three tasks:
  - Task 1 — Full-stack smoke script (build binary, start `prometheos serve`, `curl` health + project CRUD, teardown).
  - Task 2 — CI integration for the full-stack smoke (separate job after the Rust build).
  - Task 3 — Queue handoff and next-loop recommendation.
- Created `PROGRESS.md` and this `HANDOFF.md`.

**Files:** `specs/active/phase2-fullstack-compat-smoke/QUEUE.md`, `PROGRESS.md`, `HANDOFF.md`

## Verification run

Docs-only change. No Rust or frontend behavior changed, so no `cargo` / `npm` build was required by `docs/LOOP_ENGINEERING.md` (docs-only rule).

Cross-checked the queue scope against source-of-truth docs:

| Queue claim | Source match |
|---|---|
| Phase 2 builds binary, starts `prometheos serve`, curls frontend-reachable routes | `frontend-api-compatibility-plan.md` Phase 2 section |
| Routes verified: health + project CRUD | matches Phase 1 routes in `tests/api_frontend_compatibility.rs` and compatibility plan Step 1–2 |
| No Playwright / no new deps / no frontend source change | matches `frontend-smoke-strategy.md` (Level 4 excluded) and plan "what this does NOT do" |
| API server / frontend remain experimental | `docs/guides/product-surface-inventory.md` (serve = experimental, frontend = experimental) |

## What was not run

- Task 1–3 were not executed; this PR defines the queue only.
- No `cargo build` / `prometheos serve` / `curl` smoke was run.
- No `npm` / frontend build was run.
- No CI job was added (that is Task 2 of the execution run).

## Blockers

None encountered.

## Risks

- Low. This is a planning PR. The main execution risk (flagged in Task 1 stop conditions) is that `prometheos serve` startup in CI may need specific config or a writable DB path; the execution queue must report that as a real finding rather than masking it. This is noted in the queue, not resolved here.

## Next queue recommendation

After merge, execute the queue under Epic Completion Mode:

- **Primary:** Task 1 (full-stack smoke script) → Task 2 (CI job) → Task 3 (handoff).
- Secondary candidates for a later queue (not part of this one):
  - **Level 3 API connectivity smoke** — verify the frontend can reach the API server (separate from Phase 2 route smoke; see `frontend-smoke-strategy.md` Level 3).
  - **ReviewOnly GitHub Action implementation** — the read-only PR review Action (Level 1 of the automation ladder), now that `AGENTS.md` house rules exist.
  - **Loop structure validator** — validate `QUEUE.md` / `PROGRESS.md` / `HANDOFF.md` structure.

## Stop reason

Queue defined, not executed. Handoff documents the definition only; execution is a separate approved run.

## Confidence

High. Scope contained, no boundary violations, no dependencies, no behavior changed, and queue scope matches the compatibility plan Phase 2 and smoke strategy.
