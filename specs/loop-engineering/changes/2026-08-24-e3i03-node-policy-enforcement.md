# Change Spec: E3/I03 - Lite node authority/policy enforcement (#118)

- **Issue:** #118 (parent #104; depends on #116 MERGED, #152 MERGED)
- **Branch:** feat/node-policy-enforcement
- **Status:** single bounded PR (effective-authority resolver + enforcement + evidence)

## Objective

Lite runtime policy mapping: resolve an immutable, versioned
**EffectiveExecutionSnapshot** per attempt from declared SOMA-aligned
authority (via lite.node.v1 manifests from #179) plus local restrictions,
then ENFORCE it before effects occur. Authority may only be preserved or
reduced - never widened - across fallback, retry, nested execution, or
harness replacement.

## Ownership (binding)

- SOMA owns authority/escalation/irreversibility/governance-predicate
  semantics (AuthorityProfile). Lite owns resolution/reduction/mapping/
  enforcement/durable recording.
- Until soma#80 publishes a canonical ExecutionProfile, the snapshot is
  explicitly **Lite-owned** (`lite.policy.v1`); after publication a versioned
  fail-closed mapping is required. No second authority taxonomy.

## Deliverables

`src/workflow/policy.rs`:

- `EffectiveExecutionSnapshotV1`: schema_version, snapshot_id, base
  authority fields (readable/writable scopes, token budget), resolved
  restrictions (denied providers/harnesses, forbidden paths), retry policy,
  max_attempts, escalation target label, recorded_at. Immutable (no mutators).
- `resolve_effective(manifest: &NodeManifestV1, local: &LocalRestrictions)
  -> Result<EffectiveExecutionSnapshotV1>`:
  monotone-decreasing intersection of manifest scopes with local scopes;
  union of denied lists; min of budgets; fail closed when intersection leaves
  NO readable scope while memory reads are declared.
- `enforce_before_effects(snapshot, requested) -> Result<()>`:
  - write to scope not in writable => typed PolicyViolation("scope");
  - provider in denied list => typed violation ("provider");
  - attempts exhausted => typed violation ("attempts").
- `PolicyViolation` typed error with kind + detail.
- Durable decision record: `PolicyDecisionRecordV1` (snapshot digest via
  canonical digest, requested effect, allow/deny + reason) with
  parse gate + digest recompute test.
- Tests: monotone reduction property (snapshot scopes always subset of both
  inputs), no-widening across resolve+enforce loop, each violation kind,
  digest stability, future-major rejection.

## Acceptance mapping (#118)

Monotone-decreasing derivation (property test); pre-effect enforcement of
tool/scope/provider/memory paths (typed violations); retry vs terminal split
(retryable_classes consumed by attempts check); attempt limits; escalation =
typed ReviewRequired-compatible outcome reference (lite.node.v1 OutcomeKind);
privacy = denied providers; no silent memory-scope widening (intersection);
durable decisions w/ immutable snapshots (digest records); Lite-only fields
explicitly owned; representation-neutrality satisfied by operating on typed
inputs only.

## Verification plan

fmt/clippy/-D warnings; cargo test --lib workflow::policy; property-ish
looped reduction tests (fixed seed vectors, no rng dependency).
