# Handoff

_Last updated: September 25, 2026, after PR #227 (cooperative in-flight cancellation, #222) merged as `c16d820`, plus the #228 cross-process exit-race repair._

## Authority state

- `main` = `c16d820` — squash-merge of PR #227 (closes #222) at reviewed head `d056a93`; evidence binds to the reviewed head (core 11/11, platform 8/8, smoke 9/9 at `d056a93`; tree-equal to `c16d820` by empty diff).
- Tally: **28 issues closed · 51 total merges · 48 independently approved**.
- Local merged branches pruned. Working tree clean; on merged main the work module (125 tests), iteration persist (4), and run-cancel registry (4) are green.

## PR #227 evidence record (authoritative)

Reviewed head `d056a93` (full: `d056a93efac5d8afd6304b7926d72f1c0eb8a098`). Exact evidence digests, single passes:

| Suite | Result | SHA-256 |
|---|---|---|
| core | 11/11 | `d57a2a2038318680e5fe85e5058852f77cd55209adaa9f6f40aa733c70e1815d` |
| platform | 8/8 | `5736a6533f48a267040c365a17a38d58ad4433a9ba5b389edac454f5d928abe0` |
| smoke | 9/9 | `c2bdb40353ffee839698a9efe453cc1eaa2af5dde0690507e6629e0932dfde96` |

Verifier invocation and result (re-run against the retained artifacts on 2026-09-24; PASS, exit 0):

```
python scripts/local_ci.py verify --commit d056a93efac5d8afd6304b7926d72f1c0eb8a098 \
  .local-ci\evidence\d056a93efac5d8afd6304b7926d72f1c0eb8a098\windows-core.json \
  .local-ci\evidence\d056a93efac5d8afd6304b7926d72f1c0eb8a098\windows-platform.json \
  .local-ci\evidence\d056a93efac5d8afd6304b7926d72f1c0eb8a098\windows-smoke.json
PASS: 3 evidence files verify for d056a93efac5d8afd6304b7926d72f1c0eb8a098
```

Retained artifacts: `.local-ci/evidence/d056a93efac5d8afd6304b7926d72f1c0eb8a098/windows-{core,platform,smoke}.json`.
Authoritative remote record: evidence https://github.com/prometheosai/prometheos-lite/pull/227#issuecomment-5837260684, final approval https://github.com/prometheosai/prometheos-lite/pull/227#issuecomment-5838413241.

### Post-squash spot-checks on merged main (`c16d820`) — distinct from the PR-head evidence

- `git diff --stat d056a93 c16d820` → empty (tree equality; the reviewed-head evidence covers the merged content). The three-suite evidence was **not** regenerated at the squash commit.
- `cargo test --lib work::` → 125 passed; `iteration_persist` → 4 passed; `run_cancellation` → 4 passed.
- fmt/clippy proven clean by the `d056a93` core evidence; no standalone re-run on `c16d820`.

## #222 — CLOSED (completed)

Cooperative in-flight cancellation for WorkContext runs, landed through three review rounds:

1. **Cooperative checkpoints, never abrupt kills**: the tool-node child process (`tokio::process::Command` without `kill_on_drop`) makes arbitrary future-drop unsafe — the in-flight iteration is never severed. Cancellation is observed at the post-flow pre-persist checkpoint (no owned resources; nothing written yet).
2. **Race repairs (binding review 1)**: typed `CancelledRefusal` — only the pre-write entry refusal converts to graceful cancellation; flow failures surface before any cancellation observation. A flip that beats the loop's entry converts to a graceful evidenced stop only when this run's token fired.
3. **Atomic iteration persistence (binding review 2)**: `db::repository::iteration_persist` — ONE cancellation-conditional transaction per iteration; the guarded full-row UPDATE (`update_work_context_on_conn`, `WHERE status <> Cancelled`) is the first statement; all-or-nothing across context row, artifact rows, events, and the performance record; `execution_interrupted` is mandatory fail-closed evidence with `checkpoint_ref: null`.
4. Registry: `RunCancelRegistry` in `AppState` (RAII guard, fire-all, identity-safe). Cross-process honesty: same-process wake at the next checkpoint; cross-process cancels observed at the same checkpoints via the durable status — graceful polling, never an immediate mid-step wake.

Host flake disclosure: the documented Norton-AV-vs-`.git/objects` race (rotating git-fixture victims, each green in isolation) blocked several suite attempts across the rounds; all published evidence is single-pass green at the exact heads.

