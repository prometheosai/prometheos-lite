# Handoff

_Last updated: September 26, 2026, after PR #229 (Slice 1A: durable provenance) merged as `245cf9b`, PR #228 (exit/completion race repairs) merged as `50ee17e`, and PR #227 (cooperative cancellation) merged as `c16d820`._

## Authority state

- `main` = `245cf9b` — squash-merge of PR #229 (Slice 1A of #132: durable provenance for every journal event) at reviewed head `677337b`; three P1 repair rounds incorporated from binding reviews.
- Tally: **28 issues closed · 53 total merges · 50 independently approved**.
- Branches pruned; working tree clean. On merged main: 15 provenance enforcement tests, 6 envelope unit tests, 135 work-module tests all green.

## PR #229 evidence record (authoritative)

Reviewed head `677337b` (full: `677337bf175d732b9e0ecf51164248d2165bc0a1`). Exact evidence digests, single passes:

| Suite | Result | SHA-256 |
|---|---|---|
| core | 11/11 | `1f5d1a24ecf8a7cf3fea330ffed8c4a00f5b3be18537252fe31492804535aa8c` |
| platform | 8/8 | `4370b4821f8ae03c60ebae0fb50ff3e2ff9df7c13b7170ea85f4e5290e3764b4` |
| smoke | 9/9 | `d838753cd6a2472f7dc6fca5150e7fc1f2aad6c6c41527dfd0f498cdcecb1151` |

Post-squash spot-checks: `git diff --stat 677337b 245cf9b` empty (tree equality); provenance enforcement 15/15, envelope unit tests 6/6 on merged main.

## Slice 1A design record (#132)

**The provenance envelope** (`src/work/provenance.rs`): closed typed structures (serde `deny_unknown_fields`); Producer (Human is valid), PrincipalRef (typed Absent — never fabricated), CausationRecord (real recorded parent, never seq-adjacent), AuthorityRecord (declared/effective stored separately, never widens), RepoBinding (Bound/Dirty{workspace_digest}/Unbound), RunIdentity (work-run, graph-run, request — preserved distinctly).

**Write boundary**: `record_event_conn` enforces the canonical-BYTES fixpoint (serialize→parse→re-serialize identical bytes), semantic invariants (never-widen execution class, non-empty identities), and derives the flat columns from the same envelope.

**Database enforcement**: `json_valid()` in the INSERT trigger (not just NULL/shape checks); entire journal rows append-only (UPDATE trigger). Idempotent column migration converges from partial states.

**Read verification**: source digest re-verified on read; flat columns revalidated against the parsed envelope; mixed legacy/provenance states refused; clean legacy rows surface as `LegacyUnverified`.

**Causation**: `cancel_context` returns the exact event ID; `observe_cancellation` reads context + event ID in one SQL transaction (atomic signal); `record_execution_interrupted` receives the carried ID.

**Producer/authority per path**: direct human mutations → Human producer, context-derived authority; run-loop/execution → Harness producer, context-derived authority; harness API → user principal + harness producer + `detect_repo_binding` (actual git state); CLI → operator principal + harness producer for run/continue paths.

## Open follow-ups (operator-approved order)

1. **#132 Slice 1B** — pure fail-closed SOMA projection (consumes only what 1A recorded; fails closed on legacy rows).
2. **#132 Slice 2** — portable observation endpoint.
3. **#132 Slice 3** — SPEC 007 compatibility (vendor the upstream v1.2 additions).
4. **#217** — Dependabot bump: requires explicit operator approval.

## Verification baseline

- fmt/clippy: clean at `677337b` per core evidence; covers merged main by tree equality.
- `cargo test --lib`: 1045 total (1 ignored); plus 15 enforcement + 6 envelope unit tests.
- Any new PR: run the three local suites at the exact head, record digests, request independent fresh-context review before merge authorization.
- Host note: builds/temp routed to D:; C: near capacity; Norton AV intermittently races git-object writes under parallel test load (documented flake since #214).
