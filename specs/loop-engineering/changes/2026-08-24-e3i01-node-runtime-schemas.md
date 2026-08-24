# Change Spec: E3/I01 - Lite NodeManifest + NodeResult runtime schemas (#116)

- **Issue:** #116 (parent #104; depends on #151/#152 - MERGED)
- **Branch:** feat/node-runtime-schemas
- **Status:** single bounded PR (contracts + fixtures), same discipline as #152.

## Objective

Typed, versioned Lite runtime schemas for one node's execution contract:
identity/purpose, typed input/output, authority reference, evidence
references, terminal outcome, retryability, next-route hints, governed
memory operations (#152 types), and portable work-state references.

## Ownership (binding)

- `NodeManifestV1` / `NodeResultV1` are **Lite-owned** (`lite.node.v1`) -
  no published SOMA++ family exists for node runtime manifests/results.
  Upstream AST work is #159; nothing here claims canonicality.
- Alignment (by name/semantics, not redefinition):
  - authority => SOMA AuthorityProfile semantics via scope lists + budgets
    (same convention as lite.memory.v1);
  - terminal outcome => compatible with SOMA TypedOutcome categories:
    completed / failed / blocked / review_required / cancelled;
  - evidence references => reuse lite EvidenceReferenceV1 (canonical
    SOMA v1 EvidenceReference mirror) verbatim;
  - memory ops => explicit MemoryQuery/MemoryWrite through #152 types.
- Retry/route hints/runtime metadata = explicitly Lite-only fields.

## Deliverables

`src/workflow/node_contracts.rs`:

- `NodeManifestV1`: schema_version, node_id, purpose, inputs (typed kv via
  `NodeIo`), outputs (declared), authority alignment via
  `readable_scopes`/`writable_scopes`/`token_budget`, evidence_refs:
  Vec<EvidenceReferenceV1>, memory_reads: Vec<MemoryQuery>,
  memory_writes: Vec<MemoryWrite>, work_state_ref: Option<String>
  (versioned PortableWorkState pointer), retry: RetryPolicy{max_attempts,
  retryable_classes}, next_route_hints: Vec<String>.
- `OutcomeKind` enum: Completed/Failed/Blocked/ReviewRequired/Cancelled;
  reason string required for every non-Completed outcome.
- `NodeResultV1`: schema_version, node_id ref, outcome: OutcomeKind,
  reason, outputs, evidence_refs, memory_reads/writes_executed counts,
  work_state_ref, started_at/completed_at, failure_classification Option,
  result_digest (canonical, excluding itself).
- parse_json fail-closed major gate on BOTH; deny_unknown_fields;
  non-empty id validation; outcome-reason required for Failed/Blocked/
  Cancelled/ReviewRequired.
- Deterministic canonical digest helper for results (audit).
- Contract examples documenting the SOMA boundary (tests double as
  examples per acceptance).

## Acceptance mapping

- JSON serialization + validation: serde roundtrip tests + gate tests
- unknown fields / future versions fail closed (deny_unknown_fields +
  major gate tests)
- outcomes distinguish five states w/o contradicting SOMA semantics (enum +
  docs + tests)
- versioned references to PortableWorkState/ContextBundle/evidence (fields +
  tests)
- memory reads/writes as typed #152 operations (field types)
- no hidden provider/harness/conversation state (no such fields; doc note)
- Lite-only fields namespaced by doc + module boundary
- examples document SOMA boundary (test-named examples)

## Verification plan

fmt/clippy/-D warnings; cargo test --lib node_contracts; scenario-style
fixture test file mirroring #152 pattern if size allows within budget.
