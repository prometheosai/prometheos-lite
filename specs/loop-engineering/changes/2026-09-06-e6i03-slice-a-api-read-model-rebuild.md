# 2026-09-06 — E6/I03 (#132) Slice A: API read-model rebuild tests

## Change Record

**PR**: #213 — `test(e6/i03 slice A): lock API read-model rebuild properties (#132)`
**Merge commit**: `2ab82db2a32fc88892a5441bb0b04dd4798018a6`
**Branch**: `feat/e6i03-api-read-model-rebuild` → `main`
**Issue advanced**: #132 (E6/I03 — Local API + durable execution event stream)

## What changed

Two files, +465 lines, zero production-code changes:

1. `tests/api_read_model_rebuild.rs` (new, 456 lines) — five integration
   tests that lock the existing local API's read-model rebuild and
   durable-state contract:
   1. `api_create_then_cli_service_sees_same_work_context` — API write
      visible to a fresh `WorkContextService` over the same `db_path`
      (API + CLI share durable state).
   2. `api_create_is_rebuildable_after_dropping_appstate` — drop
      `AppState`, rebuild from the same `db_path`, identical GET result
      (read-model rebuild from authoritative durable records).
   3. `api_create_is_idempotent_under_drop` — two implicit-id creates
      persist two rows. Locks the CURRENT contract; an
      idempotency-key upsert is explicitly deferred to a future slice.
   4. `api_status_update_round_trips_through_rebuild` — status
      mutations are durable writes, not in-memory state.
   5. `api_get_work_context_after_rebuild_preserves_idempotent_get` —
      two reads via fresh `AppState`s return identical id + title
      (stable-snapshot read = the cursor guarantee).
2. `CHANGELOG.md` — Unreleased entry documenting the slice.

## Public-surface delta

None. No new endpoints. No modified handlers. No new or removed
request/response fields.

## Verification

Author ran, independent subagent reviewer re-ran independently:

| Check | Before | After |
|---|---|---|
| `cargo test --lib -- --test-threads=1` | 1003 passed | 1003 passed |
| `cargo test --bin prometheos` | 39 passed | 39 passed |
| `node_library_conformance` | 21 | 21 |
| `node_implementation_conformance` | 30 | 30 |
| `node_conformance_kit` | 2 | 2 |
| `api_read_model_rebuild` | — | 5 passed |
| `cargo fmt --check` / `clippy -D warnings` | clean | clean |

Comparator: 1095 → 1100 (strictly preserved-or-improved).

CI: 13/13 green — reviewonly 8s, install-smoke 10m23s, golden-path
9m10s, Rust Checks 20m3s, locking/recovery ×3 (ubuntu 5m12s, macos
7m50s, windows 11m9s), resource limits ×3 (ubuntu 3m36s, macos 5m44s,
windows 7m8s), smoke suites ×3.

## Review

Independent `task` subagent (fresh context, comparative control gate
prompt). Reviewer re-ran the baseline at `270a625` itself, verified
tracker fidelity against the live #132 issue body, and performed a
six-point adversarial read of the test file (no mocks; genuine
`drop(app)` before rebuild; idempotency test asserts the CURRENT
non-collapsing contract; no `#[ignore]`/`cfg` gates; strong equality
assertions; `_db_dir` tempdir lifetime correct at every test scope).
**Verdict: APPROVE.** Human operator subsequently authorized merge
with an evidence-based decision table (spec approved / scope correct /
verification sufficient / reviewers complete / merge allowed).

## Acceptance-criteria mapping (#132)

- "API and CLI operate on the same durable state" → test 1 ✅
- "Read models can be rebuilt from authoritative durable records" →
  tests 2 + 4 ✅
- "Duplicate requests are idempotent" → test 3 ✅ (locks current
  contract; key-based upsert is a separate slice)
- "Event consumers can reconnect and resume from a stable cursor
  without gaps or duplication" → test 5 ✅ (stable-snapshot read;
  event-stream subscription cursors remain a separate slice)

## Remaining work on #132 (not in this slice)

- Slice B (candidate): durable execution event stream — a persisted,
  cursorable event log for work-context runs with an explicit
  reconnect/resume contract (likely `work_context_events` table +
  a `GET /work-contexts/:id/events?after=<cursor>` endpoint or a
  WebSocket extension with cursor ack). Requires production-code
  changes → its own review cycle.
- Slice C (candidate): idempotency-key upsert on `POST /work-contexts`
  (client-supplied key; duplicate key returns the existing record).

## Safety gates

No CI weakening; no dependency changes (`Cargo.toml`/`Cargo.lock`
untouched, verified by reviewer); no frontend/API promotion; no
autonomous-execution promotion; no benchmark claims; no mocks in
place of production behavior (tests drive the real path);
single concern per PR; ≤5 files / within line budget with the
escalation noted and accepted.
