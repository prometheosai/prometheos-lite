# E4/X07 Slice 2: Graph Projection with Nested Composite Disclosure Boundaries — Design Spec

**Status**: Approved (rev. 3 — final test 28 precision repair applied; cleared for commit and `writing-plans`; implementation only after plan review)
**Branch**: `feat/e4-x07-slice-2-graph-projection`
**Base**: `main@84d44e4` (post PR #234 merge; tree `3d7ecf75…`)
**Activates**: issue #164 comment `#5952896515` ("ACTIVATED — E4/X07 Slice 2")
**Bounds**: approval comment `#5952406222` (graph only; review report, evidence timeline, model-native formats excluded)

---

## 1. Scope and Ownership

Slice 2 delivers a **graph data projection** of the canonical SOMA++ workflow AST: a deterministic, byte-stable canonical JSON document with nested composite boundaries whose internals are withheld by default and revealed only by an explicit, pre-adjudicated disclosure policy. Projections remain read-only views over one source AST; they never define, alter, or extend SOMA semantics.

### In Scope (Slice 2)

| Deliverable | Notes |
|---|---|
| Inline recursive body parsing (`BodyItem`) | exact `oneOf` discrimination per vendored schema |
| Recursive audit validation | nested scopes validated with the same gate rules |
| Graph payload + `project_graph_json` | nested-subgraph model, withheld-by-default boundaries |
| `GraphDisclosurePolicy` | boundary-level, non-cascading, principal-free |
| `verify_graph_projection_bytes` / `verify_graph_against_source` | B2 self-contained verification |
| Domain-separated digests | `childSubgraphDigest`, `policyDigest`, `graphSchemaVersion` binding |
| Fail-closed dataflow algorithm | §7; outcome/boundary-value edge classes |
| Change record | CHANGELOG bullet + exact-head evidence |

### Explicit Non-Goals (Slice 2)

- Review report and evidence timeline projections; model-native encodings (gated behind SOMA #77 + Foundry #80 promotion).
- An always-unredacted graph variant — canonical JSON (Slice 1) is the complete semantic artifact.
- Principal/identity resolution: the projector receives an already-adjudicated policy; no identity plumbing.
- Constraints, evidence, review gates, escalation, secrets as graph nodes — they remain absent (attributes/references only where already declared at visible boundaries).
- Execution/compiler/graph-runtime support for nested composites: those surfaces **fail closed** (§2.4). No runner, plan, or permit changes.
- Client editing, any write path, authority expansion.

---

## 2. Source Contract: Inline Recursive Body Items

### 2.1 Normative basis (facts of record)

- `vendored/soma/v1.1/schemas/WorkflowDefinition.schema.json` declares `body.items = oneOf[OperationDefinition, CompositeDefinition]`.
- `vendored/soma/v1.1/schemas/CompositeDefinition.schema.json` declares `body.items = oneOf[OperationDefinition, CompositeDefinition]` — **recursion is normative**.
- There is **no external linkage mechanism**: `WorkflowDefinition` has no child-workflow reference field; `references` is audited ⊆ {own id} ∪ operation ids (`audit_workflow.rs:228`, `SOMA-CMP-0002`); no resolver/catalog type exists in `src`.
- Lite's Rust type today is `body: Vec<OperationDefinition>` with `deny_unknown_fields` — a **pre-existing contract gap**: inline composite entries fail deserialization. No valid fixture exercises nesting (`wf-valid-composite.json` has a flat body under `kind: "composite"`).

**Chosen source: inline canonical recursion.** Children are parsed from the same canonical document the caller already holds. Resolution is a pure function of those bytes — there is no second snapshot to keep in sync, no catalog key to invent, and `sourceDigest` (digest of the root serialization minus `contentDigest`) binds the entire nested tree. An external `WorkflowResolver` source is **rejected**: no canonical linkage field exists, so any catalog key would be an invented naming convention (forbidden by the activation bounds), and its "missing/ambiguous/cyclic/digest-mismatched" failure modes cannot be made fail-closed without that invention.

### 2.2 Exact `oneOf` discrimination (required)

```rust
pub enum BodyItem {
    Operation(OperationDefinition),
    Composite(CompositeDefinition),
}
```

`WorkflowDefinition.body: Vec<BodyItem>`; `CompositeDefinition.body: Vec<BodyItem>` (recursive).

Parsing must prove that each item matches **exactly one** variant — implemented as a **manual `Deserialize`** (not `#[serde(untagged)]` trial order, not key heuristics):

1. Deserialize the item to `serde_json::Value`; non-object ⇒ reject.
2. Strict-decode as `OperationDefinition` (its `deny_unknown_fields` + required fields make this an exact-match test).
3. Strict-decode as `CompositeDefinition` (same strictness).
4. Accept `Operation` iff (2) succeeds **and** (3) fails; accept `Composite` iff (3) succeeds **and** (2) fails; **both succeed ⇒ reject** ("ambiguous body item"); **both fail ⇒ reject** ("matches no body-item variant"), surfacing the most specific underlying error.

Exclusivity proof: the variants have **disjoint required discriminator keys** — `executionClass` is operation-only; `body`/`inputPorts`/`outputPorts` are composite-only — and both structs carry `deny_unknown_fields`, so an object carrying both key sets matches **neither**. The both-succeed branch is unreachable under today's types; it is retained fail-closed for future type evolution and covered by review, not runtime.

Parse-time rejections are document-load failures (serde error before any projection/audit runs) — fail closed by construction.

**Required parse negatives** (§12): discriminator collision (union of both variants' keys ⇒ reject), operation missing `executionClass` ⇒ reject, composite missing `inputPorts`/`body` ⇒ reject, malformed nesting (composite body entry that is neither variant) ⇒ reject.

### 2.3 Serialization transparency — hard compatibility gate

`BodyItem` serializes exactly as its inner struct (plain object, schema-compatible `oneOf`). Therefore **every flat workflow's canonical bytes, `workflow_digest_of`, `contentDigest` checks, plan digests, and goldens must be bit-identical to `main@84d44e4`**.

Hard gate test: before the contract change, pin the current `workflow_digest_of` value for **every** vendored `fixtures/valid/wf-*.json` into the test file; after the change they must still match. Any drift blocks delivery.

### 2.4 Consumer fail-closed matrix

Rust's exhaustiveness over `BodyItem` forces every `.body` consumer to decide. Required behavior:

| Consumer | Behavior on `BodyItem::Composite` |
|---|---|
| `audit_workflow` | **Recurses**: the same rules apply per nested scope (ports, dataflow, `SOMA-EXP-*`, `SOMA-OUT-*`, constraints scoping, per-level `authorityImports ⊇ grants` generalizing `audit_workflow.rs:514`, duplicate/self-recursive boundary-id rejection). Root `contentDigest` (`SOMA-CMP-0004`) covers nested bytes. |
| `execution_graph::topological_order` | Returns `None` ⇒ all existing callers fail closed (incl. human plan via `PROJ-0001`). |
| `governance` / `governance_compiler` | Compile/permit refuses with a catalogue-governed error; no plan, permit, or partial output ever emitted. |
| `projection::human` | Fails closed via `PROJ-0001` (uses `topological_order`). |
| `projection::graph` | Full nested support (this slice). |
| `projection::canonical` | Unchanged: serializes the AST as-is (nested bytes included — already true today for any document that parses). |

Refusal parity: nested documents do not parse at all on `main@84d44e4`, so no execution surface regresses; they become parseable and explicitly refused.

**Resolution fail-closed rules** (any depth): duplicate node/boundary ids across the document (including equality with an ancestor id) ⇒ `PROJ-0001`; malformed nesting (§2.2) ⇒ load-time reject; `contentDigest` mismatch ⇒ `SOMA-CMP-0004` at the gate; cyclic dataflow ⇒ `SOMA-EXP-0002` at the gate (plus `topological_order == None` defense ⇒ `PROJ-0001`).

---

## 3. Module Layout and Public API

```
src/workflow/projection/
├── mod.rs          # + graph exports
├── graph.rs        # project_graph_json, verify_graph_projection_bytes,
│                   #   verify_graph_against_source, edge/node builders
├── disclosure.rs   # GraphDisclosurePolicy + validation/normalization
src/workflow/soma/
├── contracts.rs    # + CompositeDefinition, BodyItem (manual Deserialize)
├── audit_workflow.rs  # recursive nested-scope validation
tests/
├── projection_conformance_tests.rs   # extended (graph families)
└── projection_golden/                # untouched (no graph goldens)
```

```rust
pub fn project_graph_json(
    root: &WorkflowDefinition,
    policy: Option<&GraphDisclosurePolicy>,
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>>;

pub fn verify_graph_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>>;

pub fn verify_graph_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    root: &WorkflowDefinition,
    policy: Option<&GraphDisclosurePolicy>,
) -> Result<(), Vec<Diagnostic>>;
```

The verifier takes the **identical root document** — the "same resolver snapshot / sealed bundle identity" requirement is satisfied structurally. Envelope reuse: `VersionedProjectionEnvelope<serde_json::Value>`, `projection.v1`, shared `verify_envelope_metadata`, all Slice-1 gate/digest machinery.

---

## 4. Payload Schema

```jsonc
{
  "graphSchemaVersion": "lite.graph-projection.v1",   // Lite-owned; never confused with SOMA schemaVersion
  "policyDigest": "<lowercase 64-hex>",               // §8.2; present even for empty policy
  "inputPorts":  [ { "name", "type", "cardinality", "requiredness" } ],   // root declared boundary
  "outputPorts": [ { "name", "type", "cardinality", "requiredness" } ],
  "nodes": [ Node, ... ],   // root body items, scope topological order
  "edges": [ Edge, ... ]    // root-scope edges only; nested edges live in containers
}
```

Root ports carry no `dataContract` (free-form attachments are deliberately not graph topology).

**Node** (kind-discriminated):

```jsonc
// kind: "atomic"
{ "id": "s0000:<operation_id>",
  "kind": "atomic",
  "inputs":  [ { "name", "type", "acceptedOutcomes": [...] } ],
  "outputs": [ { "name", "type", "emits": [...] | absent } ] }

// kind: "composite"
{ "id": "s0001:<operation_id>",
  "kind": "composite",
  "inputs":  [ { "name", "type", "cardinality", "requiredness" } ],   // boundary ports
  "outputs": [ { "name", "type", "cardinality", "requiredness" } ],
  "childSubgraphDigest": "<lowercase 64-hex>",          // §8.1; ALWAYS present, view-invariant
  // present iff REVEALED (policy-authorized):
  "children": [ Node, ... ],                            // scope topological order; may be []
  "internalEdges": [ Edge, ... ],                       // sorted; may be []
  // present iff WITHHELD (default):
  "disclosure": { "withheld": true,
                  "reason": "private-composite-default",
                  "category": "composite-boundary",
                  "hiddenNodes": <n>,                    // ONLY with count authorization
                  "hiddenEdges": <n> } }                 // ONLY with count authorization
```

Node ids: `s{NNNN}:{operation_id}` — zero-based, 4-digit zero-padded index in the **containing scope's** topological order (same grammar as the human plan, `human.rs:85`). Ids are unique document-wide (§2.4).

**Edge**:

```jsonc
{ "kind": "outcome" | "boundary-value",
  "from": { "node": "<node id>", "port": "<port name>" },
  "to":   { "node": "<node id>", "port": "<port name>" },
  "label": [ "<string>", ... ] }   // §7.4; never empty on a rendered edge
```

Withheld containers never contribute child ids, internal endpoints, labels, or counts (unless count-authorized) to any parent-level structure — see §7.6.

---

## 5. Boundary Semantics (private = structural)

There is **no normative privacy marker** anywhere in `WorkflowDefinition` or `CompositeDefinition` (field inventories verified against the vendored schemas). Therefore:

- **Every `CompositeDefinition` at every depth is a boundary, withheld by default.** Composite ⇒ private boundary; no labels, names, or conventions modify this.
- The **root workflow is visible at its declared boundary**: root identity is carried by the envelope + `sourceDigest`, root ports are in the payload, root-level atomic operations and their external edges render fully.
- **Root body follows boundary policy**: root-level composite items are withheld like any nested one; root-level atomics are not boundaries and always render.
- The disclosure policy is the **only** reveal mechanism; absence of policy ⇒ minimum disclosure (identically for `None` and an empty policy — §6).

---

## 6. Disclosure Policy

```rust
pub struct GraphDisclosurePolicy {
    pub authorized_boundaries: Vec<String>, // node ids of composite boundaries
    pub count_authorization: Vec<String>,   // node ids whose hidden counts may be emitted
}
```

**Validation** (all ⇒ `PROJ-0003`, in `project_graph_json` before rendering):

1. Unknown id (no such node, or id not present in the document) ⇒ reject.
2. Id targeting a non-composite node (or atomic) ⇒ reject.
3. Duplicate id **within either array** ⇒ reject. (Cross-array presence of the same id is **legal and non-contradictory**: count authorization never implies content authorization, and content authorization never implies counts.)
4. **Ancestry traversability**: authorizing boundary `D` requires every ancestor boundary of `D` to also appear in `authorized_boundaries`. Missing ancestry ⇒ reject — a hidden descendant can never surface through unauthorized parents, and no auto-reveal of a parent's other internals can occur.
5. Non-composite ids in `count_authorization` are rejected by rule 2 (counts have no separate target grammar).

**Normalization**: after validation, both arrays are sorted lexicographically (duplicates impossible post-rule 3). Input order never changes output bytes. The raw policy is never embedded — only `policyDigest` (§8.2).

**Semantics**:
- `None` and `Some(empty)` render **byte-identical** payloads (both normalize to the empty policy; same `policyDigest`).
- **Non-cascading**: authorizing a composite reveals its direct contents (`children` + `internalEdges` at that level) only. Every private descendant requires its own entry; each nested boundary keeps its own disclosure state.
- **Counts are independent**: `hiddenNodes`/`hiddenEdges` are emitted (on the withheld boundary itself) only when that boundary id is in `count_authorization`; they reveal only integers (recursive totals inside that boundary), never content, labels, ids, or edges. Count authorization never renders children.
- The projector performs no principal lookup; callers supply an already-adjudicated policy.

---

## 7. Boundary-Port Dataflow Algorithm

### 7.1 Scope and channel model

A **scope** is the root workflow body or one composite's body. Within a scope:

- **Channels** are names, exactly as the audit models them: an input named `N` is supplied by internal producers (outputs named `N`, `producers` map parity with `audit_workflow.rs:330-334`) and/or the scope's **boundary input ports** named `N` (`SOMA-EXP-0005` parity: no producer and no boundary port ⇒ gate already failed).
- At root, boundary supply is **ambient** (workflow input/output ports are declared in the payload, not rendered as edges — matching the audit, where `boundary_in` is a set, not an edge). At nested scopes, boundary supply and pass-through **are rendered** as edges whose endpoint is the container node's own port.
- Body items at a scope: operations (own I/O ports) and composites (boundary ports). A container's children and internal edges belong to the **child scope**, never to the parent's `nodes`/`edges` arrays.

### 7.2 Edge classes

| Class | Endpoint kinds | Label domain |
|---|---|---|
| `outcome` | operation output → operation input, same scope | outcome variants (§7.4) |
| `boundary-value` | anything touching a composite/boundary port (container port ↔ operation port, container port ↔ container port) | `[<shared value type>]` |

`CompositeDefinition` ports declare no `acceptedOutcomes`/`emits` (schema fact), so boundary edges carry value types only; outcome validation of nested operations remains the audit's job inside the child scope.

### 7.3 Rules

**R1 — composite input supplies matching child inputs (authorized rendering).** Inside a revealed container, for each child input named `N` with no internal producer: if the container declares input port `N`, render `boundary-value` edge `{from: {container, N}, to: {child, N}}`, label `[port.type]`, requiring `port.type == childInput.type` (mismatch ⇒ `PROJ-0001`, "incompatible boundary supply"). Multiple child inputs named `N` each get their own edge (name-channel broadcast, matching audit semantics; sink-side fan-out is legal).

**R2 — child output supplies a composite output.** For each container output port `Q` (type `T`): exactly one internal producer output named `Q` is required — **zero producers ⇒ `PROJ-0001` ("absent producer for declared boundary output"); two or more ⇒ `PROJ-0001` ("ambiguous boundary output")**. The single match renders (when the container is revealed) as internal edge `{from: {child, Q}, to: {container, Q}}`, label `[T]`, requiring `producer.type == T` (else `PROJ-0001`). Note the asymmetry with §7.4's multi-producer rule: operation inputs are name-channels with legal fan-in; a boundary output port is a single-value pass-through and must be unambiguous.

**R3 — external edges terminate at composite ports.** At the parent scope, a composite participates only through its boundary ports:
- parent producer output `N` ↔ container input port `N`: `boundary-value` edge into `{container, N}`, label `[port.type]`, requiring `producer.type == port.type`;
- container output port `N` → parent consumer input named `N`: `boundary-value` edge out of `{container, N}`, label `[port.type]`, requiring `consumerInput.type == port.type`;
- container ports supplied by / feeding the parent's own boundary: ambient at root (§7.1); at nested parents, rendered as container↔container `boundary-value` edges by the same name rule.
An edge whose endpoint targets a node that is not in the same scope's `nodes` (including any hidden child) is a **cross-boundary internal reference ⇒ `PROJ-0001`**.

**R4 — authorized rendering of boundary-to-child edges.** When a container is revealed, its `internalEdges` include R1 supply edges, R2 pass-through edges, and the scope's internal `outcome` edges — endpoints pair container ports with child ports exactly as above. Nothing else in the parent structure changes when a container is revealed.

**R5 — withheld rendering never rewires or leaks.** A withheld container contributes exactly: node identity (`id`, `kind`), declared boundary ports, parent-scope edges **already terminating at its ports** (R3 endpoints are ports by construction — no rewiring is possible), its `childSubgraphDigest`, and its `disclosure` block. Its `children`/`internalEdges` fields are absent; no child id, name, count (without count authorization), label, or internal endpoint appears anywhere in the payload. Parent edges are computed before/independently of disclosure state — disclosure only removes subtree fields, never adjacency.

**R6 — free inputs.** An operation input with no internal producer **and** no boundary port fails at the gate (`SOMA-EXP-0005`); the graph builder re-checks defensively (`PROJ-0001`). An operation input with no producer but a matching boundary port renders no parent-level edge at root (ambient) and an R1/R3 edge at nested scopes.

**R7 — unused declared ports/outputs never fail.** An operation output no consumer input matches (no `outcome` edge) renders nothing and is not validated for `emits` — declared-but-unused is legal (audit parity: `SOMA-EXP-0004` only constrains input-less units, and empty-`emitted` inputs `continue` at `audit_workflow.rs:418-420`). Likewise a container input port with no child consumer, and a container output port with an internal producer but no parent consumer, are legal.

**R8 — topological order and cycles.** Each scope's node order = `execution_graph::topological_order` semantics over that scope (dataflow edges, smallest declaration ordinal among ready units). Cycle ⇒ gate `SOMA-EXP-0002`; builder defense: order failure ⇒ `PROJ-0001`.

**R9 — determinism of arrays.** `nodes`/`children`: scope topological order. `edges`/`internalEdges`: lexicographic sort by `(from.node, from.port, to.node, to.port, label-joined)`. Disclosure arrays absent (state lives on nodes); `policyDigest` from sorted policy (§6). Payload serialized through the pinned canonical renderer (lexicographic keys, no whitespace, DecimalV2).

### 7.4 Operation-to-operation outcome labels

For each consumer input `I` (name `N`) at a scope, for each producer output `O` (name `N`) at the same scope (audit union semantics allow fan-in; each producer gets its own edge):

```
label(O→I) = sort_lexicographic( emits(O) ∩ acceptedOutcomes(I) )
```

Fail-closed rules (**`PROJ-0001`**, graph builder):

- **Empty intersection on a matched connection** ⇒ fail. This includes **`emits: None` or empty `emits` on a matched output** — a matched connection requires a label and none can be honestly derived ("rather than invent labels").
- **Unused outputs are exempt**: if no consumer input at that scope shares the output's name, there is no matched connection, no edge, and no `emits` requirement — the output simply does not appear as an edge endpoint (R7).
- **Ambiguity** ⇒ fail: duplicate output names within one operation; duplicate input names within one operation; duplicate boundary port names within one container/scope; duplicate node ids (§2.4).
- **Incompatibility** ⇒ fail: `emits ∩ acceptedOutcomes = ∅` with non-empty `emits` is already refused by the gate (`SOMA-OUT-0002`); the builder re-checks defensively and fails on any empty label (in-module unit test, since the gate makes the path unreachable through `project_graph_json`).
- **Multiple producers**: legal when every matched pair yields a non-empty label (audit `SOMA-OUT-0001/0002` govern union validity at the gate); "multiple illegal producers" fail because each illegal pair fails the empty-intersection rule. Boundary output pass-through is the sole single-producer rule (R2).

Parity note: on an audit-green document, a matched pair with non-empty `emits` always has non-empty intersection (`emitted ⊆ accepted`), so the only new strictness vs. the audit is matched outputs without `emits` data — which the graph fails closed on by design (§12 test 17 proves all vendored valid fixtures project cleanly).

---

## 8. Digests and Version Validation

### 8.1 `childSubgraphDigest` (per composite, always present, view-invariant)

**Acyclic commitment construction.** The preimage is the boundary's subtree in **unwithheld form**, with two rules that eliminate self-reference:

- the **subject boundary's own `childSubgraphDigest` field is omitted** from its own preimage (and its `disclosure` state is irrelevant — unwithheld form has no `disclosure` fields anywhere);
- **descendant `childSubgraphDigest` fields are included as final values**, computed **bottom-up** (depth-first post-order: leaf composites first — their preimage is the node structure without its own digest field — then each ancestor's preimage embeds the already-computed descendant digests).

Every preimage therefore depends only on strict descendants plus the subject's own structure-without-digest; the construction is deterministic, terminates on any finite tree, and never references itself.

```
childSubgraphDigest =
  try_canonical_digest({
    "domain":   "projection.graph.child-subgraph.v1",
    "graphSchemaVersion": "lite.graph-projection.v1",
    "rootSourceDigest": <envelope sourceDigest of the root>,
    "boundaryId": "<composite node id>",
    "content": <unwithheld render of the boundary's subtree,
                subject's own childSubgraphDigest field omitted,
                descendant childSubgraphDigest fields present (bottom-up)>
  })
```

Binds purpose, schema version, root identity, boundary identity, and canonical content (transitively, via descendant digests). Because `content` is disclosure-independent, the digest is **identical under every disclosure view**.

**Honest threat statement (accepted tradeoff).** The commitment directly discloses no subtree structure. It *does* deliberately enable equality correlation of a boundary's content across views (cross-view audit is the point of view-invariance), and — like any public content hash — may permit **offline guessing of low-entropy candidate subtrees** by an attacker able to enumerate plausible contents. This tradeoff is **accepted for this slice**: verification must recompute commitments from public inputs only, consistent with the Slice-1 pinned digest policy; a keyed/secret-salted construction is explicitly rejected because `verify_graph_against_source` would cease to be publicly reproducible.

**Verification location.** Recomputation against the source happens only in `verify_graph_against_source` (§11, steps 6-7 ⇒ `SOMA-CMP-0004`); the byte path checks shape only (lowercase-64-hex) because a withheld boundary's subtree is absent from the bytes. Mismatch under semantic recomputation ⇒ `SOMA-CMP-0004`.

### 8.2 `policyDigest` (payload root, always present)

```
policyDigest =
  try_canonical_digest({
    "domain":   "projection.graph.policy.v1",
    "graphSchemaVersion": "lite.graph-projection.v1",
    "rootSourceDigest": <envelope sourceDigest of the root>,
    "policy": { "authorizedBoundaries": [sorted ids], "countAuthorization": [sorted ids] }
  })
```

`None` and empty policy produce the same normalized content ⇒ same digest. The raw policy and any principal data are never embedded — consequently the byte path **cannot** recompute this digest (shape check only) and semantic verification happens exclusively at `verify_graph_against_source` §11 step 6 ⇒ `SOMA-CMP-0004`.

### 8.3 `graphSchemaVersion` validation

`"lite.graph-projection.v1"` is the **only** allowed value (closed allow-list constant). Validated explicitly in:

- `verify_graph_projection_bytes` — during payload structural validation (after envelope parse, before digest check);
- `verify_graph_against_source` — as its own step after envelope metadata, before payload digest comparison.

Unknown/absent/mistyped ⇒ `SOMA-CMP-0001`. This is independent of the envelope's `schemaVersion` (SOMA schema, `SOMA-CMP-0001` via `verify_envelope_metadata`, Slice-1 B1) and of the payload identity binding (§11).

---

## 9. Diagnostics

| Code | Graph-slice meaning |
|---|---|
| `PROJ-0001` | graph structure/dataflow failures (§7 ambiguity/incompatibility/absent-producer/dangling/cross-boundary/cycle/duplicate), non-canonical payload bytes, human/execution surfaces refusing nested composites, policy-independent shape errors. **Not** malformed workflow `oneOf` input: that is rejected as a **raw `serde` deserialization error at document load**, before any `Diagnostic` exists — no public loader maps it to `PROJ-0001` (§2.2) |
| `PROJ-0002` | identity vs source: `sourceDigest`, payload/fresh-render equality, envelope `schemaVersion` vs root |
| `PROJ-0003` | **new**: disclosure-policy violations (§6 rules 1-4) |
| `SOMA-CMP-0001` | unsupported version: envelope `projectionVersion`/`schemaVersion`, **or `graphSchemaVersion`** |
| `SOMA-CMP-0004` | digest failure: payload digest, `childSubgraphDigest`, `policyDigest`, root `contentDigest` |

All errors remain `Vec<Diagnostic>`; no panics, no partial outputs, no silent fallbacks. Non-projection execution surfaces use catalogue-governed refusal diagnostics (§2.4). The Slice-1 diagnostics table stays valid; this table extends it for the graph slice (`PROJ-0003` is new here).

---

## 10. Determinism

- Scope order = deterministic topological order (declaration-ordinal tie-break); edges sorted (R9).
- Single serialization path: pinned canonical renderer (lexicographic keys, no whitespace, DecimalV2, fail-closed on non-canonical numbers).
- Two builds of the same AST with different map insertion orders ⇒ identical envelope bytes (seeded determinism test, Slice-1 technique).
- Policy permutations ⇒ identical bytes (sorted normalization).

---

## 11. Verification Paths

**`verify_graph_against_source(envelope, root, policy)` — ordered, self-contained (B2 pattern).** Steps 6-7 deliberately precede the wholesale payload comparison so derived-digest mismatches report `SOMA-CMP-0004`, never `PROJ-0002`:

1. Validate source and metadata: `validated_source(root)` — audit gate (recursive, §2.4) + schema equality — then `verify_envelope_metadata(envelope)` — `projectionVersion`, SOMA `schemaVersion`, digest hex shapes.
2. Validate graph version: `graphSchemaVersion` allow-list check (§8.3) ⇒ else `SOMA-CMP-0001`.
3. Verify envelope payload digest: `digest_of(envelope.payload) == envelope.projection_digest` ⇒ else `SOMA-CMP-0004`.
4. Bind envelope/root schema: `envelope.schema_version == root.schema_version` ⇒ else `PROJ-0002`.
5. Normalize and validate policy (§6 rules 1-4; `None` ⇒ normalized empty) ⇒ else `PROJ-0003`.
6. Explicitly recompute `policyDigest` from the normalized policy and compare to `payload.policyDigest` ⇒ else `SOMA-CMP-0004`.
7. Explicitly recompute every `childSubgraphDigest` from the source (§8.1 acyclic construction) and compare to the embedded values ⇒ else `SOMA-CMP-0004`.
8. Fresh `project_graph_json(root, policy)` (re-runs policy validation, all §7 rules, digest recomputation) and compare remaining fresh payload/source identity — `sourceDigest` + full payload equality ⇒ else `PROJ-0002`.

No prior byte-verification call is required; forged `graphSchemaVersion` ⇒ step 2 (`SOMA-CMP-0001`), forged payload digest ⇒ step 3 (`SOMA-CMP-0004`), forged `policyDigest`/`childSubgraphDigest` ⇒ steps 6/7 (`SOMA-CMP-0004`), other node/edge/disclosure/source edits ⇒ step 8 (`PROJ-0002`).

**`verify_graph_projection_bytes(raw)` — byte-path capability contract.** Envelope parse (duplicate keys → shape → envelope metadata → canonical re-render) → `graphSchemaVersion` allow-list → payload structural validation (node/edge/disclosure shapes, id grammar, digest-field shapes (lowercase-64-hex), endpoint references visible in the payload, kind-specific required fields, withheld/revealed exclusivity) → payload digest check (`projectionDigest`). It establishes **byte integrity, versions, structure, visible references, and `projectionDigest` only** — it cannot establish semantic truth of `policyDigest` (the normalized policy is intentionally not embedded) or of a withheld boundary's `childSubgraphDigest` (the hidden subtree is absent), so it **never claims `SOMA-CMP-0004` for those fields**; semantic recomputation is exclusively `verify_graph_against_source` steps 6-7 (same split as Slice 1, with an explicitly narrower byte path).

---

## 12. Test Matrix (RED → GREEN)

Contract & compatibility (items 1-2 pin current behavior; items 3-5 start RED on `main@84d44e4` — all captured **before** the contract change):

| # | Test | Asserts |
|---|---|---|
| 1 | `flat_workflow_digests_remain_pinned` | `workflow_digest_of` for every vendored `valid/wf-*.json` equals constants captured at `main@84d44e4` (hard gate) |
| 2 | `flat_canonical_bytes_unchanged` | canonical bytes pinned for every vendored flat fixture at `main@84d44e4`; round-trip after the change equals the pinned bytes |
| 3 | `nested_composite_document_parses` | inline `CompositeDefinition` at root and nested depth parses (RED on main) |
| 4 | `body_item_one_of_negatives` | discriminator-collision (union keys), missing `executionClass`, missing `inputPorts`/`body`, malformed nesting ⇒ each rejects; no partial accepts |
| 5 | `recursive_audit_rejects_nested_faults` | nested duplicate/self id, nested un-fed input (`SOMA-EXP-0005`), nested authority-import violation ⇒ gate fails, no projection |

Disclosure semantics:

| # | Test | Asserts |
|---|---|---|
| 6 | `default_withholding` | composites present as boundary nodes only; no children/internalEdges/child ids/labels/counts in payload (full-text scan for child ids) |
| 7 | `authorized_selective_reveal` | direct contents render; each nested boundary keeps its own state |
| 8 | `non_cascading_authorization` | parent entry does not reveal descendants (separate entries required) |
| 9 | `ancestry_traversal_enforced` | descendant-without-ancestors ⇒ `PROJ-0003`; no partial reveal |
| 10 | `policy_validation_negatives` | unknown id, non-composite target, in-array duplicates ⇒ `PROJ-0003` |
| 11 | `count_authorization_independent` | counts render without content; content stays hidden; without count auth no counts |
| 12 | `none_equals_empty_policy` | `None` vs `Some(empty)` ⇒ byte-identical payloads + equal `policyDigest` |
| 13 | `policy_order_irrelevant` | permuted policy arrays ⇒ identical bytes |
| 14 | `disclosure_views_share_source_digest` | two views: equal `sourceDigest`, distinct `projectionDigest` (explicit) |

Leakage, structure, determinism:

| # | Test | Asserts |
|---|---|---|
| 15 | `structural_no_leakage_diff` | canonical-parse default vs revealed payloads; exclude exactly `{envelope.projectionDigest, payload.policyDigest}`; deep-equal outside authorized regions; **assert `childSubgraphDigest`s equal across views** (view-invariance) |
| 16 | `graph_deterministic_across_seeds` | N seeds, different map insertion order ⇒ identical envelope bytes |
| 16a | `nested_digest_construction_acyclic_and_deterministic` | depth-2 fixture: building twice ⇒ equal digests; embedded value == recomputed value from the §8.1 acyclic preimage (subject's own field excluded, descendant digests bottom-up); mutating a deep-leaf field ⇒ every ancestor `childSubgraphDigest` changes (transitive binding); no self-reference (recomputation terminates and matches) |
| 17 | `all_vendored_valid_fixtures_project` | every `valid/wf-*.json` projects + verifies honestly (guards §7.4 parity claim) |
| 18 | `authority_expansion_impossible` | every node/edge/disclosure field traceable to AST declarations (subset traversal; no invented ids/labels) |

Dataflow algorithm:

| # | Test | Asserts |
|---|---|---|
| 19 | `composite_input_supplies_child_inputs` | R1 edges render with shared type label; type mismatch ⇒ `PROJ-0001` |
| 20 | `child_output_feeds_composite_output` | R2 single-producer pass-through edge (authorized) |
| 21 | `boundary_output_absent_or_ambiguous_fails` | zero producers / two producers ⇒ `PROJ-0001` |
| 22 | `external_edges_terminate_at_ports` | parent edges target container ports only; cross-boundary child reference in crafted bytes ⇒ `PROJ-0001` |
| 23 | `withheld_no_rewiring` | default view's parent adjacency byte-identical to revealed view's parent adjacency (parent-level edge set unchanged by disclosure) |
| 24 | `matched_without_emits_fails_unused_output_passes` | matched output with `emits: None` ⇒ `PROJ-0001`; same output with no name-matched consumer ⇒ projection succeeds (R7) |
| 25 | `dataflow_defensive_rules` (in-module) | empty-intersection, duplicate port/output names, dangling endpoints ⇒ `PROJ-0001` (unit tests below the gate) |

Verification & envelope integrity (graph flavors of Slice-1 B1/B2):

| # | Test | Asserts |
|---|---|---|
| 26 | `graph_honest_round_trips` | project → canonical bytes → verify bytes → verify against source: `Ok` (default and authorized views) |
| 27 | `graph_forged_envelope_metadata_fails` | forged `projectionVersion`/`schemaVersion`/`projectionDigest` directly constructed ⇒ same codes as Slice 1 |
| 28 | `graph_semantic_forgery_rejected_by_source_verification` | (a) edited `graphSchemaVersion` + recomputed `projectionDigest` ⇒ byte path `SOMA-CMP-0001` (allow-list independent of digests); (b) structurally valid payload field edited **without** recomputing `projectionDigest` ⇒ byte path `SOMA-CMP-0004` (step 3; the stale digest — not any shape error — is asserted to be the cause); (c) derived-digest forgeries — edited `policyDigest` (⇒ step 6) or `childSubgraphDigest` (⇒ step 7) with recomputed `projectionDigest` — **pass the byte verifier's structural contract by design** (normalized policy and withheld subtrees are not derivable from bytes; asserted explicitly to pin that limitation) yet **fail `verify_graph_against_source` at steps 6/7 with `SOMA-CMP-0004`**; (d) structurally valid disclosure-state/content modification (disclosure block, `children`, `internalEdges`) with unchanged valid derived digests and recomputed `projectionDigest` ⇒ passes steps 6-7 (derived digests still verify) and **fails the fresh-render comparison at step 8 with `PROJ-0002`** |
| 29 | `human_and_canonical_regressions` | existing Slice-1 suites (35) unchanged and green; full suite green |

Golden fixtures: none for graph JSON (digest + structural assertions cover determinism; JSON structure does not need text goldens).

---

## 13. Delivery Contract

- Branch `feat/e4-x07-slice-2-graph-projection` from `main@84d44e4`; **one coherent draft PR**; keep draft until fresh independent exact-head review approves; no self-merge, no hosted CI/third-party gates.
- Subagent-driven execution with per-task briefs/reviews, ledger under `.superpowers/sdd/`, controller adjudicates all deviations (Slice-1 process).
- TDD Iron Law: RED evidence captured before each GREEN; test numbering as §12.
- Local gates only: `cargo fmt --all -- --check`; `cargo clippy --offline --all-targets --all-features -- -D warnings`; `cargo test --offline --all-targets --all-features -- --test-threads 4`; `cargo test --offline --doc --all-features` — via the standing `cmd /c` + `CARGO_TARGET_DIR` wrapper, PowerShell 5.1, offline.
- Exact-head evidence on a clean tree: `local_ci.py run --suite core|platform|smoke` (one call each) + `local_ci.py verify --commit <sha>` with the three JSONs; `--allow-dirty` forbidden; honest failed-attempt history in the PR.
- CHANGELOG bullet at delivery (byte-verified, em-dash style as Slice 1).
- PR body: honest history, evidence SHA-256s, disclosed deviations; fresh review requested while draft.

## 14. Acceptance Criteria

1. All §12 tests green, including the hard flat-digest invariance gate and the parse negatives.
2. Default view withholds every nested boundary; no leakage through edges, labels, ids, ordering, or (unauthorized) counts — structurally proven (tests 6, 11, 15, 23).
3. Distinct disclosure views share `sourceDigest`, differ in `projectionDigest`; both verify against the same root (test 14, 26).
4. All four gates exit 0; exact-head core/platform/smoke evidence PASS at the delivered commit.
5. Flat-workflow artifacts bit-identical to `main@84d44e4`; full suite green.
6. Nested composites fail closed on every non-projection surface (§2.4 matrix tested where reachable).
7. Draft PR approved by fresh independent exact-head review before merge.
