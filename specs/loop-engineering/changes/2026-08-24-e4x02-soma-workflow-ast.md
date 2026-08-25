# Change: canonical SOMA++ workflow AST (Rust types + validator + fixtures)

**Issue:** #159
**Owner:** PrometheOS Lite
**Depends on:** #116 ✓ #151 ✓ #152 ✓
**Normative source:** `prometheosai/soma` v1.1 bundle, commit `04858275ac61e4c8cd61b2a826242297f0915ff6` (SHA-pinned below).

## Ownership correction (per #159)

SOMA/SOMA++ own the normative semantics and canonical contract families. Lite does **not** define a competing AST. This change *implements* the published contracts in Lite's production compiler/runtime and proves conformance against the published fixtures.

`prometheosai/soma#77` compact/model-native syntax is **out of scope**. PortableWorkState remains a Lite runtime continuity artifact — this PR introduces no PortableWorkState changes.

## Deliverables

1. **Vendored spec snapshot** at `vendored/soma/v1.1/`:
   - 22 schema files (WorkflowDefinition, OperationDefinition, CompositeDefinition, PortDefinition, AuthorityProfile, GovernanceConstraint, TypedOutcome, SuspensionPoint, CheckpointEnvelope, EvidenceReference, ExecutionPlan, ExecutionProfile, HarnessAdapter, AdapterConformance, ResumeToken, ReviewDecision, ReviewRequest, OperationState, WorkEvent, WorkEventBatch, Diagnostic, plus AuthorityProfile);
   - `manifest.json` with per-artifact SHA-256;
   - `canonicalization.json` + `diagnostics.json` catalogue;
   - `fixtures/manifest.json` + `fixtures/{valid,invalid}/*.json` (63 total).

2. **`src/workflow/soma_ast.rs`** — typed Rust models for the primary workflows families (WorkflowDefinition, OperationDefinition, CompositeDefinition, PortDefinition, AuthorityProfile, GovernanceConstraint, TypedOutcome). Fail-closed serde (deny unknown fields), `parse_json` façade, canonical serialization that matches SOMA canonicalization v1.0.0 exactly (lexicographic key order, no whitespace, canonical-decimal numbers, null only where nullable).

3. **`src/workflow/soma_validate.rs`** — mechanical checks producing SOMA diagnostics (SOMA-AUTH-0001..0010, CMP-0001..0007, EXP-0001..0007, GOV-0001..0003, OUT-0001..0002). Unsupported versions and unresolvable references fail closed.

4. **`tests/soma_ast_conformance.rs`** — loads every fixture from `vendored/soma/v1.1/fixtures/`, runs the validator, asserts:
   - all 63 fixture files parse into typed models;
   - every `valid/` fixture yields zero errors;
   - every `invalid/` fixture yields at least its expected `SOMA-*` codes;
   - content digests re-canonicalize correctly when re-derived.

## Out of scope

- Governance compiler, IL lowering (later X-chain).
- PortableWorkState changes.
- Model-native compact syntax (deferred: `prometheosai/soma#77` + Foundry #80 evidence).
- Protocol version negotiation with Mnemosyne (spec #159 accepted, ties to #170 contract work).

## Verification plan

- `cargo test --lib workflow::soma_ast` — typed parses + digest round-trip.
- `cargo test --test soma_ast_conformance` — all 63 fixtures.
- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`.
- `vendored/soma/v1.1/manifest.json` self-consistency: every entry's `sha256` must recomputed-match its file's canonical bytes.
- No dependency changes (uses existing `serde_json`, `sha2`).

## Fixture → diagnostic mapping (discovered during implementation)

Invalid fixtures each flip exactly one semantic boundary; the validator must produce the named code (or a superset that includes it):

| Fixture kind | Expected code(s) | Specific violation pattern |
|---|---|---|
| wf-auth-0001 | SOMA-AUTH-0001 | operation uses capability `ghost-cap` not granted in authority.tools |
| wf-auth-0002 | SOMA-AUTH-0002 | composite exceeds imported authority |
| wf-auth-0003 | SOMA-AUTH-0003 | operation writes scope not in writableScopes (via `writable:finance` marker) |
| wf-auth-0004 | SOMA-AUTH-0004 | contentRestriction targets non-allowlisted provider |
| wf-auth-0005 | SOMA-AUTH-0005 | `uses` includes operation not in tool allowset |
| wf-auth-0006 | SOMA-AUTH-0006 | `secrets` names not declared |
| wf-auth-0007 | SOMA-AUTH-0007 | effect with `review:true` has no EvidenceAttachment / approval |
| wf-auth-0008 | SOMA-AUTH-0008 | irreversible effect w/o mutation authority |
| wf-auth-0010 | SOMA-AUTH-0010 | abstention behavior with no recovery |
| wf-cmp-0001 | SOMA-CMP-0001 | `schemaVersion != "1.1.0"` |
| wf-cmp-0002 | SOMA-CMP-0002 | `references: ["ghost-ref"]` unknown |
| wf-cmp-0003 | SOMA-CMP-0003 | missing required `authority` (top-level) |
| wf-cmp-0004 | SOMA-CMP-0004 | `contentDigest` does not match canonical |
| wf-cmp-0005 | SOMA-CMP-0005 | port type mismatch: body input `Customer` does not satisfy port type `Order` |
| wf-cmp-0006 | SOMA-CMP-0006 | unsupported type vocabulary (`not-a-Type`) |
| wf-cmp-0007 | SOMA-CMP-0007 | duplicate JSON key at top level |
| wf-exp-0001..0007 | SOMA-EXP-0001..0007 | cyclic / nondeterministic / unreachable / unfed-required / optional→required / leakage |
| wf-gov-0001..0003 | SOMA-GOV-0001..0003 | constraint unsatisfied / unsatisfiable / undecidable |
| wf-out-0001 | SOMA-OUT-0001 | failure outcome silently coerced to success |
| wf-out-0002 | SOMA-OUT-0002 | emitted outcome not in accept-set of next operation |

## Correct ownership posture
- Valid fixture set passes cleanly; invalid fixture set produces the pinned codes.
- Nothing in this change introduces Lite-only semantics masquerading as SOMA.
- Published hashes are verified byte-for-byte from vendored copies on every test run.

