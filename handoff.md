# Handoff

_Last updated: 2026-09-24, after PR #225 (repository-native verification) merged as `68d54b6`._

## Authority state

- `main` = `68d54b6` — repository-native verification replaces hosted GitHub Actions as the merge/release authority. Squash-merge of PR #225 at reviewed head `b7e321b` (evidence binds to the reviewed head, not the squash commit — see the evidence record below).
- Tally: **27 issues closed · 49 total merges · 46 independently approved**.
- Local branches pruned after each merge. Working tree clean; `main` == `origin/main`.

## PR #225 evidence record (authoritative)

Reviewed head `b7e321b` (full: `b7e321b4ef023b41c844e53de96674654a719666`). Exact evidence digests:

| Suite | Result | SHA-256 |
|---|---|---|
| core | 11/11 | `dcf1dccaf5ccdad6cc5cdfd87cdf2dc1a7838acc8a8b20b9740114968129af16` |
| platform | 8/8 | `b38831436a06ac274c644668bd10ba645b79e741fb4c15b37e7b8a4baaa2f4d8` |
| smoke | 9/9 | `fc0dc742f9217c30a760f81c7e6fe85d42289bd952516f9c61a66842920294b9` |

Exact verifier invocation and result (re-run against the retained artifacts on 2026-09-24; still PASS, exit 0):

```
python scripts/local_ci.py verify --commit b7e321b4ef023b41c844e53de96674654a719666 \
  .local-ci\evidence\b7e321b4ef023b41c844e53de96674654a719666\windows-core.json \
  .local-ci\evidence\b7e321b4ef023b41c844e53de96674654a719666\windows-platform.json \
  .local-ci\evidence\b7e321b4ef023b41c844e53de96674654a719666\windows-smoke.json
PASS: 3 evidence files verify for b7e321b4ef023b41c844e53de96674654a719666
```

Retained artifacts: `.local-ci/evidence/b7e321b4ef023b41c844e53de96674654a719666/windows-core.json` (1977 bytes), `windows-platform.json` (1545 bytes), `windows-smoke.json` (1863 bytes).

Authoritative remote record:
- Evidence comment: https://github.com/prometheosai/prometheos-lite/pull/225#issuecomment-5804398023
- Final approval: https://github.com/prometheosai/prometheos-lite/pull/225#issuecomment-5804777790

### Post-squash spot-checks on merged main (`68d54b6`) — distinct from the PR-head evidence above

- `git diff --stat b7e321b 68d54b6` → empty: the merged tree is identical to the reviewed head, so the `b7e321b` evidence covers the merged content exactly. The three-suite evidence was **not** regenerated at `68d54b6` (squash commit is only the merge vehicle; evidence binds to the reviewed head by design).
- `cargo test --lib` on `68d54b6` → 1015 passed, 0 failed, 1 ignored.
- `python scripts/test_local_ci_verify.py` → PASS (58 regressions).
- fmt/check/clippy are proven clean by the `b7e321b` core evidence (rustfmt, cargo check, clippy checks), which transfers to the merged tree via the tree-equality diff above; they were not re-run standalone on `68d54b6`.

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

## Open follow-ups (operator-approved order)

1. **#222**: cooperative in-flight execution interruption (`CancellationToken` through `WorkExecutionService`/`WorkOrchestrator`). Not started. **Next up** per operator.
2. **#132** remains open: remaining acceptance items (versioned fail-closed `WorkEvent` mapping to SOMA++ SPEC 006, multi-client observation semantics, SOMA #83 capability/compatibility projections). Resume after #222.

## Verification baseline

- fmt / check / clippy `-D warnings`: clean at reviewed head `b7e321b` (recorded in the core evidence above; covers merged `68d54b6` by tree equality).
- `cargo test --lib`: 1015/1015 (1 ignored).
- Any new PR: run the three local suites at the exact head, record digests, and request an independent fresh-context review before merge authorization.
