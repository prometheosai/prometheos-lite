# Handoff

_Last updated: September 30, 2026, after PR #233 (#232 residual provenance) merged as `c7feab8`, and PR #234 (e4-x07 projections) merged as `84d44e4`._

## Authority state

- `main` = `c7feab8` — includes PR #233 (Slice 1A provenance hardening, closes #232) at reviewed head `9231316`, and PR #234 (e4-x07 projections slice 1, governance compiler + execution graph + human projection + golden conformance).
- Tally: **29 issues closed · 54 total merges · 51 independently approved**.
- Branches pruned; working tree clean. On merged main: 36 provenance enforcement tests, 6 envelope unit tests, 135 work-module tests, 1052 lib tests all green.

## PR #233 evidence record (authoritative)

Reviewed head `9231316` (full: `92313160b4bff757191c441b9e02b574acb6ab4a`). Exact evidence digests, single passes:

| Suite | Result | SHA-256 |
|---|---|---|
| core | 11/11 | `c988a130cc0021472ad98cd81ebde344866df4cd35f92fbbe67e1b8c5b493a64` |
| platform | 8/8 | `047b687f52ab4c91b0628043eebda62e434a6772d55f9d3aa84457b6949d42f8` |
| smoke | 9/9 | `5f61cec8671b94a13a04901ac49818d842b36bf50c49caf9ba1b14c704c5d98a` |

## Provenance enforcement contract (authoritative after #232)

**Write boundary** (`record_event_conn`): SOMA canonical renderer (byte fixpoint: serialize → parse → re-serialize = identical bytes); semantic invariants (never-widen across execution class + autonomy + approval policy); source digest over the complete canonical source event (id, context, type, data, created_at, provenance_json).

**Database trigger** (`provenance-trigger-v3`): `json_valid` + `json_extract` typed-path checks (schemaVersion, producer identity, correlation ID, request ID); GLOB hex digest check; flat/envelope column equality via COALESCE; principal-null semantics. Version marker is the sole upgrade criterion. `PRAGMA schema_version` (SQLite's documented DDL counter) proves the trigger is untouched on reopen.

**Read verification**: source digest re-verified; flat columns revalidated against the parsed envelope; canonical byte equality; mixed legacy/provenance state refused; legacy rows surface as `LegacyUnverified`.

**Cancellation signal**: `cancel_context` returns the exact event ID → `fire_with_cancellation` → `CancellationToken.cancel_with` (payload before flag, documented SeqCst contract). Every orchestrator observation path prefers the token payload; the durable lookup is only the cross-process fallback. Regression: orchestrator produces correct evidence even when the durable lookup is deliberately broken.

**Repository binding** (`detect_repo_binding`, pub): `git rev-parse --git-dir` (linked worktrees, subdirectories). `Bound` / `Dirty { digest_policy: "soma-canonical-json-v1" }` / `Unbound` — fails closed. Corrupted-index test rejects `Ok(Unbound)` (silent lie about a detected repo).

**Git fixtures**: `git_cmd` helper asserts success + sets explicit author/committer identity.

## Open follow-ups (operator-approved order)

1. **#132 Slice 1B** — pure fail-closed SOMA projection. Implementation plan to be posted for review before code lands. Depends on trustworthy provenance records (now closed by #232/#233).
2. **#132 Slice 2** — portable observation endpoint.
3. **#132 Slice 3** — SPEC 007 compatibility.
4. **#217** — Dependabot bump: requires explicit operator approval.

## Verification baseline

- fmt/clippy: clean at `9231316` per core evidence; covers merged main.
- `cargo test --lib`: 1052 total (1 ignored); plus 36 enforcement + 6 envelope unit tests.
- Any new PR: run the three local suites at the exact head, record digests, request independent fresh-context review before merge authorization.
- Host note: builds/temp routed to D:; C: near capacity; Norton AV intermittently races git-object writes under parallel test load (documented flake since #214).
