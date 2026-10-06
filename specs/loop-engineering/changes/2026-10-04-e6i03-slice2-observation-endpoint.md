# Change: E6/I03 Slice 2 — portable observation endpoint over the SOMA projections

**Issue:** #132 (Slice 2; approved plan `5982726737`, authorized in the plan-approval review; merge authorized at head `dc6ac7f` under an expected-head guard)
**Depends on:** #132 Slice 1B (PR #236, merged `4348475`) — `project_page`, `recorded_run_keys`, `project_run_work_event_batch`; PR #234's projection envelope.
**Builds on:** `src/api/work_contexts.rs` (the existing ownership-scoped router), `src/api/router.rs`.

## Objective

Three GET-only observation endpoints on the **experimental** API server exposing the Slice 1B projections as reconnectable, byte-deterministic HTTP resources. The journal stays the only source of truth; every response is a rebuildable, digest-locked projection artifact. No promotion, no WebSocket, no write surface, no dependency changes; #217 untouched.

## Deliverables

1. **Endpoints** (`src/api/work_contexts.rs`, `src/api/router.rs`)
   - `GET /work-contexts/:id/work-events?after=&limit=` — one reconnectable stream segment via `project_page`.
   - `GET /work-contexts/:id/work-event-runs` — the typed rebuild enumeration via `recorded_run_keys`.
   - `GET /work-contexts/:id/work-event-runs/:kind/:run_id` — the causally-closed `WorkEventBatch` for one recorded run; `:kind` (`work-run`/`graph-run`/`request`) is the wire spelling of the TYPED `RunKey` — equal strings under different kinds are distinct resources.

2. **Byte-deterministic transport** — the response body IS the envelope's `canonical_bytes()` (the golden-locked form); no wrapper objects, no transport fields inside the artifact. The continuation cursor (`X-Next-Cursor`, ALWAYS present — last returned seq, or the request's cursor for an empty page) and availability (`X-More-Available`) ride in headers.

3. **The complete-representation validator** (review rounds 1–2):
   - Stream ETags bind **body + paging metadata**: the canonical digest of `{body sha256, nextCursor, moreAvailable}` — a newly appended event beyond a full page leaves the body byte-identical but flips `more`, so the ETag flips and `If-None-Match` revalidation returns a full 200 with fresh headers; stale exhaustion cannot survive revalidation.
   - Stream 304s carry BOTH paging headers. Non-paged resources bind the body alone.
   - `If-None-Match` parses RFC 9110 list syntax: comma-separated tags, weak `W/` forms, `*`.

4. **Fail-closed error mapping** — 400 validation (unknown `:kind` lists allowed kinds) → 403/404 ownership → 500 for journal read-gate failures carrying the gate's own detection text → 422 (`ApiError::Unprocessable`, new) for records that cannot be honestly projected, naming the event → 404 for unrecorded run keys (the new typed `ProjectionError::UnknownRun` — honest absence, never an invented empty batch) → 500 for audit-gated batches (never emitted dirty). No error path returns a partial artifact.

## Verification (exact head `dc6ac7f`, single passes)

core 11/11 `ca7349ffe36c93e4325b815ed88ad197f61aa1a90e2b1b7ceaebffc669aa1d05` · platform 8/8 `d38b33259a440bf9add73896083036039ab7850bdba738f7ec83c3d67dbe5c15` · smoke 9/9 `981c40ab954542ab136cd12e42dbce008482fa4fd0a069c57b1123347ee53694` · 3-file verify PASS. 24 API tests + 23 Slice 1B conformance tests; full `cargo test` single-pass green; fmt/clippy `-D warnings` clean. The documented #214 AV-vs-git-objects flake was markedly worse this session — rotating victims across attempts, each rerun green in isolation, disclosed in the PR evidence. Squash-merged as `65dff3c` under an immutable expected-head guard (`dc6ac7f` verified immediately before merge).

## Scope

+1,515/−27 across 5 files (~75% tests) — accepted at merge authorization.

## Boundaries

Experimental router only — no alpha promotion, no WS/push, GET-only, no SPEC 007 (Slice 3 remains open under #132), no dependency changes, #217 untouched.
