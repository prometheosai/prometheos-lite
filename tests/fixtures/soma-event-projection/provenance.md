# Slice 1B projection goldens — provenance

`cancelled-work-run.batch.canonical.json` — the byte-locked canonical
output of `project_run_work_event_batch` (issue #132, Slice 1B) for the
deterministic cancelled-work-run scenario. `cancelled-work-run.batch.sha256`
is the SHA-256 of those bytes.

## Scenario (fully deterministic)

A fresh in-memory journal (`seq` 1..=3), fixed event ids, fixed RFC 3339
timestamps, fixed envelopes:

1. `ev-cancel` — `context_cancelled`, produced by the cancel REQUEST
   (`req-cancel`, a human acting directly: actor kind `human`).
2. `ev-run-start` — `status_changed`, produced by the work run `wr-1`
   (`req-run`, harness actor).
3. `ev-interrupted` — `execution_interrupted`, produced by the work run
   `wr-1`, its recorded causal parent being the EXACT `ev-cancel` event
   id (never a "latest" inference).

Projected run key: `{ kind: WorkRun, id: "wr-1" }`. The batch therefore
contains the work run's two own events PLUS the cross-run causal
ancestor (`ev-cancel`, verbatim — its own correlation and actor intact),
in durable seq order, with `runId: "wr-1"` (the real recorded identity —
never the work-context container). Parent closure holds by
construction; the full vendored `WorkEventBatch::audit` gate passes
(zero diagnostics).

## Lock policy

Regenerating requires an explicit, reviewed change to this file (the
mapping tables, the canonical renderer, or the vendored SOMA bundle all
feed these bytes). Byte stability is enforced by
`cancelled_work_run_batch_matches_the_locked_golden_bytes` in
`tests/soma_event_projection_conformance.rs`; the directory is
`-text` in `.gitattributes` (no EOL conversion on any platform).
