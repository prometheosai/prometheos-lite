# Change: E4/I04 — resumable graph execution + graph-level cancellation

**Issue:** #123 · **Depends on:** #151 ✅ #120 ✅ #121 ✅ #122 ✅.

## Scope

`src/workflow/graph_resume.rs` — resume/cancel orchestration over the durable state law:

1. **`resume_run`**: import checkpoint (inherits #120 fail-closed: digest chain, stale manifest revision, unsupported versions) THEN additionally rejects **stale repository revisions** (typed) and **unauthorized replay** — if any frontier node already holds a Completed attempt, resume requires explicit `ReplayAuthorization{reason, authorized_by}` (empty fields refuse). Authorization is carried in the resumed state's evidence trail.
2. **Interrupted-attempt reconciliation**: attempts with `completed_at == None` are closed as `Cancelled` (evidence preserved verbatim, nothing deleted); frontier membership is unchanged (the node remains eligible under normal caps).
3. **Whole-run cancellation**: `cancel_run` reconciles interrupted attempts, records `RunTerminationV1{reason, recorded_at}`, and clears the frontier. Subsequent routing refuses with typed `RunAlreadyTerminated`. Decisions/attempts/evidence stay intact — nothing reopened, nothing lost; checkpoints round-trip the termination.
4. **Implementation-change provenance**: `record_implementation_change` logs provider/model/harness swaps WITH mandatory 64-hex policy-evidence digest (compatibility + policy evidence per acceptance); missing evidence refuses.

Additive state fields (both `skip_serializing_if` empty/none → pre-#123 sealed checkpoints remain byte-identical; legacy fixture proves it).

Out of scope: cross-provider continuation proof (#155); UI cancellation surfaces (#131).

## Verification
cargo test --lib workflow::graph_resume (8 fixtures incl. a legacy-shaped checkpoint that omits the additive members) + workflow::graph_state + workflow::graph_gates; cargo fmt --check; cargo clippy --all-targets --all-features -- -D warnings; full lib green. No dependency changes.
