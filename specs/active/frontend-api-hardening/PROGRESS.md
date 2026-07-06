# Frontend/API Experimental Surface Hardening Progress

## Mode

Epic Completion Mode

## Status

Task 2 complete. Continuing to Task 3.

## Approved scope

See `QUEUE.md`.

## Current queue

- [x] Task 1 — Frontend lint/typecheck decision
- [x] Task 2 — Minimal frontend smoke/E2E design
- [ ] Task 3 — Frontend/API compatibility smoke plan
- [ ] Task 4 — Queue handoff and next-loop recommendation

## Completed tasks

| Task | Commit | Files | Verification | Notes |
|---|---|---|---|---|
| Queue creation | 5e007d86 | `QUEUE.md`, `PROGRESS.md`, `HANDOFF.md` | PR #63 verified | Active queue created, not executed |
| Task 1 | 383dcc8 | `.github/workflows/frontend-ci.yml`, `PROGRESS.md`, `docs/guides/frontend-alpha-status.md` | `npm run lint` — exit 0, 3 warnings | Lint enabled in CI. 3 pre-existing warnings documented. |
| Task 2 | pending | `docs/guides/frontend-smoke-strategy.md`, `PROGRESS.md` | Rust baseline pass | Docs-first design. Defines 4-level strategy. No implementation. |

## Current task

Task 3 — Frontend/API compatibility smoke plan.

## Blockers

None.

## Verification evidence

Task 1 — `npm run lint` exit 0. 3 warnings:
- `react-hooks/exhaustive-deps` in `conversations/[id]/page.tsx:28`
- `react-hooks/exhaustive-deps` in `projects/[id]/page.tsx:17`
- `@next/next/no-img-element` in `profile-modal.tsx:680`
- `next lint` deprecation notice (Next.js 15.5, informational only)

All warnings are pre-existing and non-blocking. No errors.

## Stop / continue decision

Continue to Task 2 after PR for Task 1 merges.

## Next recommended action

Execute Task 3 — Frontend/API compatibility smoke plan.
