# Change: E4/I02 — deterministic sequential graph routing + bounded cycles

**Issue:** #121 · **Depends on:** #120 ✅ (#190), #117 ✅ (#184/#187/#188).

## Scope

`src/workflow/graph_exec.rs` — pure, durable-state-driven routing on top of the #120 transaction law:

- **Typed outcome→edge mapping**: a completed node's `OutcomeCategory` selects among outgoing edges — `sequence` edges are unconditionally eligible; `conditional` edges are eligible iff their `conditionLabel` equals the outcome label (snake_case). Eligible set sorted by target id.
- **Fail-closed routing**: zero eligible edges ⇒ `RouteError::MissingRoute`; multiple eligible ⇒ `AmbiguousRoute` (with candidate targets). Routing for a node with no journaled completion refuses (`RouteError::UnjournaledSource` via existing law).
- **Limits**: per-node cycle cap (visits of one node) and global attempt budget — exceeded ⇒ typed `CycleLimitExceeded` / `AttemptBudgetExhausted` BEFORE the decision is applied. Infinite loops terminate.
- **Determinism**: `route_after(state, from, outcome, recorded_at)` is a pure function of durable state + inputs — same state yields byte-identical decisions (recorded_at is an input).
- **Completion**: `run_complete(state)` true iff frontier is empty (all paths reached terminal exits).
- Reverse mapping `OutcomeKind → OutcomeCategory` bridges NodeResultV1 results into routing.

## Acceptance mapping

- three-node deterministic execution ✔ fixture
- same durable state ⇒ same next route ✔ (purity fixture)
- missing/ambiguous routes fail closed ✔ typed fixtures
- max attempts + cycles stop loops ✔ fixtures
- decisions persisted as evidence ✔ checkpoint round-trip contains decisions

Out of scope: parallel branches/joins/locks (#124); human gates/retry-edge semantics beyond caps (#122).

## Verification

cargo test --lib workflow::graph_exec; fmt/clippy/full lib green. No dependency changes.
