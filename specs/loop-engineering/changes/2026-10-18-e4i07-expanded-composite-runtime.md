# Change: E4/I07 — governed sequential execution of expanded nested composites

**Issue:** #253 · **Parent epic:** #105 (exit criterion "Sequential nested workflow runs end to end")
**Depends on:** #120 ✅, #121 ✅, #123 ✅, #158 ✅ (closed complete), #160 ✅, #161 ✅, #162 ✅, #163 ✅
**Builds on:** `composite_expansion.rs` (#160 expansion), `governance_compiler.rs` (#163 sealed
plans), `governance.rs` / `governance_permit.rs` (#161/#163 permits), `execution_graph.rs`,
`graph_suspend.rs` (#162 + PR #252 P1-A/P1-B), `graph_exec.rs` (#121 routing),
`projection/*` (#164 projections).

## Objective

Bind the deterministic `CompositeWorkflow` expansion produced by #160 into
the existing governed sequential execution path, without introducing a
parallel state authority. A reviewed `GovernancePermit` now issues over
the sealed expanded plan; the flattened leaf operations execute
sequentially through the real `NodeRunner` gate pipeline into the durable
`GraphRunStateV1` law, carrying fully-qualified scope identity across
execution, routing, evidence, suspension, checkpoint, and resume. The
#160 compile-scope deferral is superseded at the runtime layer exactly as
#105 requires; the published v1.1 plan shape is untouched.

## Deliverables

1. **Runtime identity** (`composite_expansion.rs`) —
   `ExpansionStep::fully_qualified_id()`: the enclosing scope path joined
   by `/` plus the local operation id (root-scope leaves keep their bare
   id, so atomic-workflow identity is unchanged). Plus
   `leaf_operation()`: resolve the workflow-body operation a flattened
   step executes (fail-closed `PROJ-0001` for direct callers; unreachable
   through the audited path).

2. **Expanded execution graph** (`execution_graph.rs`) —
   `compile_execution_graph` now compiles composite bodies over the
   deterministic expansion: steps are the flattened leaves in scope
   order, `operation_id` is the fully-qualified runtime id, `scope_path`
   is a new additive member (`skip_serializing_if` empty — atomic
   workflows serialize byte-identically), `granted` is the leaf's own
   declared authority reduced against the reviewed ceiling
   (`reduce_leaf_authority`, shared by the atomic path), and
   `dependencies` lists same-scope operational producers by their
   fully-qualified ids. Boundary-crossing dataflow is carried by the
   deterministic flattened order and the typed crossings in the
   expansion plan, never by synthetic edges.

3. **Expanded authority graph** (`governance.rs`) — `compile_authority`
   covers the fully-qualified expanded leaves (the audit runs first, as
   before). Per-leaf `effective` keeps the atomic contract's semantics
   (the workflow ceiling); per-leaf narrowing is enforced by the
   published audit's `SOMA-AUTH-0002` ancestor-import rule, the
   expansion's chain checks, and the execution graph's reduced grants at
   request time. The snapshot digest therefore binds the expansion
   structure through the fully-qualified operation identities.

4. **Permit over the expanded plan** (`governance_permit.rs`) —
   `GovernancePermit::issue` no longer refuses composite bodies
   (`SOMA-CMP-0003` runtime-dispatch deferral removed): it verifies the
   sealed expanded plan against the reviewed identity, compiles the
   expanded authority and execution graphs from the same text, and
   carries the `ExpansionPlanV1` (new private field + public
   `expansion_plan()` accessor) bound to the same workflow digest —
   a stale or incoherent expansion refuses issuance (`SOMA-CMP-0004`).
   The hand-assembly compile-fail doctest now covers the new field.

5. **End-to-end conformance** (`tests/expanded_composite_runtime_conformance.rs`,
   10 tests, strict TDD): the two-level fixture
   (`tests/fixtures/composite/nested-two-level.json`, audit-clean, all
   leaves scoped) executes sequentially from entry to terminal through
   the real `GovernancePermit` + `NodeRunner` + `GraphRunStateV1` +
   `route_after` path; byte-identical expansion plan, route sequence,
   attempt references, suspension and token identities across two full
   drives; suspension/checkpoint/resume preserve the exact expanded
   operation (`flow/inner/leaf`) and the P1-B gates refuse the bare
   local id and foreign scope paths; negative controls for ungoverned
   nodes, per-leaf authority narrowing (chain-imported writable scope
   runs for the nested leaf, refuses for the root sibling),
   chain-authority widening at issuance (published audit
   `SOMA-AUTH-0002`), duplicate local ids at the audit gate
   (`SOMA-EXP-0003`), and stale-plan resume (`SOMA-RES-0001`); the
   public graph projection keeps composite internals unflattened by
   default and reveals them only under an authorized disclosure policy.

6. **In-crate defense** (`execution_graph.rs` tests) — reused local
   operation ids in different scopes stay distinct through
   fully-qualified runtime identity (unreachable through the public
   compiler; guards direct callers, mirroring the expansion's own
   defense-in-depth tests).

7. **Updated pin** (`projection_conformance_tests.rs`) —
   `nested_surfaces_fail_closed_matrix` →
   `nested_surfaces_bind_expanded_leaves_matrix`: the surface matrix now
   pins the #253 contract (authority graph, permit, and execution graph
   all bind the same fully-qualified leaf identities from one audited
   document). The canonical-projection and human-projection assertions
   are unchanged.

8. **Local gate wiring** (`scripts/local_ci.py`) — new check
   `expanded composite runtime` → `cargo test --test
   expanded_composite_runtime_conformance --quiet` in BOTH
   `SUITE_SPEC["core"]` and `rust_core()` (identical normalized
   commands per the #225 evidence contract).

## Scope decisions (stated honestly)

- The run manifest (`GraphManifestV1`) remains the orchestrator
  contract, exactly as for atomic runs: its nodes are the permit's
  fully-qualified step ids. No manifest-synthesis production surface was
  added.
- The runner stamps wall-clock times into `NodeResultV1`; the conformance
  driver therefore supplies the durable attempt references (per-step
  result digests and timestamps) as an orchestrator does, so every
  identity compared across two runs is a pure function of the reviewed
  workflow text. The runner's own journal chain is not asserted
  byte-identical (it is wall-clock-dependent by design).
- No API/CLI promotion, no dependency change, no parallel scheduler:
  dispatch is the existing sequential frontier law; #172 remains a
  separate P2 follow-on. #217 untouched. Vendored SOMA read-only.
- The graph projection's rendered-id model (`s{NNNN}:{id}` per scope)
  remains uniqueness-bound: a reused-local-id document fails closed
  there with `PROJ-0001` instead of rendering. That is a fail-closed
  projection boundary, not a runtime gap; the audit/permit/runtime path
  executes such documents end to end.

## Review round 2 (P1-1 / P1-2) — repairs at the new head

The independent review of the first candidate (`d2b9ab0…`) accepted the
end-to-end runtime but required two P1 repairs; both are implemented:

### P1-1 — scoped identity law

`WorkflowDefinition::audit` now enforces `SOMA-EXP-0003` per scope:
local ids are unique WITHIN their enclosing scope (duplicate siblings
fail closed, message unchanged — the vendored `wf-exp-0003` fixture and
the invalid-fixture pins hold), while the same local id in a DIFFERENT
scope is legal and remains distinct through its fully-qualified path
(the expansion's global FQ-path duplicate check stays as defense in
depth for direct callers). Document-wide id collection is unchanged for
`SOMA-CMP-0002` reference resolution and `SOMA-EXP-0001`
self-containment. The real positive control now exists end to end:
`reused_local_ids_across_scopes_issue_and_execute_distinctly` issues
one permit over a two-scope `A/x`/`B/x` workflow (boundary-reachable
dataflow `goal → A → v → B → result`) and executes, routes, and
journals both leaves as distinct identities with distinct attempt
references; negative controls pin duplicate siblings
(`SOMA-EXP-0003`) and ambiguous same-scope references
(`SOMA-CMP-0003`, expansion-layer binding check inside a composite).
The outdated first-round test that merely confirmed the refusal was
replaced; the projection fault matrix's cross-scope rename (legal
under the repaired law) was replaced by the still-invalid same-scope
duplicate form.

### P1-2 — per-leaf effective authority

`compile_authority` no longer records the workflow ceiling for
expanded leaves. New single-source reduction
`execution_graph::expanded_leaf_grants`: each flattened leaf paired
with its ACTUAL effective grant (the leaf's declared authority reduced
against the reviewed ceiling; ancestor-chain imports enforced by the
published audit `SOMA-AUTH-0002` plus the expansion chain checks).
Both the expanded execution graph (`granted`) and the compiled
authority graph (`effective`) consume THIS one reduction result, so
authority evidence can never disagree with the execution graph, and
`selection_within_authority` is bounded by the leaf's real grant. The
authority snapshot digest therefore binds every fully-qualified leaf
id AND its narrowed grant. Atomic workflows keep the accepted #161
contract (`effective` = ceiling; the request-time enforcement point is
the execution graph's `granted`) — unchanged and disclosed. New pins:
equality between every permit execution step's `granted` and the
corresponding authority-graph `effective` profile, and a negative
control where a selection inside the workflow ceiling but outside the
nested leaf grant is refused with `SOMA-PROF-0001` (a selection equal
to the leaf grant passes).

## Verification

- `cargo fmt --check`, clippy `--all-targets --all-features -- -D
  warnings`, doc tests (4 compile-fail proofs), full
  `cargo test --all-targets --all-features -- --test-threads 4`, focused
  suites: `expanded_composite_runtime_conformance` (12),
  `projection_conformance_tests` (64), `execution_graph` in-crate (4),
  plus the existing governance compiler (34) / emitted diagnostics (13)
  / soma capability (27) / composite expansion (16) + scope (8) / graph
  suspension (20) / permit suspension (18) / execution graph (8) /
  runtime-policy (1) suites.
- Repository-owned gates at the exact candidate commit:
  `python scripts/local_ci.py run --suite core|platform|smoke`, then
  `verify --commit <head> --require-platform windows` with all three
  evidence files; evidence attached to the PR.
- No GitHub Actions; no autonomous merge; open PR for fresh
  independent review.