### #228 repair: cross-process cancel before loop-limit/completion exits

A durable cancel issued from another process (no token fire) could land between iterations while the run loop's in-memory snapshot was stale; the loop then exited through the limits/completion path whose writes hit the last-guard, surfacing a bare error with no evidence. Repair (`src/work/orchestrator.rs`): every loop top now takes a fresh cancellation observation (`graceful_if_cancelled`) BEFORE the limit/completion/blocked exit checks — a durable Cancelled converts to the same graceful evidenced stop as the sentinel path. Deterministic regression: `cross_process_cancel_observed_before_limit_exit` — flip (no token fire) while the loop is parked at the next loop top after a committed iteration; asserts graceful stop, mandatory evidence (`iterations: 1`), and that the limit exit never ran (no `context_blocked` event).

### #228 repair round 2 (binding review): typed refusal + atomic completion persistence

Round 1's exit-failure converter could mask ARBITRARY completion errors as graceful cancellation whenever the row happened to be Cancelled. Round 2 removes that blanket converter entirely:

- **Typed refusal only.** `complete_context` refuses a durably cancelled context at its entry read — BEFORE any write — with the typed `CancelledRefusal`; a cancel that wins at the completion transaction's guard also returns the typed refusal. The loop exits convert EXACTLY that type into the graceful evidenced stop (`graceful_cancelled_result`); every other error — including genuine persistence failures — propagates unmasked. The limit/runtime exits take a typed pre-write refusal (`exit_cancellation_check`) in the read-to-write window; their write errors propagate unmasked.
- **Atomic completion persistence.** `persist_completion_conn` (`db::repository::iteration_persist`): the Completion trigger's every durable effect — the full-row context update (terminal status, evaluation result, harness metadata), the evolved playbook row (`update_playbook_on_conn`, extracted as single source), and the `status_changed` event — commits in ONE cancellation-conditional transaction with the guarded UPDATE first. All-or-nothing: a mid-transaction failure rolls back the terminal status AND the playbook evolution. `EvolutionEngine::compute_evolved_playbook` is the new pure half (read + mutate, no write); `evolve_playbook` delegates.

Deterministic regressions: `complete_context_refuses_cancelled_with_typed_error_and_no_writes` (typed refusal, zero writes, no evidence from the refusal itself); `completion_persistence_failure_rolls_back_all_effects_unmasked` (late injected failure — the status_changed insert — rolls back the terminal status AND the evolved playbook, and the error is NOT the typed refusal); `completion_commits_status_playbook_and_event_atomically` (positive control). All 15 orchestrator tests, 129 work-module tests, 4 iteration-persist tests, and the 7 orchestrator e2e tests (including the evolution-trigger path through the atomic completion) pass.

## Merged slice history for #132 (E6/I03)

| Slice | PR | Commit |
|---|---|---|
| A: read-model rebuild tests | #213 | merged earlier |
| B: cursorable durable event stream | #214 | `311f7c8` |
| C: governed cancellation | #220 | `bd8a8a6` |
| Prerequisite: checkpoint registry (#221, closed) | #223 | `09836f1` |
| Decide endpoint | #224 | `c23597e` |
| Repository-native verification | #225 | `68d54b6` |
| Handoff authority record | #226 | `d2caf61` |
| Cooperative in-flight cancellation (#222, closed) | #227 | `c16d820` |

## Open follow-ups (operator-approved order)

1. **#132 remains open** — next up: remaining acceptance items:
   - Versioned, fail-closed `WorkEvent` mapping to SOMA++ SPEC 006.
   - Multi-client observation semantics (multiple authorized clients observing the same run without becoming state authorities).
   - Capability/compatibility projections mapped to SOMA #83.
2. **#217** — Dependabot `postcss-selector-parser` bump: untouched; requires explicit operator approval per AGENTS.md.

## Verification baseline

- fmt and clippy `-D warnings`: clean at reviewed heads per core evidence (covers merged main by tree equality at each merge).
- `cargo test --lib`: 1036 total (1 ignored) at `d056a93`.
- Any new PR: run the three local suites at the exact head, record digests, and request an independent fresh-context review before merge authorization.
- Host note: builds/temp routed to D: (`CARGO_TARGET_DIR`/`TMP`/`TEMP`/`TMPDIR`); the C: volume runs near capacity and the Norton AV intermittently races git-object writes under parallel test load (documented flake; see handoff notes since #214).
