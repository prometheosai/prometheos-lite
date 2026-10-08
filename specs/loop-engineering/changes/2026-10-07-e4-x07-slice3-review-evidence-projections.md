# Change: E4/X07 Slice 3 — review-report + evidence-timeline projections

**Issue:** #243 (E4/X07 Slice 3 — review-report + evidence-timeline projections under parent #164)
**Governing spec:** `docs/architecture/e4-x07-slice-3-review-evidence-projections.md` (approved rev 2 at PR #244; §7/Task G carry the post-#240 repair note)
**Builds on:** Slice 1 (`84d44e4`) envelope + canonical JSON + human plan; Slice 2 (`d71cf0d`) graph projection + disclosure model; docs-state #242 (`66d3362`).
**Base:** `main@80f201d791d21c58477b6d895ad9475fd329ee28` (reconverged from `4bc6c2c9` via merge; #240 SPEC 007 bundle included)

## Objective

Deliver deterministic, fail-closed review-report and evidence-timeline projections reusing the Slice-1 envelope and Slice-2 acyclic digest rules.

## Deliverables

1. **Review projection** (`src/workflow/projection/review.rs`) — `lite.review-report.v1` deterministic payload over `(WorkflowDefinition, ReviewFacts, policy)` with explicit gate disposition and summary availability markers.

2. **Timeline projection** (`src/workflow/projection/timeline.rs`) — `lite.evidence-timeline.v1` deterministic payload over `TimelineProjectionSource` (WorkEventBatch + optional ProvenanceState + optional scope/page meta), ordered strictly by `(sequence, semanticDigest)` tie-break.

3. **Verifiers** — structural byte-path verifiers that extract the envelope shape and shape-check derived digests (policy digest, structural digest) reuse the Slice-1 duplicate-key scan and canonical strict parse; against-source verifiers for both ways fresh-render from the SAME authoritative inputs and bytewise compare instead of re-inferring semantics.

4. **Disclosure** — policy-driven filtering is consistent with §6 of the spec; unassigned principals fields remain absent with explicit omission markers; disclosure digest is acyclic per §3.4.

5. **Diagnostic boundary (post-#240 repair)** — `SOMA-CMP-0011` (`duplicate_identity`) is pinned in the repository-owned Lite diagnostic-extension registry (`src/workflow/soma/diagnostic_extensions.rs`), NOT in an upstream-vendored catalogue: #240 vendored `v1.2` with a provenance lock and a v1.1→v1.2 additivity contract, which made the earlier "edit `vendored/soma/v1.1/diagnostics.json`" instruction obsolete. The earlier edit (including an accidental whole-file EOL rewrite) was reverted to the exact upstream bytes; resolution consults the upstream catalogue first, then the extension registry; cross-registry duplicate codes fail closed; unknown codes keep the fail-safe `general` fallback.

## Scope

Slice-3-specific files: `src/workflow/projection/mod.rs`, `src/workflow/projection/graph.rs` (one derive annotation), `src/workflow/projection/review.rs`, `src/workflow/projection/timeline.rs`, `src/workflow/soma/diagnostic_extensions.rs` (Lite-owned extension registry) + `src/workflow/soma/mod.rs` (`category_for` wiring), `tests/fixtures/slice3/**`, `tests/review_timeline_projection_conformance_tests.rs`, `tests/emitted_diagnostics_conformance.rs` (extension regressions), `CHANGELOG.md` slice-3 bullet. `vendored/soma/**` remains untouched (restored byte-identical to `main@80f201d`).

## Boundaries

Same exclusions as the plan: no #132 Slice 3, #217, compiled-plan work (#163), model native, graph runtime/spec redesign. No edits to `vendored/soma/**`.
