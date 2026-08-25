# Change: E4/X04 — governed authority compilation + graph governance enforcement

**Issue:** #161
**Depends on:** #159 ✅ (merged #182), #118 ✅ (merged #180)
**Builds on:** `src/workflow/soma/*` (published contract models, audits, comparator), `src/workflow/policy.rs` (`lite.policy.v1`, `PolicyDecisionRecordV1`), memory contracts digest discipline.

## Objective

Compile per-operation authority out of a SOMA `WorkflowDefinition` into a runtime-checkable plan, and enforce the issue's seven required governance rules BEFORE any effect runs — with stable machine-readable diagnostics and durable decision records in evidence.

## Deliverables

1. **`src/workflow/governance.rs`**
   - `CompiledAuthorityGraph` — per-operation effective `AuthorityProfile` (workflow authority ∩ op grants; composite imports enforced), plus workflow-level constraints.
   - `compile_authority(workflow_value) -> Result<CompiledAuthorityGraph, Vec<Diagnostic>>` — fail-closed: runs the full published SOMA audit first; any diagnostic rejects compilation ("invalid authority graphs fail before execution").
   - Seven required rules as first-class, individually-addressable checks over the compiled graph:
     | Rule | Code |
     |---|---|
     | source application without human-approved validated proposal | `SOMA-AUTH-0007` family + `LITE-GOV-0001` |
     | restricted data routed to disallowed provider | `SOMA-AUTH-0004` (reused) |
     | review-only workflow mutating source | `LITE-GOV-0002` |
     | public-API change without contract validation + human review | `LITE-GOV-0003` (effect-name binding, configurable) |
     | destructive action without explicit mutation authority + rollback (escalation) path | `SOMA-AUTH-0008` + `LITE-GOV-0005` |
     | non-deterministic (model-assisted/open-ended) op emitting irreversible effects | `LITE-GOV-0004` |
     | composite authority exceeding imported authority | `SOMA-AUTH-0002` (reused) |
   - `enforce_before_effects(&CompiledAuthorityGraph, attempt) -> Result<(), Vec<Diagnostic>>` — the pre-execution gate.
   - `GovernanceDecisionRecordV1` (`lite.govdec.v1`) — durable evidence record: schema_version, workflow id/version, per-rule verdicts with codes, recorded_at; canonical digest + parse gate mirroring `lite.policy.v1` discipline.

2. **Ownership posture:** all semantics live under SOMA-owned codes where SOMA defines them; `LITE-GOV-*` codes mark Lite runtime rules that have no published SOMA equivalent (disclosed in the record itself — never masquerading as SOMA codes).

3. **Provider/harness selection cannot expand authority:** selection is expressed as an `AuthorityProfile` projection and checked with the merged `authority_widened` comparator — any widening is rejected before effects.

## Out of scope

- Parallel branches/joins/graph-run state (#124/#123 chain).
- Foundry fixtures (#136).
- Any change to vendored SOMA bundle or its semantics.

## Verification plan

- `cargo test --lib workflow::governance` — compile/reject paths, all seven rules fire on targeted negative fixtures and pass on a clean positive fixture, decision-record digest round-trip.
- fmt / clippy -D warnings / full lib suite green.
- No dependency changes.
