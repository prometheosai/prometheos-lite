# ReviewOnly GitHub Action v0 Progress

## Mode

Epic Completion Mode (single implementation PR; not a queue-definition PR)

## Status

Implementation complete. PR open for human review.

## Approved scope

Operator direction (updated from earlier #70 queue-definition plan):

- Add `.github/workflows/prometheos-reviewonly.yml`
- Add `scripts/reviewonly/reviewonly.mjs` (deterministic, no model)
- Update `docs/guides/prometheos-reviewonly-github-flow.md` to reflect v0
- Add `specs/active/reviewonly-github-action/PROGRESS.md` and `HANDOFF.md`

## Current queue

- [x] Implement ReviewOnly GitHub Action v0 (workflow + script + docs + progress/handoff)

## Completed tasks

| Task | Commit | Files | Verification | Notes |
|---|---|---|---|---|
| Implement ReviewOnly v0 | (this PR) | `.github/workflows/prometheos-reviewonly.yml`, `scripts/reviewonly/reviewonly.mjs`, `docs/guides/prometheos-reviewonly-github-flow.md`, `specs/active/reviewonly-github-action/PROGRESS.md`, `specs/active/reviewonly-github-action/HANDOFF.md` | `node --check` syntax valid; self-triggers on this PR for live verification | Deterministic, no model, comments only |

## Current task

None. Implementation complete.

## Blockers

None.

## Scope notes

- No model is invoked. v0 is fully deterministic.
- Permissions are `contents: read`, `pull-requests: write` only. No branch writes, no merge, no commits.
- The script exits 0 even on internal error so it never blocks CI; failures surface as a warning comment.
- This is Level 1 (ReviewOnly) automation. It does not approve or merge PRs.
- #69 (Phase 2 full-stack smoke queue) remains a registered backlog queue; this PR does not execute it.

## Verification evidence

| Command | Result |
|---|---|
| `node --check scripts/reviewonly/reviewonly.mjs` | passed (valid syntax) |
| CI (self-trigger) | the action runs on this PR (#70) and posts a ReviewOnly report comment — live proof of behavior |

No `cargo` / `npm` build required: the change is a GitHub Action + a Node script with no npm dependencies, plus docs.

## Stop / continue decision

Stop. Implementation complete. Create PR for human review.

## Next recommended action

Merge after CI green and human review. After merge, the bot runs on every PR automatically. Future work (separate PRs):
- Level 3 API connectivity smoke (frontend reaches API server).
- Loop structure validator.
- LLM-powered ReviewOnly (only after the deterministic v0 is proven stable).
