# ReviewOnly GitHub Action v0 Handoff

## Current state

Implementation complete and patched. PR #70 is open for human review. The deterministic ReviewOnly v0 automation is implemented and self-tests by running on its own PR.

## Completed work

### Workflow — `.github/workflows/prometheos-reviewonly.yml`

- Triggers on `pull_request` `opened` / `synchronize` / `reopened`.
- Permissions: `contents: read`, `pull-requests: write` only.
- Checks out the repo and runs `node scripts/reviewonly/reviewonly.mjs`.

### Script — `scripts/reviewonly/reviewonly.mjs`

- Deterministic, no external models, no npm dependencies.
- Reads PR metadata + diff via `gh`, using `execFileSync("gh", argv, ...)` (argv array, no shell interpolation). Report bodies are passed through stdin (`--input -`).
- Blocker-level checks: dependency files changed without approval, CI/workflow weakening, secrets/credentials, source-of-truth conflict markers, *affirmative* promotion/overclaim of experimental surfaces to stable alpha, benchmark claims without evidence.
- Warning-level checks: >5 files, >200 net lines, runtime/API/frontend/harness paths touched, missing verification for touched area (classified by touched area), docs-only claim with non-docs files changed.
- Promotion heuristic requires affirmative promotion language and exempts negated safety-boundary phrasing ("no", "not", "experimental", "future / not alpha"); uncertain matches downgrade to Warning.
- Verification classification splits touched areas (`srcTouched`, `frontendTouched`, `workflowTouched`, `docsTouched`, `scriptTouched`); Rust baseline is required only for `src/**` / `Cargo.*` / Rust workflows, while workflow/script changes expect `node --check` + action self-trigger + CI green.
- Produces one structured `## PrometheOS ReviewOnly Report` comment, deduped across pushes (edits the prior bot comment instead of stacking new ones).
- Always exits 0 so it never blocks CI; internal errors surface as a warning comment.

### Docs — `docs/guides/prometheos-reviewonly-github-flow.md`

- Updated from "planned design, no automation" to reflect v0 implementation.
- Added an "Implementation status" note and a "v0 implementation" subsection (workflow path, script path, behavior, checks, permission model).

### Progress/handoff

- `specs/active/reviewonly-github-action/PROGRESS.md`, `HANDOFF.md` (this file).

**Files:** `.github/workflows/prometheos-reviewonly.yml`, `scripts/reviewonly/reviewonly.mjs`, `docs/guides/prometheos-reviewonly-github-flow.md`, `specs/active/reviewonly-github-action/PROGRESS.md`, `specs/active/reviewonly-github-action/HANDOFF.md`

## Verification run

| Command | Result |
|---|---|
| `node --check scripts/reviewonly/reviewonly.mjs` | passed |
| Self-trigger CI | the action executes on PR #70 and posts a ReviewOnly report comment |

No `cargo` / `npm` build required (GitHub Action + dependency-free Node script + docs).

## What was not run

- LLM-powered review — intentionally out of scope for v0.
- Formal blocking review via `gh pr review --request-changes` — v0 posts a normal comment only, per the ReviewOnly spec.
- Local end-to-end run against a real PR — the action is verified live by running on this PR in CI.

## Blockers

None in the final (patched) implementation. The first live self-trigger reported a false-positive blocker: the original overclaim heuristic matched normal safety-boundary language ("No frontend / API / autonomous promotion"). This was caught before merge and fixed by `classifyPromotion`, which requires affirmative promotion wording and exempts negated safety-boundary phrases. See `PROGRESS.md` patch notes.

## Risks

- The bot posts on every PR opened/synchronize/reopened. Comment volume is bounded by dedupe (it edits its own prior comment). If `gh` permissions or the token change, the script degrades to a warning comment and still exits 0.
- Promotion/overclaim and benchmark detection are keyword heuristics, not semantic analysis; they may produce false positives. Affirmative-language requirements and negation exemptions reduce noise, but the checks are flagged for human review, not auto-blocking the merge (the action cannot merge regardless).
- v0 does not call models, so it cannot catch subtle logic issues a human reviewer would. It is a first-pass governance speedup, not a replacement for human review (per the ReviewOnly spec).

## Next task

None in this PR. After merge:
- Observe the bot's comments across a few PRs; tune heuristics if noisy.
- Level 3 API connectivity smoke (separate PR).
- Loop structure validator (separate PR).
- LLM-powered ReviewOnly (only after deterministic v0 is proven stable; separate PR, requires explicit approval).

## Stop reason

Implementation complete. Awaiting human review and merge.

## Confidence

High. Scope contained, no model dependency, no new npm packages, no behavioral promotion, permissions minimal, and the bot self-verifies by running on this PR.
