# Handoff

_Last updated: October 4, 2026, after PR #236 (#132 Slice 1B SOMA WorkEvent projection) merged as `4348475`; PR #233 (#232) as `c7feab8`; PR #234 (e4-x07 projections) as `84d44e4`._

## Authority state

- `main` = `4348475` — PR #236 (Slice 1B, head `9e14015`) on top of PR #233 (Slice 1A provenance hardening, closes #232) and PR #234 (e4-x07 projections slice 1).
- Tally: **29 issues closed · 55 total merges · 52 independently approved**.
- Branches pruned; working tree clean. On merged main: 23 Slice 1B conformance tests, 13 mapping unit tests, 36 provenance enforcement tests, 150 work-module tests, 1067 lib tests all green.

## PR #236 evidence record (authoritative)

Reviewed head `9e14015` (full: `9e1401594b45e0a5cbf5e676b372094d8453058e`). Exact evidence digests, single passes:

| Suite | Result | SHA-256 |
|---|---|---|
| core | 11/11 | `938145a72000af7e9bcb41ed629c26aa3bc06fcc41f0c3550798b2ae8d4fff3e` |
| platform | 8/8 | `f216db5164b9733a42c8b35e29e958aa3595e5bb3f74f45927b05f66c1be2ae9` |
| smoke | 9/9 | `3b1a75ffe61d3f8ceb6501039945f82278ee05978bd035cdf99460fe8a3aece1` |

Scope exception: +2,220/−1 across 9 files (~72% tests/fixtures) — explicitly approved at merge authorization (do-not-split).

## Provenance enforcement contract (authoritative after #232/#233)

**Write boundary** (`record_event_conn`): SOMA canonical renderer (byte fixpoint); semantic invariants (never-widen across execution class + autonomy + approval policy); source digest over the complete canonical source event. **Correction 3 (from Slice 1B)**: `internal_system` records `Harness` (the process IS the harness; the absent principal is the honest distinction); `ProducerKind::System` is parse-only for stored envelopes.

**Database trigger** (`provenance-trigger-v3`): `json_valid` + `json_extract` typed-path checks; GLOB hex digest check; flat/envelope column equality via COALESCE; principal-null semantics. Version marker is the sole upgrade criterion. `PRAGMA schema_version` proves the trigger is untouched on reopen.

**Read verification**: source digest re-verified; flat columns revalidated against the parsed envelope; canonical byte equality; mixed state refused; legacy rows surface as `LegacyUnverified`. **The read gate binds semantics; the Slice 1B projection binds raw bytes** (review-P1 division of labor — complementary by design).

**Cancellation signal**: `cancel_context` returns the exact event ID → `fire_with_cancellation` → `CancellationToken.cancel_with` (payload before flag, documented SeqCst contract). Orchestrator observation paths prefer the token payload; the durable lookup is the cross-process fallback only.

**Repository binding** (`detect_repo_binding`, pub): `Bound` / `Dirty { digest_policy: "soma-canonical-json-v1" }` / `Unbound` — fails closed; corrupted-index test rejects `Ok(Unbound)`.

## SOMA WorkEvent projection contract (authoritative after Slice 1B, #132)

- **Consumes verified records only** (`ProvenanceState::Verified`); legacy/mixed/tampered states refuse — never fabricate. Read-gate precedence: the verified read always fails BEFORE mapping.
- **Mapping** (`src/work/soma_projection.rs`): 10 pinned journal→SPEC-006 event-type rows; unmapped types, `HumanDecision` execution class, and stored `system` producers all fail closed. `Unbound→""` repoRevision. `payload`/`evidence`/`implementation`/`replay`/`conflict`: documented honest omissions. SPEC 006 `semanticDigest` computed over projected content (≠ the journal source digest).
- **Raw-byte binding**: the envelope `sourceDigest` binds the complete stored record — raw `data` TEXT, raw `created_at` TEXT (`Z` vs `+00:00` bind differently), the row's own digest, the derived columns. Projected timestamps ARE the stored representation.
- **Typed run identity** (`RunKey {kind, id}`): work-run/graph-run/request kinds never merge equal strings. `project_run_work_event_batch` emits `runId` from the real recorded identity; cross-run causal ancestors included verbatim; full `WorkEventBatch::audit` gate (never emitted dirty).
- **Stream pages** (`WorkEventStreamPage`): seq-cursor segments, provably not batches (both wire directions); `recorded_run_keys` enumerates the rebuild.
- **Vendored SPEC 006 fixtures**: 15 pinned event fixtures consumed; canonical-content digest lock; exact-set diagnostics parity with the soma-native verifier. `vendored/soma/**` is `-text`.
- **Golden**: `cancelled-work-run.batch.canonical.json` byte-locked (+ `.sha256`, `provenance.md`); regeneration requires explicit review.

## Open follow-ups (operator-approved order)

1. **#132 Slice 2** — portable observation endpoint over the Slice 1B projections (reconnectable wire cursors; consumes `project_page`/`project_run_work_event_batch`). Plan review before implementation; implementation plan to be posted to #132.
2. **#132 Slice 3** — SPEC 007 capability negotiation (`CompatibilityDecision` read/control projections; capability is never authority).
3. **#217** — Dependabot bump: requires explicit operator approval.
4. **Change record** for Slice 1B posted at `specs/loop-engineering/changes/2026-10-04-e6i03-slice1b-soma-projection.md` (in this PR).

## Verification baseline

- fmt/clippy: clean at `9e14015` per core evidence; covers merged main `4348475`.
- `cargo test --lib`: 1067 total (1 ignored); plus 23 Slice 1B conformance + 13 mapping units + 36 enforcement.
- Any new PR: run the three local suites at the exact head, record digests, request independent fresh-context review before merge authorization. No subsequent merge without a fresh local-evidence review cycle.
- Host note: builds/temp routed to D:; C: near capacity; Norton AV intermittently races git-object writes under parallel test load (documented flake since #214 — rotating victims, green in isolation).
