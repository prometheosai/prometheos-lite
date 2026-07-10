# Loop Structure Validator Queue

## Epic

Loop Structure Validator

## Mode

Epic Completion Mode

## Status

Approved queue, not yet executed.

## Owner

Operator: Diego Rhoger

## Purpose

This queue defines the next bounded work item: a deterministic Loop Structure Validator that checks the loop system's own paperwork. The active-queue pattern (`QUEUE.md`, `PROGRESS.md`, `HANDOFF.md`) is now core infrastructure with multiple live queues (#70, #71). Before adding more surface coverage, the loop should validate its own clipboard robot's filing cabinet.

The validator is a zero-dependency, deterministic checker (no model, no external calls). It scans `specs/active/*/` and asserts that every active queue directory carries the three required files, that each file contains its required structural headings, and that two cross-file consistency invariants hold. It exits non-zero and prints a clear report on any violation so CI (and humans) get yelled at.

This queue is the definition only. Execution is a separate Epic Completion run approved after this PR merges.

## Budget note

3 files, +307/-0. Exceeds the 200-line preference, accepted because this PR defines the canonical validator rules and required heading set. Execution remains separate in #73.

## Source of truth

Agents must read these before executing this queue:

- `AGENTS.md`
- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `specs/loop-engineering/AGENT_BUDGETS.md`
- `specs/loop-engineering/PR_TEMPLATE.md`
- `specs/loop-engineering/HANDOFF_TEMPLATE.md`
- `specs/loop-engineering/TASK_QUEUE_TEMPLATE.md`
- `specs/loop-engineering/PROGRESS_SCHEMA.md`
- existing active queues for dogfooding (`specs/active/*/`)

## Approved scope

Allowed:

- a deterministic, zero-dependency shell script (`scripts/loop-structure-validator.sh`) that scans `specs/active/*/`
- structural heading checks for `QUEUE.md`, `PROGRESS.md`, `HANDOFF.md`
- two cross-file consistency checks (no "complete" with unchecked tasks; no "no blockers" with blocker markers)
- a CI job that runs the validator over `specs/active/`
- bringing existing active-queue paperwork into conformance with the canonical structure (loop paperwork only)
- progress/handoff updates for this queue

Not allowed:

- changing frontend maturity classification
- changing API server maturity classification
- changing stable-alpha scope
- changing experimental/future surface maturity labels
- changing `prometheos work` behavior
- adding autonomous execution behavior
- modifying harness autonomous execution
- adding model/provider behavior
- adding Brain or Mnemosyne integration
- adding runtime behavior changes outside the loop paperwork
- weakening CI
- adding dependencies without explicit review
- skipping or narrowing tests only to pass
- changing product/runtime semantics

## Queue

### Task 1 — Loop structure validator script

Scope:

- Implement `scripts/loop-structure-validator.sh`: a POSIX `sh` script (no `bash`-only features required; runs under `bash` too) with zero external dependencies beyond shell built-ins and `grep`/`find`.
- Scan every immediate subdirectory of `specs/active/` as an active queue directory.
- For each active queue directory, assert all three files exist: `QUEUE.md`, `PROGRESS.md`, `HANDOFF.md`. Missing file = failure.
- For `QUEUE.md`, assert these `##` headings exist (case-insensitive match on the heading text):
  - `Mode`
  - `Status`
  - `Source of truth`
  - `Approved scope`
  - `Queue`
  - `Global stop conditions`
  - `Human approval gate`
- For `PROGRESS.md`, assert these `##` headings exist (case-insensitive):
  - `Status`
  - `Current queue`
  - `Completed tasks`
  - `Verification evidence`
  - `Stop / continue decision`
- For `HANDOFF.md`, assert these `##` headings exist (case-insensitive):
  - `Current state`
  - `Completed work`
  - `Verification run`
  - `What was not run`
  - `Blockers`
  - a heading containing both `risk` and `finding` (e.g., `Risks / findings`)
  - `Next task`
  - `Stop reason`
  - `Confidence`
- Consistency invariant A: if `PROGRESS.md` `Status` or `Stop / continue decision` contains the word `complete` (case-insensitive) while the `Current queue` section still contains an unchecked task line (`- [ ]`), fail.
- Consistency invariant B: if the `HANDOFF.md` `Blockers` section body is empty or matches `none` / `no blockers` (case-insensitive), but the file contains a line with the all-caps marker `BLOCKER`, fail.
- On any failure, accumulate all violations and print them as `file:rule` lines (e.g., `specs/active/foo/QUEUE.md:missing-heading 'Mode'`). Exit non-zero only after reporting every violation. On success, print a one-line `LOOP STRUCTURE OK` and exit 0.
- Make the scanned root overridable via an optional first argument (default `specs/active`) so it can be reused.

Boundaries:

- Do not add npm or cargo dependencies.
- Do not change runtime/product behavior.
- Do not change the active-queue content semantics, only enforce structure.
- Do not change surface maturity classifications.
- Do not add model/flow execution.

Verification:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- run `bash scripts/loop-structure-validator.sh` and confirm it exits 0 when the active queues conform and non-zero when a seeded violation is present (add a temporary local violation to prove the negative path, then revert)
- frontend checks only if frontend files are touched (none here)

Stop if:

- the required heading set cannot be expressed deterministically from the established templates
- a task requires dependency changes
- verification reveals the active queues cannot be brought into conformance without semantic rewrites (report, do not mask)

### Task 2 — Self-test and bring active paperwork into conformance

Scope:

- Run `bash scripts/loop-structure-validator.sh` over the current `specs/active/`.
- For any active queue that fails, edit only its loop paperwork (`QUEUE.md` / `PROGRESS.md` / `HANDOFF.md`) to satisfy the canonical structure. Do not alter runtime code or product claims.
- Re-run until the validator exits 0 across all active queues.
- Record which queues were touched and why in the progress file.

Boundaries:

- Loop paperwork only. No changes to `src/`, `frontend/`, CI behavior, or product semantics.
- Do not rewrite queue intent or content; only add/relabel structural headings to match the canonical set.
- Do not change surface maturity classifications.

Verification:

- `bash scripts/loop-structure-validator.sh` exits 0
- Rust baseline still green (no runtime changed, but run to record evidence)
- frontend checks only if frontend files are touched (none here)

Stop if:

- a queue cannot be conformed without changing its approved scope or intent (report as a finding, do not force)
- verification fails

### Task 3 — CI integration and queue handoff

Scope:

- Add a CI job/step that runs `bash scripts/loop-structure-validator.sh` over `specs/active/`.
- Keep it separate from the `rust-checks` matrix so a paperwork failure does not block core Rust checks, mirroring the `fullstack-smoke` job pattern.
- Update `PROGRESS.md` and write the final handoff (`HANDOFF.md`).

Boundaries:

- Do not weaken existing CI.
- Do not change surface maturity classifications.
- Do not add dependencies to the workflow beyond what already exists.

Verification:

- `cargo test` still green
- the new CI job runs the validator (verified in CI; document as live evidence in handoff)
- Rust baseline

Stop if:

- CI changes conflict with existing workflow structure
- the job would require secrets or external services

## Global stop conditions

Stop immediately and hand off if any hard blocker from `specs/loop-engineering/SAFETY_GATES.md` appears.

Also stop if:

- scope expands beyond this queue
- a task requires dependency changes
- a task requires API/product behavior changes outside loop paperwork
- a task requires frontend runtime changes
- a task requires autonomous execution changes
- verification fails
- source-of-truth docs conflict
- human approval is unclear

## Human approval gate

Agents must not merge PRs.

The operator reviews and approves final merges.
