# E4/X07 Slice 1: Deterministic Projections — Design Spec

**Status**: Approved for implementation  
**Branch**: `feat/e4-x07-projections-slice-1`  
**Base**: `main@db98758` (post PR #230 merge)  
**Activates**: issue #164 comment `#5913111904` ("ACTIVATED — E4/X07 Slice 1")

---

## 1. Scope and Ownership

This slice delivers **PrometheOS Lite product projections** derived exclusively from the canonical SOMA++ `WorkflowDefinition` (schema 1.1). Projections **do not define, alter, or extend SOMA semantics**; they are read-only views over one source AST.

### In Scope (Slice 1)

| Projection | Payload type | Stability | Redacted? |
|------------|--------------|-----------|-----------|
| Versioned envelope (shared wrapper) | generic `V` | required | N/A |
| Canonical JSON view | `serde_json::Value` | byte-deterministic | **never** |
| Human-readable plan | `String` | grammar-stable (golden fixtures) | optional policy |
| Read/verify path | fail-closed byte checks | required | N/A |

### Explicit Non-Goals (Slice 1)

- Round-trip / lossless compact projection; provider-native structured encodings.
- Model-native encoding — gated behind SOMA #77 + Foundry #80 promotion.
- Graph-data projection and nested private-boundary disclosure (later slice).
- Compiled-plan changes (owned by #163/#164 later slices); review-report and evidence-timeline UI (later slices).
- Client editing, projection-to-AST mutation, or any write path that could widen authority or emit a runnable plan.
- **PR #233 is not absorbed**: this slice stays on its own branch from `main@db98758`; if #233 lands first, rebase and regenerate exact-head evidence, no file mixing.

---

## 2. Architecture

```
src/workflow/projection/
├── mod.rs          # Public API: project_canonical_json, project_human_plan,
│                   #   verify_* , VersionedProjectionEnvelope, PROJ diagnostics
├── envelope.rs     # VersionedProjectionEnvelope<V> (nested payload) + digest helpers
├── canonical.rs    # canonical JSON projection + byte verify path
├── human.rs        # human plan renderer (grammar below)
├── redaction.rs    # RedactionPolicy + human-path disclosure application
tests/
├── projection_conformance_tests.rs   # integration tests (RED→GREEN)
└── projection_golden/*.human.txt     # golden fixtures for human plan
```

- **Source of truth**: `crate::workflow::soma::contracts::WorkflowDefinition`.
- **No competing AST**: projections read the canonical AST; never mutate it; never accept an independently authored semantic object.
- Production modules stay free of `#[cfg(test)]`; conformance lives in `tests/`.

---

## 3. Input Gate (validated canonical AST only)

Both projection functions run a **fail-closed gate** before rendering:

1. `wf.audit(&super::supported_version())` must return no diagnostics (this covers `SOMA-CMP-0001` unsupported schema/workflow versions and `SOMA-CMP-0004` `contentDigest` verification).
2. `wf.schema_version` must equal the supported SOMA schema (`super::SUPPORTED_SCHEMA_VERSION`-compatible per `SemVer::is_compatible_with`).

Failure ⇒ `Err(diagnostics)` with the original audit codes; no partial projection. Half-parsed, non-canonical, or non-audited AST input is never projected.

---

## 4. Versioned Projection Envelope

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionedProjectionEnvelope<V> {
    /// Projection format version. Slice 1 accepts exactly "projection.v1".
    pub projection_version: String,
    /// SOMA schema version of the source workflow (e.g. "1.1.0").
    pub schema_version: String,
    /// Canonical digest of the SOURCE AST: serde_json::to_value(wf) with
    /// `contentDigest` removed, then soma::canonical::try_canonical_digest.
    pub source_digest: String,
    /// Canonical digest of the SERIALIZED PROJECTION ARTIFACT:
    ///  - canonical view: try_canonical_digest(payload_value)
    ///  - human plan:     sha256_hex(payload_text.as_bytes())
    pub projection_digest: String,
    /// Projection-specific payload — NESTED, never #[serde(flatten)].
    pub payload: V,
}
```

Two identity layers, both covered by tamper tests:

- `source_digest` — identity of the AST the projection came from (compile-time identity).
- `projection_digest` — identity of the payload bytes actually carried (artifact identity).

**Digest policy (pinned, shared by both digests):** SOMA canonicalization v1.0.0 / DecimalV2 via `soma::canonical::{try_canonical_bytes, try_canonical_digest, sha256_hex}` — lexicographic key order, no whitespace, canonical number lexemes, declared array order, fail-closed on magnitude > 1e10000 / precision > 400 digits / non-finite (`CanonicalError` ⇒ `SOMA-CMP-0004`).

**`projection_version` policy (fail closed):** allowed set is `{"projection.v1"}` only. Any other value, missing or unknown, is rejected — never silently passed (`SOMA-CMP-0001` family).

---

## 5. Canonical JSON Projection

```rust
pub fn project_canonical_json(
    wf: &WorkflowDefinition,
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>>
```

1. Run §3 gate.
2. `payload` = `serde_json::to_value(wf)?`.
3. `source_digest` = digest of `to_value(wf)` minus `contentDigest`.
4. `projection_digest` = `try_canonical_digest(&payload)`.
5. Envelope: `projection_version = "projection.v1"`, `schema_version = wf.schema_version`.

**Determinism contract:** identical AST (regardless of map insertion order — `AuthorityProfile.tools` is already a `BTreeMap`) ⇒ byte-identical canonical payload ⇒ identical both digests ⇒ identical envelope JSON under `serde_json::to_string`.

**Redaction:** never applied (see §7).

### Read/verify path (fail closed)

```rust
pub fn verify_canonical_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>>
```

Rejects, in order:
1. duplicate keys (`soma::canonical::find_duplicate_key`);
2. unknown fields / wrong shape (serde `deny_unknown_fields`);
3. unsupported `projection_version` (not `projection.v1`);
4. non-canonical bytes — re-rendering the parsed envelope through `try_canonical_bytes` must reproduce `raw` byte-for-byte (this rejects whitespace, unsorted keys, and forbidden raw-number lexemes);
5. `projection_digest` mismatch against recomputed payload digest;
6. `source_digest`/`projection_digest` not lowercase 64-hex.

```rust
pub fn verify_projection_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    wf: &WorkflowDefinition,
) -> Result<(), Vec<Diagnostic>>
```

Recomputes the source identity from the AST (§3 gate + digest) and requires both the `source_digest` and the payload equality to match — this is the identity layer that catches a tampered `source_digest`. The envelope is not self-authenticating; identity is always established against the source AST (or a sealed record of it).

---

## 6. Human-Readable Plan Projection

```rust
pub fn project_human_plan(
    wf: &WorkflowDefinition,
    policy: Option<RedactionPolicy>,
) -> Result<VersionedProjectionEnvelope<String>, Vec<Diagnostic>>
```

- Non-normative, non-editable, read-only text. First line of the artifact:
  `NON-NORMATIVE VIEW — derived from source digest <source_digest>; not an executable contract.`
- `projection_digest = sha256_hex(plan_text_bytes)` — any tamper (including the disclosure section) is detectable against the expected digest.
- Body order = `workflow::execution_graph::topological_order(wf)` (the compiler's SPEC_002 / declaration-ordinal ordering), not an ad-hoc sort. If `topological_order` returns `None` (cycle), fail closed with `PROJ-0001`.

### Stable grammar (golden-fixtured)

Section order and field lines are fixed; tests assert exact byte equality against `tests/projection_golden/*.human.txt`.

```
NON-NORMATIVE VIEW — derived from source digest <source_digest>; not an executable contract.
# Workflow: <id> v<version> (<name>)
Schema: <schema_version>  Kind: <workflow kind or plain>
Purpose: <purpose or omitted line>

## Authority Ceiling
  ExecutionClass / Mutation / Readable scopes / Writable scopes / Tools /
  Network / Provider / Secrets / Escalation / Review / Abstention /
  Budgets / Content restrictions   (each: value or "none")

## Body (topological order)
### s<ordinal>:<operation_id> [COMPOSITE|ATOMIC]
  Inputs:  <name>: <type>[, outcomes...]
  Outputs: <name>: <type>[, emits...]
  Authority: <reduced authority delta vs ceiling, or "inherit ceiling">
  Effects: <name> [review] [irreversible]
  Uses: [<capabilities>]
  Secrets: [<names>]
  Context: discloses=[...] requires=[...]
  Grants: <authority grants on this unit, or "none">

## Constraints
  <id>: <predicate> (<violationCategory>, <kind>, eval=<evaluationPoint|none>)

## Evidence References
  <id>: event=<eventDigest> artifact=<artifactDigest> kind=<artifactKind> by=<producedBy> at=<producedAt|->

## Disclosure
  Redactions: <n>   Omissions: <n>
  redacted: <item>   omitted: <item>       (one line each, when present)
```

Composite boundaries are explicit: each body unit prints its ordinal + `[COMPOSITE]`/`[ATOMIC]` marker; the workflow-level `kind: composite` is shown in the header. Typed inputs/outcomes, reduced authority, review/escalation gates, and explicit omissions/redactions are all first-class lines per the activation contract.

**Map ordering:** every map-like field (tools, budgets) is rendered from `BTreeMap` iteration or explicitly sorted — never from `HashMap` order.

---

## 7. Redaction / Disclosure Semantics (single sentence, binding)

**Canonical JSON is an unredacted semantic projection and is never redacted; the human plan may apply an optional disclosure/redaction policy, applied to the rendered text before envelope serialization, and its output is not a semantic authority artifact.**

- Human path, `policy = Some(p)`:
  - known secrets = `p.known_secrets` ∪ `workflow::redaction::collect_known_secrets(wf)`;
  - rendered text passed through `workflow::redaction::Redactor` (verbatim replacement with `REDACTED_PLACEHOLDER`, plus the existing credential-shape patterns: URL userinfo, auth headers, Bearer tokens, JSON/query credentials);
  - replaced/omitted items are enumerated in the `## Disclosure` section with counts (redaction may drop or mask text).
- Human path, `policy = None`: no redaction; `Disclosure: Redactions: 0 Omissions: 0`.
- Re-uses **only** `crate::workflow::redaction` — no second redaction taxonomy.
- Bit-stability: unredacted human plans are byte-stable; redacted plans are stable **only for a pinned secret set** (tests pin it). Redacted output must never be presented as an authority-bearing artifact.

---

## 8. Diagnostics (projection-specific codes)

| Code | Meaning |
|------|---------|
| `PROJ-0001` | projection input failed the validation gate, or ordering/cycle failure |
| `PROJ-0002` | projection identity mismatch (source_digest / payload vs expected) |
| `SOMA-CMP-0001` | unsupported version (SOMA schema **or** `projection_version`) |
| `SOMA-CMP-0004` | digest / canonicalization failure (CanonicalError mapping) |

All errors are `Vec<Diagnostic>`; no panics, no partial outputs, no silent fallbacks.

---

## 9. Test Matrix (RED → GREEN)

| Test | Asserts |
|------|---------|
| `canonical_projection_is_byte_deterministic_across_seeds` | 10 AST seeds, each built twice with different map insertion order ⇒ identical envelope bytes + both digests |
| `one_byte_semantic_change_changes_both_digests` | single-field AST mutation ⇒ `source_digest` and `projection_digest` both change |
| `tampered_payload_fails_projection_digest_check` | payload byte flip ⇒ `verify_canonical_projection_bytes` fails closed |
| `tampered_source_identity_fails_against_source` | `source_digest` rewrite ⇒ `verify_projection_against_source` fails `PROJ-0002` |
| `tampered_version_fails_closed` | `projection_version` → `"projection.v2"` ⇒ `SOMA-CMP-0001`, rejected |
| `tampered_disclosure_metadata_fails_closed` | human `## Disclosure` line edit ⇒ recomputed `projection_digest` mismatch |
| `non_canonical_bytes_fail_closed_on_read` | injected whitespace / raw-number lexeme / duplicate key / unknown field ⇒ rejected |
| `human_golden_fixtures_match` | golden files cover authority, gates, omissions, typed outcomes, composite boundary markers — exact equality |
| `human_order_matches_compiler_topological_order` | printed order = `execution_graph::topological_order` |
| `human_plan_cannot_add_authority_or_canonical_fields` | negative: no capability/scope/field appears in human output that is absent from the source AST |
| `projection_input_requires_validated_ast` | un-audited / wrong-schema AST ⇒ `PROJ-0001`, no output |
| `existing_compiler_and_permit_tests_remain_green` | full suite (covered by local gates) |

Golden fixtures: regenerate only via an explicit, reviewed version bump — never silently.

---

## 10. Delivery Contract

- Branch: `feat/e4-x07-projections-slice-1` from clean `main@db98758`.
- Keep the PR **draft**; one coherent production change + RED→GREEN regressions + change record.
- Local gates only (no GitHub Actions / hosted CI): `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features -- --test-threads 4`, doc tests.
- Exact-head evidence on a clean tree: `local_ci.py run --suite core`, `--suite platform`, `--suite smoke`, then `local_ci.py verify --commit <HEAD>` with the three evidence JSONs (verify rejects `--allow-dirty`).
- Attach evidence paths + SHA-256 digests and honest failed-attempt history to the PR; no merge until fresh independent review.

## 11. Acceptance Criteria

1. All §9 tests green (determinism, tamper ×5, golden, authority-subset, input gate).
2. Golden fixtures match exactly; any drift fails the suite.
3. Full `--all-targets --all-features` suite + doc tests pass (no regressions to compiler/permit tests).
4. Exact-head `core` + `platform` + `smoke` PASS; `local_ci.py verify` PASS for the exact HEAD commit.
