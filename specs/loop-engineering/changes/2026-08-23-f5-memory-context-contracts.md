# Change Spec: F5 — Memory/Retrieval/Context Contracts (#152)

- **PR series:** first PR opens against `main` from `feat/memory-context-interfaces`
- **Parent epic:** #104 · **Issue:** #152 · **Depends on:** #151, #113 (merged)
- **Status:** slice 1 of 4 in progress; each slice = one bounded PR through the
  independent gate loop.

## Scope

Lite-facing versioned interfaces for explicit memory operations and final
context delivery:

- `MemoryQuery` / retrieval request;
- `MemoryWrite`;
- provenance-rich `RetrievalResult`;
- final `ContextBundle`;
- `ProjectCheckpoint` mapping to `PortableWorkState`.

## Contract ownership (binding)

Verified upstream: `prometheosai/soma` PR #75 publishes SOMA++ v1 families:
AuthorityProfile, CheckpointEnvelope, EvidenceReference, TypedOutcome,
ResumeToken, Diagnostic, … (spec/soma/v1/schemas/*). There is NO published
SOMA++ family for memory/retrieval/context.

Therefore, per #152's contract rule:

| #152 concept | Ownership | Canonical alignment |
|---|---|---|
| `MemoryQuery`, `MemoryWrite` | **Lite-owned** `lite.memory.v1` | authorization scopes/budgets mirror SOMA `AuthorityProfile.readableScopes/writableScopes/budgets.token` |
| `RetrievalResult` candidate provenance | Lite-owned container embedding **canonical** SOMA v1 `EvidenceReference` field-set verbatim (id/eventDigest/artifactDigest/artifactKind/producedBy/producedAt) | canonical identifiers preserved |
| `ContextBundle` | **Lite-owned** `lite.memory.v1` | digests use SOMA canonicalization (sorted-keys compact JSON, sha256-64hex) |
| `ProjectCheckpoint` | Lite-owned mapping INTO existing `PortableWorkState` (which follows SOMA CheckpointEnvelope digest-chain conventions) | schemaVersion semver, fail closed |

No type is named "soma.*" unless byte-compatible with a published schema.
Upstream publication of a memory family remains future work outside this
issue; nothing here claims canonicality.

## Slices

1. **Contracts module** (`src/workflow/memory_contracts.rs`): versioned types,
   fail-closed version gate, canonical digest, EvidenceReference-mirroring
   provenance, ProjectCheckpoint↔PortableWorkState mapping, unit tests.
2. Retrieval pipeline: scope checks + token budgeting + provider-neutral port;
   stale-revision fail-closed; backend-unavailable semantics.
3. ContextBundle assembly: ordering, selected/omitted reasons, digest,
   operation policy; integration with context/builder.
4. Examples: local-only, Mnemosyne-backed stub port, cloud-allowed, conflict,
   stale-revision, deletion/expiry, backend-unavailable.

## Acceptance mapping (from #152)

Each acceptance checkbox maps to slices above; C-level tests are cited per
slice PR body. Stale/incompatible revisions and unsupported versions FAIL
CLOSED everywhere (no silent upgrade).

## Rules honored

No dependency changes; no CI changes; minimality budget per PR; every PR
returns to REVIEW_GATE via the independent gate agent before merge.
