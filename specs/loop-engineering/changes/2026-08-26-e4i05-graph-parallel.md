# Change: E4/I05 — parallel branches, joins, resource locks, graph evidence index

**Issue:** #124 · **Depends on:** #123 ✅.

## Scope

`src/workflow/graph_parallel.rs` — extends the #121 inevitable-forward-progress scheduler with:

1. **Resource declarations & concurrency waves**: additive `resources: Vec<String>` on `GraphNodeV1` (skip-if-empty → pre-#124 manifests byte-identical). `concurrency_waves(manifest, frontier)` deterministically partitions the frontier into waves where no two nodes in the same wave share a resource — conflicting writers (e.g. `repo:write`) can never co-schedule. Waves are sorted (resource grab order = start-of-wave alphabetical) so scheduling is replayable.
2. **Join policy**: additive `join: Option<JoinPolicyV1>` with `allRequired: bool` and `partialOutcomes` (explicit outcome labels allowed as branch ends). `evaluate_join` computes a sealed `JoinEvaluationV1` over predecessors (latest completed attempt per branch); partial branch failures are RECORDED (evidence preserved in the eval's branch map + original attempts untouched) and the eval's digest becomes the routing basis — partial failures route deterministically through the normal conditional-edge law.
3. **Routing law extension (mirrors #122)**: `record_route_decision` now accepts a join-evaluation digest as basis (additive `joinEvaluations` map on state, skip-if-empty, mirrors `gateDecisions`).
4. **Graph evidence index**: `build_evidence_index(state)` → `GraphEvidenceIndexV1` linking every node attempt (node, attempt, outcome, digest), route decision (from/to/basis), gate decision, join eval, and evidence ref — one canonical, digested view.
5. **Visualization**: `export_mermaid(manifest, state)` — deterministic Mermaid state diagram from durable state (frontier + terminal + gate nodes classed).

Out of scope: actual OS process/thread scheduling (waves are the evidence; the runner executes waves sequentially); lock *acquisition* at runtime (waves prove mutual exclusion structurally).

## Verification
cargo test --lib workflow::graph_parallel (10 fixtures incl. legacy-node byte-compat); fmt/clippy/full lib green. No dependency changes.
