# Handoff

_Last updated: 2026-09-24, after PR #225 (repository-native verification) merged as `68d54b6`._

## Authority state

- `main` = `68d54b6` — repository-native verification replaces hosted GitHub Actions as the merge/release authority. Squash-merged at reviewed head `b7e321b` with exact-head evidence: core 11/11, platform 8/8, smoke 9/9.
- Tally: **27 issues closed · 49 total merges · 46 independently approved**.
- Local branches pruned after each merge. Working tree clean; 1015/1015 lib tests green on merged main; 58 verifier regressions green.

## Verification contract (now authoritative)

- `scripts/local_ci.py` — runner + verifier bound to the shared `SUITE_SPEC`:
  - exact normalized command arrays (identity tokens, `<root>` placeholder; any extra flag like `--no-run` rejected);
  - fail-closed schema/provenance (rustc/cargo prefix + non-empty python/architecture; clean tree; exact commit; ISO timestamp);
  - complete, exact, ordered per-suite check sets (core 11, platform 8, smoke 9, frontend 4);
  - durations finite, non-negative, non-boolean;
  - generation-time self-check refuses to write non-conforming evidence;
  - frontend verifiable; `--require-frontend` gates it when frontend paths change;
  - `scripts/test_local_ci_verify.py` — 58 regressions run inside core's own evidence chain;
  - `scripts/test_target_dir.sh` — 9 path-resolution regressions run as smoke's first check; shared logic in `scripts/lib/target_dir.sh`.
- Evidence files land under `.local-ci/evidence/<commit>/`, SHA-256-pinned, verified via `python scripts/local_ci.py verify --commit <sha> <core> <platform> <smoke>`.
- Host note: the evidence host's C: volume runs critically full; builds/temp are routed via `CARGO_TARGET_DIR`/`TMP`/`TEMP`/`TMPDIR` to the D: volume. The smoke scripts resolve the target dir accordingly (normalize-before-classify; explicit `.exe` candidates).

## Merged slice history for #132 (E6/I03)

| Slice | PR | Commit |
|---|---|---|
| A: read-model rebuild tests | #213 | merged earlier |
| B: cursorable durable event stream | #214 | `311f7c8` |
| C: governed cancellation | #220 | `bd8a8a6` |
| Prerequisite: checkpoint registry (#221, closed) | #223 | `09836f1` |
| Decide endpoint | #224 | `c23597e` |
| Repository-native verification | #225 | `68d54b6` |

## Open follow-ups

- **#132** remains open: remaining acceptance items (versioned fail-closed `WorkEvent` mapping to SOMA++ SPEC 006, multi-client observation semantics, SOMA #83 capability/compatibility projections).
- **#222**: cooperative in-flight execution interruption (`CancellationToken` through `WorkExecutionService`/`WorkOrchestrator`). Not started.

## Verification baseline

- fmt / check / clippy `-D warnings`: clean on merged main.
- `cargo test --lib`: 1015/1015.
- Any new PR: run the three local suites at the exact head, record digests, and request an independent fresh-context review before merge authorization.
