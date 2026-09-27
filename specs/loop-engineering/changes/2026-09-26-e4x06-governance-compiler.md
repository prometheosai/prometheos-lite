# Change: E4/X06 Slice 1 — governance compiler (Lite → SOMA, compile-before-expose)

**Issue:** #163 (bounded contract: comment `5850679929`)
**Depends on:** #161 ✅ (merged #180), #159 ✅, vendored `soma/v1.1` bundle (pinned, read-only)
**Builds on:** `src/workflow/soma/*` (published models, audit, canonicalization), `src/workflow/policy.rs` (`EffectiveExecutionSnapshotV1`), `src/workflow/governance.rs` (`compile_authority` precedent).

## Objective

Add a versioned, deterministic Lite → SOMA governance compilation layer that
validates a workflow BEFORE any runnable plan or effect path exists, emits
enriched stable diagnostics (code, severity, source, explanation,
remediation), proves no-plan behavior for every invalid family published by
SOMA, and seals plans with a digest binding so execution can refuse any plan
other than the reviewed one.

## Deliverables

1. **Diagnostic enrichment** (`src/workflow/soma/mod.rs`)
   - `Diagnostic` gained optional `source` (`path` + optional `subject`) and
     `remediation` (`action` + required `summary`) members, both
     `skip_serializing_if`-guarded: previously serialized diagnostics stay
     byte-identical (legacy JSON round-trips without the new fields).
   - Builders: `Diagnostic::with_source` / `with_remediation`.

2. **Versioned mapping + compiler** (`src/workflow/governance_compiler.rs`)
   - `MAPPING_VERSION = "lite-to-soma-v1"`,
     `LiteToSomaMappingV1 { version, authority, liteEnforcedOnly }` — strict
     (camelCase, `deny_unknown_fields`).
   - `map_lite_authority(level, snapshot, execution_class)` — passes SOMA
     vocabulary through; `mutation = explicit` only for `Assist`/`Execute`;
     readable/writable scopes and escalation transfer from the effective
     snapshot; restrictions the SOMA vocabulary cannot express are disclosed
     on every mapping in `liteEnforcedOnly`
     (`deniedProviders`, `forbiddenPaths`, `maxAttempts`, `tokenBudget`) —
     never silently dropped. Lite's `tokenBudget` shape is intentionally NOT
     mapped into `AuthorityProfile.budgets` (different budget schema; that
     would invent rules).
   - `compile_workflow_text(text) -> Result<CompiledGovernancePlanV1,
     Vec<Diagnostic>>` — validates against the pinned v1.1 bundle
     (`validate_artifact_text`) BEFORE any plan value exists:
     input refusals → `SOMA-CMP-0003`; any audit diagnostic → `Err` (no
     plan); on success produces a sealed `ExecutionPlan`-shaped plan
     (`schemaVersion 1.1.0`, `planVersion 1.0.0`, positional `step-<i>`
     keys bound to body operation ids, `workflowDigest` = canonical digest
     of the serialized workflow minus `contentDigest` — the audit's
     `SOMA-CMP-0004` rule — plus self-seal `canonicalization {version 1.0.0,
     sha256}`).
   - Remediation table for the contract's key codes (AUTH-0001..0008,
     EXP-0007, CMP-0001/0003/0004/0007); every other code falls back to a
     catalogue pointer (no invented guidance).

3. **Digest binding** (`verify_reviewed_plan(plan_text,
   reviewed_identity)`)
   - Fail-closed ladder: parse (`SOMA-CMP-0003`) → version gate
     (`SOMA-CMP-0001`) → self-seal integrity (`SOMA-CMP-0004`) → identity
     equality with the reviewed identity (`SOMA-CMP-0004`, remediation:
     restore the reviewed plan or re-review).
   - `reviewed_identity` = the plan's `canonicalization.sha256` as captured
     at review time (commits to every plan member — tamper+reseal fails the
     identity step).

4. **Local gate wiring** (`scripts/local_ci.py`)
   - New core-suite check `governance compiler` →
     `cargo test --test governance_compiler_conformance --quiet` in BOTH
     `SUITE_SPEC["core"]` and `rust_core()` (identical normalized command,
     required by the #225 evidence contract).

5. **Conformance suite** (`tests/governance_compiler_conformance.rs`, 24 tests)
   - Positive: 3 valid fixtures compile to sealed plans (seal recompute,
     digest rule, step order); two-run byte-identical determinism.
   - Negative (no-plan): five contract families — authority (AUTH-0001/
     0003/0005/0006), provider (AUTH-0004), review (AUTH-0007/0010),
     recovery (AUTH-0008), composite (AUTH-0002/EXP-0007) — plus the full
     manifest-driven invalid corpus (≥25 fixtures, pinned codes) and
     malformed input.
   - Mutation: unknown-member and missing-member mutations of a valid
     workflow fail to compile; plan tamper is refused (stale seal) and
     tamper+reseal is still refused (identity).
   - Catalogue conformance: every emitted code exists in the pinned
     `diagnostics.json` with its published severity; every diagnostic
     carries explanation + source + remediation.
   - Mapping: authority-level matrix, scope/escalation transfer,
     `liteEnforcedOnly` disclosure, strict camelCase round-trip.

## Known gaps (reported, not fabricated)

- **Foundry #66 row 6** (model-output-triggered irreversibility): the
  published SOMA compiler exposes no independently verifiable contract for
  this case at the Lite layer; Lite's `LITE-GOV-0004` covers it at the
  governance-rule layer. Gap reported; no diagnostic invented.
- **Foundry #66 row 8** (parallel write overlap): deferred with the
  graph-run work; no SOMA code exists. Gap reported.
- **Category divergence:** `Diagnostic::category_for` maps AUTH codes to
  `authority_expansion`; the catalogue's own categories differ (e.g.
  `authority_exceeded_composite` for AUTH-0002). Pre-existing, out of
  slice scope; the conformance test asserts severity, not category.

## Out of scope (follow-ups)

- Wiring the compiled plan / `verify_reviewed_plan` into `node_runner`
  (the binding API exists and is tested; the runner call site lands with
  the execution-path slice).
- Provider execution, runtime routing redesign, UI, model training.
- Any change to the vendored SOMA bundle or its semantics.
- Dependency changes (`Cargo.toml` / `Cargo.lock` untouched).

## Verification plan

- `cargo fmt --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo test --all-targets --all-features --
  --test-threads 4`, `python scripts/test_local_ci_verify.py`.
- `python scripts/local_ci.py run --suite core` at the exact candidate
  commit (repository-owned gate, evidence attached to the PR).
- No GitHub Actions, hosted runners, paid services, or third-party status
  applications; no autonomous merge.
