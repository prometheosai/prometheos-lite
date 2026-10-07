# Change: E4/X07 Slice 2 — graph projection with nested composite disclosure boundaries

**Issue:** #164 (graph data model requirement rows; Slice 2 is their delivery node; approved plan comment + spec rev 3 at `docs/architecture/e4-x07-slice-2-graph-projection.md`)
**Depends on:** E4/X07 Slice 1 (PR #234, merge-commit `84d44e4`) — versioned envelope, canonical view, human plan, read/verify paths.
**Builds on:** `src/workflow/projection/*`, Slice-1 `audit`/`governance` recursion.

## Objective

Deliver the graph projection over canonical SOMA++ Workflows with **fail-closed composite disclosure boundaries**: nested composite bodies are disclosed scope-by-scope under explicit authority, every consumer of `.body` fails closed until the recursive audit lands, derived digests are domain-separated and acyclic bottom-up, and Human/graph payloads never leak withheld internals.

## Deliverables

1. **Contract expansion (Slice 2):** recursive `BodyItem`/`CompositeDefinition`, exact `oneOf` discrimination, union-key collision and malformed-nesting rejects at document load.
2. **Recursive nested-scope audit:** root-only/per-scope/document-wide check families recurse per scope with typed fault codes; every `.body` consumer fails closed until recursion lands (SOMA-CMP-0005 family).
3. **Graph projection (`projection/graph.rs`):** per-scope topological order; document-wide node/edge registry; withheld-by-default composite boundaries; validated/sorted/non-cascading `GraphDisclosurePolicy` (PROJ-0003); §7 boundary-port dataflow with §7.4 outcome labels; hidden node/edge counts under count authorization; domain-separated `childSubgraphDigest` (acyclic bottom-up) and `policyDigest`.
4. **Verification paths (spec §11):** `verify_graph_projection_bytes` (envelope parse → schema allow-list → structural validation → digest) and `verify_graph_against_source` (ordered steps 1–8; SOMA-CMP-0004 steps / PROJ-0002 step 8). Derived digests are shape-checked only by design.

## Verification (exact head `1af765d` — authoritative final pass; retries honestly disclosed)

conformance suite `tests/projection_conformance_tests.rs`: 64/64 green (35 Slice-1 + 29 Slice-2), including §12 items 1–29 (item 17 verify-half now asserts both verifier entry points on every vendored `valid/` fixture).

Final authoritative exact-head gate results: G1 fmt exit 0 (first attempt) · G2 `clippy --all-targets --all-features -D warnings` exit 0 (first attempt) · G3 `--all-features --test-threads 4` exit 0 **on attempt 4 at this head** — 105 groups, 2207 passed, 0 failed · G4 doc exit 0 (first attempt). Retries were not erased: earlier G3 attempts hit disk/host failures, and attempt 3 hit the pre-existing Windows `cpu_breach` kernel-limit-vs-monitor race (`validation.rs` kernel `JOB_OBJECT_LIMIT_JOB_TIME` exit `0xC000013A` racing the 100 ms monitor poll; main-scoped defect, same class as the prior disclosed `memory_limit_kills_runaway_process_windows` instance; isolated repro 0/10, lib-suite re-runs 2/2 green). local_ci core/platform/smoke PASS at that head (smoke attempt 1 died mid-`isolated install` when the wrapping shell's poll was aborted; attempt 2 PASS); 3-file evidence verification PASS (core `0b6eb3d…` · platform `1f06395c…` · smoke `889c39eb…`). Reviewed head preserved as ancestor of merged `d71cf0d`; post-merge tree equals `1af765d` byte-for-byte; merged-main post-merge test pass actually run (`--lib` 1082/1082, conformance 64/64).

## Scope

+5,028/−360 across 13 files (>60% tests/fixtures) on the Slice-2 PR; reconvergence merge added upstream's disjoint 22 files.

## Boundaries

No squash (history preserved behind the merge commit); no widened authority; withheld internals never appear in any payload; projection edits never mutate the AST; source AST remains the sole authority. The Slice-2 graph projection is a stable E4/X07 projection contract under this slice's spec rev 3; it is experimental only in the narrow sense that remaining E4/X07 projections (Slice 3: review report / evidence timeline) still await their own nodes. Compact/model-native projection remains gated and unpromoted — by SOMA #77 + Foundry #80 promotion requirements — independently of this slice.
