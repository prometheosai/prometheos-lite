# Phase 2 Full-Stack Compatibility Smoke Handoff

## Current state

Queue executed. All 3 tasks complete. PR #71 is open for human review. This is the first implementation PR reviewed by the now-live ReviewOnly bot — a real test of the new operating model.

## Completed work

### Task 1 — Full-stack smoke script (`scripts/fullstack-smoke.sh`)

- Bash script, no npm deps, no Playwright, uses only `curl`, `cargo`, and shell built-ins.
- Builds the `prometheos` binary, starts `prometheos serve` in an isolated temp dir (config copied so the repo tree stays clean), polls `GET /health` until ready, then verifies the project CRUD cycle over real HTTP: `POST /projects` → 201, `GET /projects` → 200 (name present), `GET /projects/:id` → 200 (name present).
- Always tears down the server and temp dir via `trap`, exits non-zero on any failure.
- Locally verified end-to-end via Git Bash: `FULLSTACK SMOKE PASSED`.

### Task 2 — CI integration (`.github/workflows/ci.yml`)

- Added a separate `fullstack-smoke` job (ubuntu-latest) that checks out, installs Rust, uses the existing cargo cache, and runs `bash scripts/fullstack-smoke.sh`.
- Kept separate from the `rust-checks` matrix so a full-stack environment issue does not block core Rust checks. No existing CI weakened.

### Task 3 — Queue handoff (this file + `PROGRESS.md`)

- Finalized progress and wrote this handoff.

**Files:** `scripts/fullstack-smoke.sh`, `.github/workflows/ci.yml`, `specs/active/phase2-fullstack-compat-smoke/PROGRESS.md`, `specs/active/phase2-fullstack-compat-smoke/HANDOFF.md`

## Verification run

| Command | Result |
|---|---|
| `bash scripts/fullstack-smoke.sh` | PASSED |
| `cargo fmt --check` | passed (no Rust changed) |
| `cargo clippy --all-targets --all-features -- -D warnings` | passed (no Rust changed) |
| `cargo test` | unaffected (no Rust behavior changed) |

The new `fullstack-smoke` CI job runs on this PR as live proof. ReviewOnly also posts a report on this PR.

## What was not run

- Model/flow execution paths (intentionally out of scope; the smoke is data-only).
- Frontend server / browser (Level 3 API connectivity smoke is a separate future queue).
- WebSocket, auth, conversation/message CRUD beyond project CRUD (Phase 2 scope per compatibility plan).

## Blockers

None. `prometheos serve` boots offline; the smoke exercises only health + project CRUD, which work without a provider.

## Risks / findings

- **Startup warning (benign):** `prometheos serve` logs `WARN ... OpenRouter provider has no API key set in env 'OPENROUTER_API_KEY'` at boot. This is non-fatal — model calls fail only at request time, and the smoke does not invoke models. Reported here per the explicit instruction not to mask server bootstrap behavior. No duct-taping was applied; the smoke simply does not depend on a model.
- The smoke script hardcodes an isolated port (`3100`) and an isolated temp work dir, so it will not collide with a default `prometheos serve` on `3000` nor pollute the repo with `prometheos.db`.
- If `prometheos serve` ever changes its config requirements or port flags, the script will fail clearly (the readiness poll detects early exit and dumps the server log). That is intended — it should surface real wiring regressions, not hide them.

## Next task

None in this PR. After merge:
- Level 3 API connectivity smoke (frontend reaches API server) — separate PR.
- Loop structure validator (validate QUEUE/PROGRESS/HANDOFF structure) — separate PR.
- LLM-powered ReviewOnly only after deterministic v0 is proven stable — separate PR, explicit approval.

## Stop reason

Queue complete. All 3 tasks executed and verified.

## Confidence

High. Scope contained, no boundary violations, no dependencies, no runtime/API behavior changes, and the smoke is verified live (local run + CI self-trigger). The bot reviewer gets to watch another robot try `curl`.
