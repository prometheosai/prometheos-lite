# Proposed Foundry→Lite ingestion contract (unpublished; owner-gated)

**Status:** PROPOSED. **Not ratified.** Do not implement, import, or rely on this as a contract until the owners listed below decide it.

Owner of the gate: the owner, with:

- Foundry (prometheosai/foundry) — evaluator interface and verdict (`prometheosai/foundry#150`);
- PrometheOS Lite — subject adapter & export pinning (`prometheosai/prometheos-lite#156`);
- SOMA — the evidence-envelope / vocabulary boundary (`prometheosai/soma#146`, `prometheosai/soma#133`);
- This document is authored by SOMA Lite per commission, not ratified.

## 1. What this proposal covers

The smallest contract an external evaluator could consume from Lite is the **pinned failure-case bytes exported by `case_export_json`** — the existing `lite.evaluation.failure-case` v1.0.0 document — together with the replay matrix `lite.evaluation.failure-case-variant-matrix` v1.0.0 (both in-tree, both digest-pinned).

Proposed contract: **Lite exports `lite.evaluation.failure-case` v1.0.0 + `lite.evaluation.failure-case-variant-matrix` v1.0.0; Foundry reads, re-derives, and evaluates them; the scores are Foundry's.**

## 2. Required verifications at the ingestion boundary

Any ingestion contract must minimally require (all enforced by the in-tree types today, except where noted):

- Fail-closed parse of `lite.evaluation.failure-case` through `LiteFailureCase::parse_json` (schemaVersion/version/namespace verification scope pinned to the documented string).
- Re-derivation of `content_digest = sha256(canonical(case minus contentDigest))`; any mismatch refuses ingestion.
- `case_id` must equal `lite-failure-case-` + `failure_identity[..16]`; refusal otherwise.
- Exact schemaVersion/version/namespace equality (not semver compatibility).
- The `unsupported` `externalEvaluationFormat` marker on every fixture is structurally present; any presence of another status refuses ingestion.
- All digests lower-hex 64; `validatedAt` RFC 3339; `runner`/`runnerRevision` non-empty; fixture flags `injected_clock=true`, `fixture_only=true` for fixture inputs; live inputs are a separate (unpublished) ingestion profile.
- The variant matrix pins `case_id` + `failure_identity` + per-variant axis + expected/observed signatures + `matrix_digest`; re-derivation required.
- Byte stability: identical exports are byte-identical (verifiable by roundtrip; see `tests/failure_case_ingest_conformance.rs`).

## 3. Forbidden at the ingestion boundary

- Any provider invocation of an LLM or subprocess **inside** the ingestion boundary; the contract only consumes exported, pinned artifacts.
- Promotion of fixture/simulation evidence to live or certified evaluation; `simulation_only:true` and `verification_scope:"lite-internal-fixture-only"` are pinned refusals at ingestion.
- Rewriting of `unsupported` markers, or claiming `foundryAccepted: true`, `certified: true`, or any evaluator verdict field anywhere in the export.
- Any benchmark score, ranking, or comparative claim about models/providers in this document or anywhere in the export; the only correct consumables are byte-level digests and typed refusals.

## 4. The claim levels this contract does and does not carry

- Structural conformance of the bytes: Lite/Foundry both verify.
- Internal consistency (digests re-derive): Lite/Foundry both verify.
- Observed effects (what a process did): the pinned artifact only carries 343 observed measurements for the fixture harness; live-vs-fixture classification is enforced by `fixture_only:boolean` and `verification_scope`.
- Correctness/verdict of any model: **explicitly not carried; only Foundry may opine on that, and only after a real matched run exists.**

## 5. What must happen before any ingestion code is written

1. Owner ratifies or rejects the proposal as written.
2. If accepted: Foundry publishes the ingestion schema as a versioned, digest-pinned public artifact; Lite flips `externalEvaluationFormat.status` from `unsupported` to the newly published contract id in a new document version; the change is a new PR, with Foundry's review required.
3. If rejected: the answer is "no external ingestion is allowed"; nothing changes.

## 6. Non-claims

- This proposes **nothing** as live, certified, or production-gated.
- No Foundry or Lite contract is ratified by this document.
- No model, harness, or provider claim is made.
- This document exists because a dispatch asked for the minimal ingestion contract to be identified before any ingestion code is written.
