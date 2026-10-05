# Handoff

_Last updated: October 4, 2026, after PR #237 (#132 Slice 2 observation endpoint) merged as `65dff3c`; PR #236 (Slice 1B) as `4348475`; PR #233/#235 as `7033ee8`/`c7feab8` lineage._

## Authority state

- `main` = `65dff3c` — the SQUASH COMMIT that merged PR #237 (#132 Slice 2). The REVIEWED HEAD was `dc6ac7f2cd3ff465b1fdc3e7ece3d34be43f9af8` (verified immediately before the merge by the immutable expected-head guard; all evidence digests below were recorded AT that reviewed head, not at the squash commit).
- **Post-merge verification, actually run on merged main `65dff3c`** (not merely projected): 24 API observation tests, 23 Slice 1B conformance tests, 36 provenance enforcement tests, 1067 lib tests — all green on the checked-out merged main immediately after the merge; branch pruned; working tree clean.
- Tally: **29 issues closed · 58 total merges · 55 independently approved** (through PR #238, squash `b1120a6`).

## PR #237 evidence record (authoritative)

Reviewed head `dc6ac7f` (full: `dc6ac7f2cd3ff465b1fdc3e7ece3d34be43f9af8`). Exact single-pass evidence digests recorded AT that head: core 11/11 `ca7349ffe36c93e4325b815ed88ad197f61aa1a90e2b1b7ceaebffc669aa1d05`, platform 8/8 `d38b33259a440bf9add73896083036039ab7850bdba738f7ec83c3d67dbe5c15`, smoke 9/9 `981c40ab954542ab136cd12e42dbce008482fa4fd0a069c57b1123347ee53694` — 3-file verify PASS. Scope: +1,515/−27, accepted. #214 host-flake disclosure: markedly worse AV-vs-`.git/objects` interference this session — rotating git-fixture victims across attempts, each green in isolation; all published evidence is single-pass at the exact heads.

## SOMA projection + observation contract (authoritative after Slice 2)

- **Projections** (Slice 1B): verified-records-only mapping to SPEC 006 `WorkEvent`s; typed `RunKey` identities (equal strings never merge); per-run causally-closed batches with ancestor inclusion; raw stored-byte source digest (`data`/`createdAt` bind as the raw SQLite TEXT); the read gate binds semantics, the projection binds bytes.
- **Observation endpoints** (Slice 2, experimental router): `GET …/work-events` (reconnectable stream segments; `X-Next-Cursor` ALWAYS present, `X-More-Available` separate), `GET …/work-event-runs` (typed enumeration), `GET …/work-event-runs/:kind/:run_id` (run batches).
- **Transport validators**: bodies ARE canonical envelope bytes; stream ETags bind **body + continuation cursor + availability** (a metadata flip invalidates the cached representation — stale exhaustion cannot survive revalidation); stream 304s carry both paging headers; `If-None-Match` supports lists, weak tags, `*`; non-paged resources bind the body alone.
- **Error mapping**: 400 (unknown kind lists kinds) → 403/404 ownership → 500 gate failures (gate text verbatim) → 422 unprojectable records (`ApiError::Unprocessable`) → 404 unrecorded run keys (`ProjectionError::UnknownRun`) → 500 audit-gated batches. No partial artifacts on any path.

## Provenance enforcement contract (authoritative after #232/#233, refined by Slice 1B)

Write boundary: canonical-bytes fixpoint, never-widen invariants, complete source digest; `internal_system` records `Harness` (absent principal is the honest distinction; `System` is parse-only). Trigger `provenance-trigger-v3` with `PRAGMA schema_version` untouched-proof. Read verification: source digest + column drift + mixed-state refusals; legacy → `LegacyUnverified`. Cancellation: exact-event-id → token payload (SeqCst, payload-before-flag); durable lookup is cross-process fallback only. `detect_repo_binding`: Bound/Dirty/Unbound, fails closed.

## Open follow-ups (operator-approved order)

1. **#132 Slice 3** — SPEC 007 capability negotiation (`CompatibilityDecision` read/control projections; capability metadata is never authority). Plan review before implementation; plan to be posted to #132.
2. **#217** — Dependabot bump: requires explicit operator approval.
3. **Change records** for Slice 2 posted at `specs/loop-engineering/changes/2026-10-04-e6i03-slice2-observation-endpoint.md` (this PR).

## Verification baseline

- fmt/clippy: clean at `dc6ac7f` per core evidence; covers merged main `65dff3c`.
- `cargo test --lib`: 1067 total (1 ignored); plus 24 API observation + 23 Slice 1B conformance + 36 enforcement.
- Any new PR: three local suites at the exact head, digests recorded, independent fresh-context review before merge authorization; immutable expected-head guards at merge time. No merge without a fresh local-evidence review cycle.
- Host note: builds/temp routed to D:; C: near capacity; the #214 Norton-AV-vs-`.git/objects` flake rotates victims under parallel load (severe this session — cooldowns and isolated victim reruns are the documented protocol).
