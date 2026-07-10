# Loop Structure Validator Handoff

## Current state

Queue approved and defined. Not yet executed. This PR is the queue/specs PR only; the implementation is a separate follow-up PR (#73) per the established loop pattern (queue first, execute second).

## Completed work

### Task 0 — Queue definition (this PR)

- Authored `specs/active/loop-structure-validator/QUEUE.md` with the canonical required-heading set for `QUEUE.md`, `PROGRESS.md`, and `HANDOFF.md`, plus the two cross-file consistency invariants.
- Authored `specs/active/loop-structure-validator/PROGRESS.md` and this handoff, themselves conforming to the canonical structure the validator will enforce (dogfooding).
- Scoped the validator as a zero-dependency POSIX `sh` script with no model and no external calls.
- Replaced "promoting frontend/API server to stable alpha" wording with surface-maturity-classification wording to avoid ReviewOnly promotion blockers.

**Files:** `specs/active/loop-structure-validator/QUEUE.md`, `specs/active/loop-structure-validator/PROGRESS.md`, `specs/active/loop-structure-validator/HANDOFF.md`

## Verification run

| Command | Result |
|---|---|
| `cargo fmt --check` | not applicable (no Rust changed) |
| `cargo check` | not applicable (no Rust changed) |
| `cargo test` | not applicable (no Rust changed) |
| `cargo clippy --all-targets --all-features -- -D warnings` | not applicable (no Rust changed) |
| `bash scripts/loop-structure-validator.sh` | pending (script authored in #73) |

## What was not run

- The validator script itself (implemented in #73).
- The self-test against existing active queues (#73, Task 2).
- CI integration (#73, Task 3).
- Runtime/product behavior changes (intentionally out of scope).

## Blockers

None. The active-queue pattern and templates already exist; the validator only enforces them.

## Risks / findings

- Some existing active queues may not yet carry every canonical heading (e.g., heading naming drift such as `Risks` vs `Risks / findings`). Task 2 of #73 brings them into conformance and records exactly which were touched.
- The all-caps `BLOCKER` marker convention for invariant B is chosen to avoid false positives from ordinary prose containing the word "blocker". This convention must be documented in #73 so future handoffs use it deliberately.

## Next task

None in this PR. After merge, the follow-up implementation PR executes:
- Task 1: implement `scripts/loop-structure-validator.sh`.
- Task 2: self-test and conform existing active queues.
- Task 3: CI job + handoff.

## Stop reason

Queue definition complete. Handing off for separate execution PR (#73) per loop protocol.

## Confidence

High. Scope is contained (loop paperwork only), zero new dependencies, no runtime/product behavior changes, and the queue file set itself conforms to the structure it defines.
