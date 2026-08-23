# Change Spec: F5 - Memory/Retrieval/Context Contracts (#152)

- **Issue:** #152 (parent epic #104; depends on #151/#113, merged)
- **Branch series:** `feat/memory-context-interfaces` (#174, MERGED),
  `feat/memory-retrieval-pipeline` (#176, this PR)
- **Status:** slice 1 MERGED via #174; slices 2-4 combined here under review.

## Scope

Lite-facing versioned interfaces for explicit memory operations and final
context delivery: `MemoryQuery`, `MemoryWrite`, provenance-rich
`RetrievalResult`, final `ContextBundle`, and `ProjectCheckpoint` mapping to
`PortableWorkState`.

## Contract ownership (binding)

Verified upstream: prometheosai/soma PR #75 publishes SOMA++ v1 families
(AuthorityProfile, CheckpointEnvelope, EvidenceReference, TypedOutcome,
ResumeToken, Diagnostic, ...). There is NO published SOMA++ family for
memory/retrieval/context.

Per #152's contract rule:

| Concept | Ownership | Canonical alignment |
|---|---|---|
| MemoryQuery / MemoryWrite | Lite-owned lite.memory.v1 | scopes/budgets mirror AuthorityProfile.readableScopes / writableScopes / budgets.token |
| RetrievalResult provenance | Lite container embedding canonical SOMA v1 EvidenceReference field-set verbatim | identifiers/digests preserved |
| ContextBundle | Lite-owned lite.memory.v1 | digests use SOMA-style canonicalization (sorted-key compact JSON, sha256 64-hex) |
| ProjectCheckpoint | Lite-owned mapping INTO PortableWorkState (CheckpointEnvelope-style semver + digest chain) | fail closed on unsupported majors |

No type is named "soma.*" unless byte-compatible with a published schema.
Upstream publication of a memory family stays future work; nothing here
claims canonicality.

## Slices delivered

1. **Contracts module** (MERGED via #174): versioned types, fail-closed major
   gate, canonical digest, EvidenceReference-mirroring provenance,
   ProjectCheckpoint mapping, unit tests.
2. **Retrieval pipeline** (this PR): `MemoryRetrievalPort` trait +
   BackendKind; `assemble_retrieval` enforcement:
   - empty readable_scopes => hard error (nothing authorized is not an empty success);
   - conflict rule: one candidate per memory_id, highest relevance wins
     (deterministic ordering: relevance desc, then memory_id); losers omitted
     as "conflicting duplicate (... superseded by ...)";
   - stale revision vs current => omitted "stale revision: ..." (never delivered);
   - token budget trimmed greedily in that order, overflow omitted
     "token budget exceeded"; estimate_tokens = chars/4 ceil (no tokenizer).
3. **ContextBundle assembly** (this PR): deterministic block order,
   per-block token estimates, `digest` computed over the canonical form of
   every field EXCEPT the digest itself (recomputable), fail-closed parse_json.
4. **Scenario suite** (`tests/memory_contracts_scenarios.rs`): local-only,
   mnemosyne-backed port interchangeability, cloud-allowed policy, conflict,
   stale-revision, unauthorized write (deletion/expiry surface), typed
   backend-unavailable, accounting consistency.

## Completion notes / disclosed semantics

- Digest exclusion: ContextBundle.digest intentionally excludes itself from
  its own canonical pre-image so receivers can re-compute and compare.
- Deletion/expiry today surfaces as scope-fail-closed writes plus port-level
  refusal; TTL-aware expiry remains future work (not silently claimed).
- Known follow-up: when two candidates share a memory_id AND one is stale,
  conflict-dedupe runs before staleness marking, so the loser may be reported
  as "conflicting duplicate" rather than "stale revision". The delivered set
  is unaffected (both are excluded); refining the reason taxonomy is queued
  as a follow-up issue after merge.

## Acceptance status (#152)

- [x] Lite interfaces explicit/project-owned where no SOMA family exists
- [x] No canonical claims without upstream identity
- [x] Scopes/authorization enforced at the boundary; provider-boundary separation explicit
- [x] Candidates carry source/revision + SOMA EvidenceReference provenance
- [x] Final bundles: token estimates, selected/omitted reasons, recomputable digest, operation policy
- [x] Stale/incompatible revisions fail closed
- [x] Canonical identifiers/versions/digests preserved through mappings
- [x] Examples cover local-only, mnemosyne-backed, cloud-allowed, conflict, stale-revision, deletion/expiry-write, backend-unavailable

## Verification (this PR)

fmt clean; clippy --all-targets --all-features -D warnings clean;
module tests 15/15; scenario suite 9/9; cargo test --lib 819 passed / 0 failed.
