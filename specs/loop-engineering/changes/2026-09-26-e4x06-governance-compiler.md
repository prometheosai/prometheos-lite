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
     plan); on success produces a sealed `ExecutionPlan`-shaped plan.
     **Published v1.1 shape only (review correction 1):** `schemaVersion
     1.1.0`, `planVersion 1.0.0`, positional
     `s{index:04}:{operationId}` step keys over the topological order,
     `workflowDigest` = canonical digest of the serialized workflow minus
     `contentDigest` — the audit's `SOMA-CMP-0004` rule — plus self-seal
     `canonicalization {version 1.0.0, sha256}`. The runtime structure
     Lite owns beside the plan is the separately compiled, Lite-owned
     `CompiledExecutionGraphV1` (`compile_execution_graph`); the v1.3
     plan/member shapes are a separate PR, not mixed in here.
     **Seal rule (review correction 2):** the seal digest input is the
     plan with ONLY `canonicalization.sha256` removed —
     `canonicalization.version` stays in the digest input (matches the
     `022142b` oracle byte-for-byte; see
     `tests/fixtures/soma-golden/provenance.md`).
   - Remediation table for the contract's key codes (AUTH-0001..0008,
     EXP-0007, CMP-0001/0003/0004/0007); every other code falls back to a
     catalogue pointer (no invented guidance).
   - **Catalogue-backed categories + JSON-pointer sources:** every emitted
     diagnostic's `category` is looked up in the pinned
     `vendored/soma/v1.1/diagnostics.json` by exact code (miss → honest
     `general` fail-safe), and every diagnostic carries
     `source.path` as an RFC 6901 pointer into the offending document
     (whole-document refusals use `""`) with `source.subject` naming the
     workflow/node the diagnostic is about.

3. **Digest binding** (`verify_reviewed_plan(plan_text,
   reviewed_identity)`)
   - Fail-closed ladder: parse (`SOMA-CMP-0003`) → version gate
     (`SOMA-CMP-0001`) → self-seal integrity (`SOMA-CMP-0004`) → identity
     equality with the reviewed identity (`SOMA-CMP-0004`, remediation:
     restore the reviewed plan or re-review).
   - `reviewed_identity` = the plan's `canonicalization.sha256` as captured
     at review time (commits to every plan member — tamper+reseal fails the
     identity step).

4. **Strict `ExecutionPlan` text boundary** (`validate_execution_plan_text`)
   - `verify_reviewed_plan` accepts ONLY plan-shaped documents
     (`schemaVersion`/`planVersion`/`workflowDigest`/`steps`/
     `canonicalization`, `deny_unknown_fields`, duplicate-key scan
     first): a workflow document or any other JSON is refused with
     `SOMA-CMP-0003` instead of being silently reinterpreted. Callers
     that hold a compiled plan serialize it to plan text before
     verification.

5. **`GovernancePermit` — structural bypass closure**
   (`src/workflow/governance_permit.rs`, review correction 3)
   - `GovernancePermit::issue(workflow_text, reviewed_identity)` runs the
     full chain — compile → verify-reviewed-plan (plan text, seal
     recompute, identity binding) → authority compile → execution-graph
     compile — and returns a permit whose fields are private (no
     builder, no `Default`); it can only come from `issue`.
   - `NodeRunner::Default` is removed; `NodeRunner::new(registry, permit)`
     is the only constructor. All four public effect entry points
     (`execute`, `execute_async`, `preflight_gates`, `seal_effect`) refuse
     a request whose manifest `nodeId` is absent from the permit's
     execution graph with `SOMA-CMP-0002` **before** capability
     resolution or any journal/effect (membership check scope: the
     permit governs exactly the workflow it was issued from).
   - The evaluation orchestrator's fast-loop runs under a permit bound to
     the embedded `FAST_LOOP_WORKFLOW_TEXT`'s real seal
     (`FAST_LOOP_REVIEWED_IDENTITY`): this governs the current
     orchestrator only — it is documented wiring, not a claim that every
     future runner instantiation must reuse that workflow.
   - Regression tests cover every entry point, ungoverned-node refusal,
     identity drift, and the no-permit-is-impossible property
     (`tests/node_runner_governance_permit.rs`).

6. **Local gate wiring** (`scripts/local_ci.py`)
   - New core-suite check `governance compiler` →
     `cargo test --test governance_compiler_conformance --quiet` in BOTH
     `SUITE_SPEC["core"]` and `rust_core()` (identical normalized command,
     required by the #225 evidence contract).

7. **Conformance suite** (`tests/governance_compiler_conformance.rs`)
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

## Resolved since first report

- **Category divergence:** `Diagnostic::category_for` now reads the pinned
  `vendored/soma/v1.1/diagnostics.json` by exact code (miss → honest
  `general`), so emitted categories are the catalogue's own (e.g.
  `authority_exceeded_composite` for AUTH-0002). `source.path` is an
  RFC 6901 pointer into the offending document; whole-document refusals
  use `""`.

## Cross-implementation oracle (review correction 1, pin)

- Oracle: **soma-core `022142b`**, whose v1.1
  `ExecutionPlan.schema.json` was verified byte-identical to the vendored
  schema; built offline (`cargo build -p soma-cli --offline`) on
  2026-09-27 with `rustc 1.98.0`, worktree `E:\Projects\soma-core-golden`.
- Goldens + digests are recorded in
  `tests/fixtures/soma-golden/provenance.md`; Lite's sealed plans are
  proven byte-identical to the oracle's canonical bytes and seal-equal by
  `tests/soma_golden_crossimpl.rs`. No fallback/oracle-impersonation was
  needed — the oracle built and ran offline.

## Out of scope (follow-ups)

- Provider execution, runtime routing redesign, UI, model training.
- Full v1.3 plan/member shapes (separate PR; this slice is published
  v1.1 shape + Lite-owned runtime structure only).
- Any change to the vendored SOMA bundle or its semantics.
- Dependency changes (`Cargo.toml` / `Cargo.lock` untouched).

## Verification plan

- `cargo fmt --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo test --all-targets --all-features --
  --test-threads 4`, `python scripts/test_local_ci_verify.py`.
- Repository-owned gates at the exact candidate commit: `python
  scripts/local_ci.py run --suite core`, `--suite platform`, `--suite
  smoke`, then `verify` for the recorded evidence (`.local-ci/evidence/
  <sha>/windows-<suite>.json`), attached to the PR.
- No GitHub Actions, hosted runners, paid services, or third-party status
  applications; no autonomous merge.
