# Change: E6/I03 Slice 1B — pure fail-closed SOMA WorkEvent projection over the verified journal

**Issue:** #132 (Slice 1B; approved Revision 1 plan `5970922176`, authorization `5971868162`, merge-authorization review `5982…` on PR #236)
**Depends on:** #232/#233 (Slice 1A provenance hardening, merged `c7feab8`), vendored `soma/v1.1` bundle (pinned, read-only), PR #234 projection envelope (`projection.v1`)
**Builds on:** `src/work/provenance.rs` (Slice 1A envelope), `src/db/repository/work_context_events.rs` (the verified read gate), `src/workflow/soma/event.rs` (SPEC 006 models + audit), `src/workflow/projection/envelope.rs` (`VersionedProjectionEnvelope`).

## Objective

A deterministic, read-only, library-only projection from durable `work_context_events` records (`ProvenanceState::Verified` only) to the published SOMA SPEC 006 `WorkEvent`/`WorkEventBatch` contracts, wrapped in the versioned projection envelope. No endpoint, no transport, no client, no SPEC 007, no dependency changes — those are Slice 2/3 boundaries.

## Deliverables

1. **Vendored SPEC 006 fixture conformance** (`tests/soma_event_projection_conformance.rs`)
   - The previously-unconsumed v1.1 event fixtures (4 valid + 11 invalid `wev-*`) are wired in: the pinned set is asserted exactly, each fixture's **canonical content digest** equals the manifest sha256 (probe finding: the manifest digests are `try_canonical_digest` renders, NOT raw file bytes — EOL-independent), valid fixtures audit clean, invalid fixtures audit with EXACTLY the manifest-pinned `expected_codes` (set equality, the upstream ground truth).
   - `.gitattributes`: `vendored/soma/**` and `tests/fixtures/soma-event-projection/**` are `-text` — `core.autocrlf` was re-smudging the byte-stable bundle on Windows checkouts.

2. **Correction 3 (write-time producer honesty)** (`src/work/provenance.rs`)
   - `JournalContext::internal_system` records `ProducerKind::Harness`: the Lite runtime process IS the harness; the meaningful distinction — the **absent principal** — is unchanged. `ProducerKind::System` remains in the vocabulary solely so already-stored envelopes keep parsing; nothing writes it; the projection fails closed on any stored `system` row. Single construction point verified; no test asserted `System`.

3. **The mapping core** (`src/work/soma_projection.rs`, `pub fn map_record`)
   - Journal event type → SOMA `eventType`: the 10 pinned rows (`context_created→context`, `artifact_added→evidence`, `decision_added`/`graph_decision→decision`, the six lifecycle-family events→`lifecycle`); anything else **fails closed** naming the type (the journal column is an open string).
   - Actor: `Human→human`, `Harness→harness`, `Tool→tool`, `Memory→memory` — exact; the stored legacy `system` producer fails closed. Implementation isomorphic; identity verbatim.
   - Authority: `executionClass` per the pinned table (`Deterministic→deterministic`, `ConstrainedModel→model-assisted`, `ScopedAgent→open-ended`, **`HumanDecision` fails closed** — no honest SOMA v1.1 bucket); `mutation: "none"` (journal event production exercises no SOMA mutation grant); every other profile field an honest omission.
   - `repoRevision`: `Bound`/`Dirty`→revision; `Unbound→""` (honest absence). `parents`: the real recorded causal parent, never adjacency. `idempotencyKey` = journal id; `sequence` = durable seq.
   - `semanticDigest` computed per SPEC 006 over the projected content (asserted ≠ the journal source digest). `payload`/`evidence`/`implementation`/`replay`/`conflict`: documented honest omissions.

4. **Stream pages — corrections 1 + 2** 
   - `WorkEventStreamPage { events }` is deliberately NOT a `WorkEventBatch` (a page's causal parents may live in earlier pages); wire confusion proven impossible in both directions (`deny_unknown_fields`).
   - `project_page(db, context_id, after_seq, limit)`: seq-cursor paging (clamped 1..=500, limit+1 lookahead for honest exhaustion), full per-event audits.
   - **Raw stored-byte binding (review P1):** `StoredColumns` carries `data_text`/`created_at_text` — the raw SQLite TEXT — and the envelope `sourceDigest` binds those raw strings plus the row's own `sourceDigest` and the derived identity columns. The projected `WorkEvent.timestamp` IS the stored representation (`Z` stays `Z`). The Slice 1A read gate is unchanged: the gate binds semantics, the projection binds bytes.

5. **Per-run batches — correction 4 + binding requirement 1**
   - `RunKey {kind, id}` — the TYPED recorded run identity; differently typed identities with equal strings never merge (asserted: a work run `shared-1` and a graph run `shared-1` are two runs).
   - `project_run_work_event_batch`: `runId` = the run's real recorded identity (never the work-context container); the batch contains the run's own events PLUS transitive causal ancestors that live outside the run (the cancel-request's `context_cancelled` parent of a work run's `execution_interrupted`), each verbatim; parent closure holds by construction and the full vendored `WorkEventBatch::audit` gates every emitted batch — a batch is never emitted dirty (cycles surface as `SOMA-EVT-0002`).
   - `recorded_run_keys`: the rebuild enumeration, distinct typed keys in first-appearance seq order; fails closed on legacy rows. Unknown run keys never invented as empty runs.
   - Byte-locked golden: the deterministic cancelled-work-run scenario (`cancelled-work-run.batch.canonical.json` + `.sha256` + `provenance.md`; relocked once for the P1 raw-byte change, documented).

6. **The durable fail-closed matrix** (`tests/soma_event_projection_conformance.rs`)
   - Legacy rows refused by all three entry points (never fabricated); tampered digests, drifted columns, and mixed states surface as `Journal` errors carrying the read gate's own detection text — **read-gate precedence proven** (binding requirement 2); a hand-stored `system` producer refused at projection time.
   - The two review-P1 regressions: `lexical_json_differences_change_the_projection_source_digest` (the gate passes before AND after a semantic-preserving re-lex — the projection digest changes) and `z_timestamp_preserves_stored_form_and_changes_digest`.

## Verification (exact head `9e14015`, single passes)

core 11/11 `938145a72000af7e9bcb41ed629c26aa3bc06fcc41f0c3550798b2ae8d4fff3e` · platform 8/8 `f216db5164b9733a42c8b35e29e958aa3595e5bb3f74f45927b05f66c1be2ae9` · smoke 9/9 `3b1a75ffe61d3f8ceb6501039945f82278ee05978bd035cdf99460fe8a3aece1` · 3-file verify PASS. Full single-pass `cargo test`: 1067 lib (1 ignored), 23 Slice 1B conformance, 36 provenance enforcement, 14 cancel, 13 cursor — zero failures. fmt/clippy `-D warnings` clean. Known #214 AV flakes retried per protocol (green in isolation). Squash-merged as `4348475`.

## Scope

+2,220/−1 across 9 files (~72% tests and fixtures) — explicitly approved by the merge-authorization review (do-not-split).

## Boundaries

Slice 2 (observation endpoint, reconnectable wire cursors) and Slice 3 (SPEC 007 capability negotiation) remain future work under #132. #217 untouched. No schema changes, no dependency changes, no frontend.
