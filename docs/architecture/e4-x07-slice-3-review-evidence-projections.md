# E4/X07 Slice 3: Review-Report + Evidence-Timeline Projections — Design Spec

**Status**: Draft for independent plan review — implementation BLOCKED until this plan is approved. Not a code-change.
**Branch**: `docs/e4-x07-slice3-spec`
**Base**: `main@66d3362` (post PR #242 merge)
**Activates**: issue #243 (parent #164, CLOSED)
**Bounds**: review-report + evidence-timeline projections only; no #132 Slice 3, #217, compiled-plan work (#163), compact/model-native promotion (SOMA #77 + Foundry #80), or Lite runner/governance changes.

---

## 1. Scope and Ownership

Slice 3 adds the two remaining required-stable E4/X07 projections required by the #164 acceptance matrix: a deterministic **review-report** projection and a deterministic **evidence-timeline** projection. Both are read-only views; they never define, alter, or extend SOMA semantics, never mutate the canonical AST, and never widen reviewer/caller authority. Human-facing/reporting fields are explicitly non-normative state.

### In Scope (Slice 3)

| Deliverable | Notes |
|---|---|
| `ReviewProjectionPayload` (`lite.review-report.v1`) | deterministic review-report rendering over authoritative review/gate/evidence facts |
| `TimelineProjectionPayload` (`lite.evidence-timeline.v1`) | deterministic evidence-timeline rendering over authoritative journal/work-event facts |
| Shared envelope reuse | `VersionedProjectionEnvelope<V>` for both; no fork of envelope semantics |
| Disclosure policies | field/section-level withholding, non-cascading; counts independently sensitive |
| Two-verifier contract | structural byte verifier + against-source recompute verifier per projection |
| Diagnostics | existing taxonomy reused; minimal additions only where no existing code expresses the fault |
| Change record + fixtures | redacted goldens + invalids |

### Explicit Non-Goals (Slice 3)

- Compiled-plan projections (#163 owns).
- Always-unredacted review/timeline variants (canonical canonical JSON and Slice-2 graph are the complete artifacts; disclosure is filter-only).
- Principal resolution or authen/authz identity plumbing: the projector receives already-adjudicated inputs as Slice 2 did.
- Any write path, projection-to-AST mutation, editable semantic surface, unstable format, or nondeterministic ordering.
- UI, WebSockets/streaming, runtime projection gates for the Lite runner.

**Facts of record (reconnaissance; recon mapping table in §2).** Existing authoritative types enumerated below are used as *source material for projection only*. Their semantics/domains/back-compat must not change.

---

## 2. Reconnaissance: Source-of-Truth Inventory

Reconnaissance against `main@66d3362` (slice-1 spec §1 basis, slice-2 spec §1 basis; projection code semantics from Slice 1/2; structs verified by grep) established the following authoritative sources — Slice 3 projects rather than duplicates them:

| Source concept | Existing authoritative type/contract (path) | Slice-3 projected representation | Disclosure rule | Digest/identity binding |
|---|---|---|---|---|
| Versioned envelope wrapper | `VersionedProjectionEnvelope<V>` (`src/workflow/projection/envelope.rs:18`) | direct reuse for Slice-3 payloads (`lite.review-report.v1`, `lite.evidence-timeline.v1` `payload` values) | n/a (envelope itself carries non-negotiable bindings) | envelope `sourceDigest` = `source_digest_of(wf)` (`projection/mod.rs:59`); `projectionDigest` = canonical digest of payload |
| Canonical AST identity | `governance_compiler::workflow_digest_of` (`src/workflow/governance_compiler.rs`), alias `source_digest_of` (projection/mod.rs:59) | envelope `sourceDigest` (unchanged) | n/a | bound to canonical `WorkflowDefinition` minus `contentDigest` |
| Human plan projection | `render_plan_body` / `project_human_plan` (`projection/human.rs`) | unchanged | optional `RedactionPolicy` | payload digest = `sha256_hex(text bytes)` |
| Canonical JSON projection | `project_canonical_json` (`projection/mod.rs`), `verify_canonical_projection_bytes` | unchanged | n/a (complete artifact) | `digest_of(canonical_value)` |
| Graph projection + disclosure | `GRAPH_SCHEMA_VERSION`, `GraphPayload`, `GraphDisclosurePolicy`, §7 dataflow (`projection/graph.rs`/`disclosure.rs`) | unchanged; Slice 3 may reference its policy/digest conventions but does not fork them | boundary-level non-cascading policy | `policyDigest = digest_of(policy_digest_preimage)` with domain `projection.graph.policy.v1` |
| Review report facts | `ReviewReport`, `ReviewIssue`, `ReviewIssueType`, `ReviewSeverity`, `ReviewSummary`, `ReviewQualityScore`, `ReviewQualityMetrics` (`src/harness/review.rs`) | `ReviewProjectionPayload.issues`, `.summary`, `.disposition`, `.review` reference | `withheld`/`unavailable` semantics; counts independently sensitive | issue list → canonical digest via `digest_of`; references require `reportId` stable string (projection-domain) |
| Gate decision records | `HumanDecisionRecordV1`/`HumanVerdict`/`ReviewChannel`/`FailureClass` (`src/workflow/graph_gates.rs`) | `ReviewProjectionPayload.gates` / `.verdict` reference | presence of `decided_by` principal is disclosed only when policy authorizes principal visibility; else `withheld` with `category: "principal"` | each gate entry carries its own digest-domain reference (see §6) |
| Node review findings | `SecurityFindingV1`, `ReviewKind` (`src/workflow/node_review.rs`) | optional cross-reference entries only; no duplication of typed payload | existing path-scoped redaction carries over | evidence pointer retains `artifactDigest/producedBy` semantics |
| Governance evaluation evidence | `EvidenceBundle`, `ValidationRecord`, `ProviderProvenanceRecord`, `IntegrityRecord`, `CleanupRecord`, `FailureClassification` (`src/workflow/evaluate/evidence.rs`) | timeline events of type `evaluation.*` plus review verdict derivation | preview fields (`stdout_preview`, `stderr_preview`) disclosed only under explicit policy; failures/classifications always inspectable but NOT ever promoted to fake empty evidence | `EvaluationBundleId = canonicalDigest(evaluationEnvelopeMinusPayload?)` — NOT used bluntly; each `ValidationRecord`-derived event references `runId` + `completedAt` + `patchHash`/`baseSha` presence, never inline tails |
| Provenance envelope | `ProvenanceEnvelope`, `Producer`, `AuthorityRecord`, `ExecutionClass`, `RepoBinding` (`src/work/provenance.rs`) | timeline event `provenance` summary subset (producer.kind, repo_binding.revision/dirty marker, execution_class) | absent principal disclosed as `Absent`, never promoted to success/absence | opaque reference `provenanceDigest = digest_of(provenanceJson)`; never re-serialized independently |
| Journal record | `JournalRecord`, `StoredColumns`, `ProvenanceState` (`src/db/repository/work_context_events.rs`) | source-of-truth rows projected into events | provenance state `LegacyUnverified` is surfaced as a first-class status, never silently upgraded | stored `sourceDigest`/`data`/`createdAt` bind as stored bytes |
| WorkEvent semantic identity | `WorkEvent` (`schema_version, version, id, event_type, actor, authority, effective_authority, sequence, timestamp, idempotency_key, correlation_id, repo_revision, compatibility, semantic_digest, parents, evidence, payload, ...`) (`src/workflow/soma/event.rs`) | timeline event derived fields; `sequence`, `semantic_digest`, `id`, `event_type` mapping categories (`context/evidence/decision/lifecycle`) per §6.x of issue context (`work/soma_projection.rs:82`) | event payload families (`EventPayload` variants) disclosed by category | `WorkEvent.semantic_digest` is the authoritative event digest — no new field is invented |
| Run/grouping binding | `WorkEventBatch`, `RunKey`, `RunKeyKind` (`src/work/soma_projection.rs`) | timeline `scope` carries exactly one `RunKey`; grouping/canonical ordering per run | scope metadata is mandatory; no cross-run merge without explicit policy | `RunKey` typed id/kind wire spelling per existing binding |
| Diagnostic vocabulary | `Diagnostic` (`src/workflow/soma/mod.rs:42`), families `SOMA-CMP-*`, `SOMA-AUTH-*`, `SOMA-EVT-*`, `PROJ-*` | prose: enumeration over the same type with minimal new codes (§7) | n/a (diagnostics are only emitted, never transformed) | n/a |
| Typed outcome vocabulary | `OutcomeVariant` (`Produced|Skipped|Blocked|Failed|Cancelled|ReviewRequired`) (`src/workflow/soma/types.rs:163`); `SUCCESS_VARIANT`, `FAILURE_VARIANTS` | used verbatim for review disposition categories where applicable | never treated as auth | n/a |
| Timeline lineage | legacy `TimelineEvent` lives in `src/flow/tracing.rs` — explicitly NOT the projection (kept out of scope as source-of-truth) | Slice-3 timeline sources from `WorkEventBatch`/`JournalRecord` only | n/a | n/a |
| Local-CI evidence manifest | `.local-ci/evidence/*.json` schema keys (`commit`, `suite`, `platform`, `python`, `rustc`, `cargo`, `architecture`, `workingTreeClean`, `completedAt`, `checks[]{name, command[], seconds, status}`) | informs timeing-manifest event type vocabularies only for telemetry metadata; Slice 3 does not project local-CI artifacts | n/a | n/a |

**Discipline enforced by this table:** there is no new semantic type for events, findings, gates, outcomes, or authority. `ReviewProjectionPayload` and `TimelineProjectionPayload` are thin, deterministic adapters over the source material, plus disclosure lists and a final `disclosurePolicyDigest`/`timelineDigest` digest-binding key.

---

## 3. Shared Slice-3 Contract (Envelope / Identity / Semantics)

### 3.1 Projection envelope binding (reuse, no fork)

Both projections produce `VersionedProjectionEnvelope<V>` (envelope.rs:18) with:

- `projectionVersion` = `"projection.v1"` — same allow-list as Slice 1/2; no new envelope-semantics value.
- `schemaVersion` = `"lite.review-report.v1"` or `"lite.evidence-timeline.v1"` respectively (see §4/§5). Existing `SOMA-CMP-0001` → unsupported version semantics carry over unchanged.
- `sourceDigest` = `projections::mod::source_digest_of(wf)` identity — not overloaded or redefined.
- `projectionDigest` = `digest_of(payload)` where payload serializes with the existing canonical-JSON rules (§3.4). Human-style `sha256_hex(payload bytes)` used for envelopes applies only where the payload payload is `String`; for JSON payloads, `digest_of` is retained (Slice 1/2/consistent).
- Envelope JSON is rendered via `envelope::canonical_bytes()` (to_value + `try_canonical_bytes`); bytes MUST round-trip through `parse_envelope_bytes` (duplicate-key scan, strict metadata bounds, canonical-render equality). Unknown keys / duplicates / non-canonical render ⇒ parse failure — no special branch in Slice 3.

### 3.2 Direct-edit / non-editability semantics (Slice 1/2 invariant)

- Slice-3 payloads are projection artifacts only: there is no `canonicalize_report`-to-AST edit path; a projection MUST NOT be accepted as semantic input anywhere else in the codebase.
- Any projection-envelope with rewired `sourceDigest` not matching its `project_*` output ⇒ `PROJ-0002` (identity mismatch). Directed tampering of `projectionDigest`, payload bytes, or envelope metadata ⇒ structural verifier `SOMA-CMP-0004` / `PROJ-0002` pair.

### 3.3 Authority non-expansion invariant

- Projection reading MUST NOT manufacture: review authority, gate decisions, findings, sources of evidence, timestamps, or principals. All fields come from source material. In particular, `decision: "approved"` MUST NOT imply an absent `HumanDecisionRecordV1` claim; `passed: true` requires a present source `passed` field; otherwise the corresponding section is rendered `withheld`/`unavailable` with explicit `category` and `reason`.

### 3.4 Exact digest domains (Slice-3 new preimage domains; no collision with existing)

| Preimage domain label | Used by | Preimage material |
|---|---|---|
| `"projection.review-report.v1"` | review-report envelope | `{domain, schemaVersion, rootSourceDigest, disclosurePolicyDigest, reportCancellations:[], reviewReferenceDigest}` — normalized |
| `"projection.evidence-timeline.v1"` | evidence-timeline envelope | `{domain, schemaVersion, rootSourceDigest, disclosurePolicyDigest, scope{runKey}, eventStreamDigest}` |

### 3.5 Unknown fields, duplicate keys, unsupported versions, malformed bytes

- Unknown top-level envelope fields ⇒ `deny_unknown_fields` rejection (PROJ-0001 structural path).
- Duplicate JSON keys ⇒ byte-level `find_duplicate_key` scan in `parse_envelope_bytes` fails ⇒ PROJ-0001.
- `projectionVersion` outside `ALLOWED_PROJECTION_VERSIONS` ⇒ `SOMA-CMP-0001`.
- `schemaVersion` outside the two Slice-3 allow-lists ⇒ `SOMA-CMP-0001` with code `UnsupportedSchemaVersion` detail.
- Non-canonical envelope re-render ⇒ PROJ-0001.
- Invalid payload schema (unknown key inside payload) ⇒ payload struct `deny_unknown_fields` rejection mapped to `SOMA-CMP-0003` (`schema violation`).

### 3.6 Runtime/evidence identity (not folded into AST identity)

- The AST-level envelope `sourceDigest` remains canonical-workflow-semantic.
- Runtime facts (review decisions, journal rows, work-event family, EvidenceBundle) are referenced, never restated. Each reference is either (a) a stored `semantic_digest` on a `WorkEvent`, (b) a stored `source_digest` on a `JournalRecord` row, or (c) a domain-labelled `digest_of(json_document)` computed over the source object's canonical re-render. The timeline payload's ordering/disclosure fields do NOT carry new identity fields — `timelineDigest` binds the whole rendered payload, one per envelope; individual events carry references, not synthesized ids.

---

## 4. Review-Report Projection (`lite.review-report.v1`)

### 4.1 Payload type

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewProjectionPayload {
    pub review_schema_version: String,        // "lite.review-report.v1"
    pub disclosure_policy_digest: String,     // domain "projection.review-report.v1" preimage
    pub source: ReviewSourceIdentity,         // workflow identity + report scope
    pub authority: ReviewAuthoritySummary,    // derived, never widened
    pub gates: Vec<ReviewGateView>,           // deterministic order (see §4.3)
    pub summary: ReviewSummaryView,           // counts; zero allowed only when §4.4 permits
    pub issues: Vec<ReviewIssueView>,         // deterministic order
    pub omissions: Vec<OmissionView>,         // withheld vs unavailable distinguished
    pub disposition: DispositionView,         // final disposition or `unavailable`
    pub evidence_references: Vec<EvidenceReferenceView>,
}
```

`ReviewSourceIdentity { workflowId: String, sourceDigest: String, schemaVersion: String, reportId: Option<String>, runKey: Option<RunKey> }` — `reportId` is projection scope only; when the authoritative source carries no review-report id, `reportId: null` with `available: false` reason (never synthesized).

`ReviewAuthoritySummary { reviewerPrincipals: Vec<PrincipalView>, reviewChannel: Option<String>, executorClass: Option<String>, repoBinding: Option<RepoBindingView> }` — every principal/channel/class value comes from a source field; if a source has no principal value, the corresponding vector entry is absent AND a `PrincipalView`-shaped `kind: "absent"` marker is emitted in `omissions`.

`ReviewGateView { nodeId: String, gateKind: String, verdict: String, failureClass: Option<String>, basisEvidenceDigest: Option<Hex64>, provenanceState: String }` — verdict is one of `approved | changes_requested | rejected | unknown | unavailable`; maps `HumanVerdict` verbatim; `unknown`/`unavailable` carry the explicit `unknownReason`/`unavailableReason`.

`ReviewSummaryView { totalIssues: Option<usize>, byType: BTreeMap<String, usize>, bySeverity: BTreeMap<String,usize>, passed: Option<bool> }`: counts render only when the authoritative summary exists; `passed: null` with `reason` when the source is absent ⇒ no silent success.

`ReviewIssueView { issueType, severity, file?, line?, message, suggestion?, ruleId }` — derived 1:1 from `ReviewIssue`.

`OmissionView { section: String, category: String, reason: String }` — `category ∈ {withheld, unavailable, outOfScope}`; `reason` is one of `principal`, `secret`, `disclosurePolicy`, `noAuthoritativeSource`, `scopeExcluded`.

`DispositionView { status: String, reviewRequired: bool, supportedByEvidence: bool }` — `status ∈ {approve, changes_required, reject, unavailable}`; `unavailable` requires `reason`.

`EvidenceReferenceView { eventDigest: Hex64, artifactDigest: Hex64, artifactKind: String, producedBy: String, producedAt: Option<String> }` — straight subset of contracts.rs `EvidenceReference`.

### 4.2 Rendering sources (no invention; direct mapping)

| Payload field | Authoritative source(s) | When unavailable |
|---|---|---|
| workflow.* | `WorkflowDefinition` identity + `source_digest_of(wf)` | `sourceDigest` can fail ⇒ envelope production fails (hard diagnostic) |
| reviewIssues / summary | `src/harness/review.rs::ReviewReport` (passed-in at call) | absent ⇒ `summary.passed: null`, `issues: []`, one `OmissionView{section:"issues", category:"unavailable", reason:"noAuthoritativeSource"}` |
| gates[] | `src/workflow/graph_gates.rs::HumanDecisionRecordV1` records gathered for the workflow run | absent ⇒ gates empty + `OmissionView{category:"unavailable", reason:"noAuthoritativeSource"}` |
| authority principals | `HumanDecisionRecordV1.decided_by`, ProvenanceEnvelope producer/kind | absent ⇒ `omissions` marker (never implied empty principal set means no author) |
| evidence refs | `EvidenceBundle`/`ProvenanceEnvelope`/`EvidenceReference` in the same record set | absent ⇒ `evidence_references: []` + `OmissionView{category:"unavailable"}` |

### 4.3 Deterministic ordering (MUST NOT round to input order)

- Issues sorted by `(severity rank: Critical>High>Medium>Low>Info (descending), issueType, file, line, ruleId, message)`. Ties in all fields ⇒ `SOMA-CMP-0011` duplicate-finding diagnostic (exact duplicate is a fault — ambiguity refused).
- Gates sorted by `(nodeId, verdict, failureClass, basisEvidenceDigest)` with same tie rules ⇒ `SOMA-CMP-0011`.
- `evidence_references` sorted by `(producedAt ?? "", eventDigest, artifactDigest)`.
- `omissions` sorted by `(section, category, reason)`.
- Maps use `BTreeMap<String, usize>` — JSON object keys lexical by canonical JSON rules already enforced at the envelope level.

### 4.4 MUST NOT claims (review-report)

The report MUST NOT claim or imply any of:
- success/absence from an unavailable source section;
- zero issues where the summary source was absent (`totalIssues: null` or reason-provided);
- an "approved" disposition absent a verdict record;
- any principal/actor identity when the authoritative record lacks one (render `absent`);
- any evidence exactly when none was available (`[]` paired with `OmissionView`).

### 4.5 Disclosure boundary behavior

- Disclosure replaces the payload with a **shape-preserving subset**: when a section/event/issue is not authorized, its list entry is removed AND an `OmissionView` entry is appended. The JSON document remains valid against the same payload schema; §7 parity checks apply.
- Per-count fields (`totalIssues`, `byType`, `bySeverity`, `passed`) are themselves disclosured counters: when underlying issues are withheld, their values reflect only disclosed+present data; additionally `omissions` carries the unavailable marker so counts never silently imply totals. A separate `countAuthorization`-aware variant (`SafetyMetrics`-style counters) is future work, out of scope.
- Names/rules allowed but not files/lines until disclosed: current policy decouples by `()` — that primitiveness makes it compatible with Slice 2's `GraphDisclosurePolicy` shape (§5.2).

---

## 5. Evidence-Timeline Projection (`lite.evidence-timeline.v1`)

### 5.1 Payload type

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineProjectionPayload {
    pub timeline_schema_version: String,     // "lite.evidence-timeline.v1"
    pub disclosure_policy_digest: String,
    pub scope: TimelineScope,                // run identity — mandatory or unavailable
    pub events: Vec<TimelineEventView>,      // deterministic total order (§5.4)
    pub omissions: Vec<OmissionView>,        // same vocabulary as review
    pub completeness: CompletenessView,      // (moreAvailable, nextAfter, authoritativeIds left)
}
```

`TimelineScope { runKey: Option<RunKey>, requestCorrelation: Option<String>, projected: Option<ProjectionPageMeta> }` — within-payload scope binds one run or one correlation; absent ⇒ empty timeline + OmissionView(category:"unavailable", reason:"scopeExcluded").

`CompletenessView` — mirrors Slice 1B `ProjectionPage` semantics (`more_available`, `next_after`), surfaces partial timelines honestly. When the authoritative source has more rows after the requested window, `moreAvailable` is `true` and the envelope remains valid; it is NOT a success document for the whole run.

### 5.2 Event view

```rust
TimelineEventView {
    event_id: String,                 // WorkEvent.id
    event_type: String,               // WorkEvent.event_type (mapped by RunKeyCategory if available)
    actor_kind: String, actor_identity: String,
    authority_class: String,          // effective_authority summary label
    sequence: u64,
    timestamp: Option<String>,        // declared AFTER ordering; only metadata
    correlation_id: String,
    repo_revision: String,
    semantic_digest: Hex64,           // authoritative event digest
    event_digest_reference: Vec<EvidenceReferenceView>, // payload.evidence carried through
    provenance: TimelineProvenanceView,  // summary subset or `unavailable`
    outcomes: Option<Vec<OutcomeVariant>>,  // from payload variants when typed
    status: String,                   // OutcomeVariant wire value when present; "unavailable" otherwise
    disclosure: EventDisclosureView,  // same shape as GraphDisclosureView
}
```

### 5.3 Provenance summary (no invention)

`TimelineProvenanceView { producer_kind: Option<String>, repo_binding: Option<RepoBindingLabel>, execution_class: Option<String>, refresh_state: String }` — each field either derived from `ProvenanceEnvelope` or rendered `"unavailable"` with reason; principals render `absent`/`redacted` per policy.

### 5.4 Deterministic ordering (the slice's hardest invariant)

- Event order key: `(sequence: u64, semanticDigest: Hex64)` compared as `(u64, lowercase hex string)` lexicographic tuple. `sequence` is the authoritative journal order for that run; `semantic_digest` is a stable fallback only when a run records duplicates.
- Same `(sequence, semanticDigest)` appearing more than once ⇒ duplicate identity diagnostic (`SOMA-CMP-0011`); the projection fails closed, never emits ambiguous order.
- Timestamps NEVER define order. `WorkEvent.timestamp` is recorded as metadata only (byte-exact as stored); equal timestamps on distinct events DO NOT require a secondary reorder — the sequence order decides.
- Cycles/self-parent pointer paths in `parents` MUST NOT induce recursion — parents are validated only as identifier references (`Vec<String>` membership in the page), never followed; inconsistent parent (missing from open run) ⇒ SOMA-CMP-0010 (invalid reference) pair with disclosure that the timeline is *incomplete* by page boundary (`moreAvailable` handles that case distinction).
- Map/struct ordering of payload fields is irrelevant to bytes — canonical JSON rules already fix the renderer.

### 5.5 MUST NOT claims (timeline)

- A projected "gap" (missing `sequence` numbers within a run) MUST surface as a `CompletenessView`/`OmissionView{category:"unavailable", reason:"sourceGap"}` — never silently dropped and never interpreted as fewer events.
- Legacy unverified provenance (`ProvenanceState::LegacyUnverified` rows) renders with `provenance.refresh_state: "legacy-unverified"`; never upgraded/coerced.

---

## 6. Disclosure & Withholding Semantics (Slice 2-consistent)

### 6.1 Slice-2-consistent policy shape (no new semantic axis)

- Reuse the `GraphDisclosurePolicy` field family shape: an ordered, validated allow-list of authorized disclosure targets plus a parallel count-authorization list. Slice 3 policy payload is `ReviewDisclosurePolicy`/`TimelineDisclosurePolicy` with the same wire properties (camelCase, `deny_unknown_fields`, `#[serde(default)]` lists, sorted/dedup/fail-closed validation). Targets: `principal/*` vectors, `files`, `lines`, `messages`, `predicates(` (review); `payloadDetails`, `actorIdentity`, `referenceDigest` (timeline). Unsupported target ⇒ PROJ-0003 family.
- Default disclosure state: all targets withheld; only the minimal payload (identity, counts shapes with `category: withheld`, omissions) is exposed. Authorization expands only by ADDING explicit policy entries, never by renderer heuristics.

### 6.2 Count-exposure vs existence-privacy

- Counts (`totalIssues`, `bySeverity`, `byType`, event counts per category) are independently sensitive: revealing counts is permitted only when `countAuthorization` covers the relevant target. An unauthorized count yields `null` value + `category: "withheld"` + omission marker.
- Existence-channels: payload keys that would reveal count/existence differences WERE reviewed — keys with no disclosed entries are rendered with explicit `kind: []`/`"value": null` + omission marker rather than silent omission (§4.4, §5.5).

### 6.3 Composite/private-boundary behavior

- Review: a finding group inside a private composite body maps to `GraphDisclosureView.category:"private-boundary"` reuse pattern — any cross-boundary evidence-reference whose source node path is withheld results in the finding being generalized to the owning composite scope (no private ids leak).
- Timeline: event whose disclosed payload references a private composite name surfaces the RunKey-kind only, not contained internals.

### 6.4 Non-cascading/narrowing invariant

- Narrowing only, never widening: a report/timeline rendered under PolicyA may not expose entries that PolicyA does not authorize. Applying a strictly-subsuming policy B (A ⊂ B) may expose MORE entries; applying a narrowing policy must never throw errors or coerce data — it reshapes `omissions`+counts only.
- `disclosurePolicyDigest` references the normalized policy used (§3.4 domains); envelope mismatch between `policyDigest` and re-normalization ⇒ `SOMA-CMP-0004`.

---

## 7. Diagnostics / Failure Taxonomy

Reuse existing families whenever an existing code means the required fault. New codes introduced ONLY in parentheses where no current code matches:

| Family / code (existing where possible) | Raised when |
|---|---|
| `PROJ-0001` | payload-byte parse failure; duplicate key; non-canonical re-render; envelope-level serde failure |
| `PROJ-0002` | identity/equality failure: `sourceDigest`/`projectionDigest` shape/render mismatch on fresh-render identity check |
| `SOMA-CMP-0001` | unsupported `projectionVersion`/`schemaVersion`/`graphSchemaVersion` |
| `SOMA-CMP-0003` | payload schema violation (unknown field in payload struct) |
| `SOMA-CMP-0004` | projectionDigest/sourceDigest/child-policy digest mismatch |
| (new) `SOMA-CMP-0010`  | invalid ordering/reference: cycle in parents, parent id missing from disclosed run scope, non-monotonic event sequence within a run |
| (new) `SOMA-CMP-0011`  | duplicate identity: same `(sequence, semanticDigest)` or duplicate findings entries |
| (new) `PROJ-0010` | disclosure/policy authorization expansion attempt (policy lists targeting unauthorized categories) |
| (new) `PROJ-0011` | invalid disclosure request (unknown target category; mix of counts authorized/not ordered behavior) |
| (new) `PROJ-0012` | review/evidence reference inconsistency: evidence_references with dangling event digest pair, gate basis pointer referencing undisclosed source |

All new codes must land in `diagnostics.json` category derivation first (matching SOMA `Diagnostic.category_for` convention) before being emitted. No new enum kinds around `Diagnostic`.

Verifiers per projection, both with exact ordered steps mirroring the graph pattern (§11 of Slice 2):

`verify_review_projection_bytes(raw)` steps: (1) find_duplicate_key scan; (2) serde parse to envelope; (3) allow-list (projectionVersion, schemaVersion); (4) metadata checks (digest shape per `verify_envelope_metadata`); (5) validate payload structure (deny_unknown_fields + enum values per §4.1); (6) shape-check of derived values: `disclosurePolicyDigest` shape, `source` fields non-empty/hex64 shape, digest shape of envelope projectionDigest; derived digests are shape-checked only. (7) `canonical_bytes` re-render equality of envelope. Failures: §7 table.

`verify_review_against_source(envelope, wf, policy)` steps: 1 envelope parse; 2 metadata bounds; 3 fresh-render of `CWF{policy}` via `render_review_projection(wf, policy)`; 4 `sourceDigest` equality vs `source_digest_of(wf)`; 5 `disclosurePolicyDigest` equality vs policy preimage digest; 6 derived-values compare: any `policyDigest` mismatch ⇒ `SOMA-CMP-0004` (NOT `PROJ-0002`); 7 fresh-render identity: envelope bytes from source material must equal envelope bytes supplied ⇒ `PROJ-0002`; 8 exhaustive: unknown policies fail validations above. **No remote/network/time-dependent steps.**

`verify_timeline_projection_bytes` / `verify_timeline_against_source` structurally identical, with step 6 comparing `disclosurePolicyDigest` + `eventStreamDigest`-derived values and step 7 re-rendering via `render_timeline_projection(batch, policy, scope)`.

---

## 8. Review Data Model (acceptance-class constraints)

Sliced into runtime decisions:
- Report rendering MUST be a pure function of `(WorkflowDefinition, ReviewReport|None, GateRecords|None, EvidenceBundle|None, policy)`. No HashMap iteration in any enumerated path (all `BTreeMap`, `Vec` sorted).
- Disposition: if no verdict record ⇒ `DispositionView.status = "unavailable"` + omission marker. If any `HumanVerdict::Rejected` among effective gates ⇒ `"reject"`; else if any `ChangesRequested` ⇒ `"changes_required"`; else if all available ⇒ `"approve"`; otherwise `"unavailable"`. Policy application does not override verdicts — in particular, withholding rendered gate entries requires that the final disposition block truthfully captures them, never *deriving* an apparent consent.
- Fail-closed on unknown source categories: a `FailureClass::Evidence` mapped gate renders as `gateKind` derived verbatim from the category string with its own label; no new "unknown category → success" branch.

## 9. Recon-driven edge cases folded into the spec

- `ReviewReport.passed` from `src/harness/review.rs` is a required field; only its absence (no passed report at all) triggers `unavailable`.
- `EvidenceBundle.final_state` text is authoritative; the report/timeline MUST NOT synthesize a `final_state` for a run missing a bundle — use `unavailable`.
- `WorkEvent.timestamp` raw-byte exact `created_at_text`; timeline must preserve raw rather than parsing it (back-compat with `WorkContextEvent.cursor`).
- `ProvenanceState::LegacyUnverified` is preserved as enum value `legacy-unverified`; upgrade attempt fails `§7 SOMA-CMP-0010`.
- `OutcomeVariant::ReviewRequired` correctly routes to `disposition.reviewRequired = true`; SUPER-influence: it never maps to `status: "approve"` at the report level.

---

## 10. RED → GREEN Acceptance Matrix (Slice 3)

Adapted from the §9 contract of Slice 2: the Slice 3 matrix MUST cover the Slice-2 matrix items 1-18 wherever conceptually applicable, plus the timeline-specific items. Implementation may reorder but MUST NOT silently drop any "required" row. Items 1-18 below are the normalized implementation matrix.

| # | Acceptance row | Expected evidence | Target test |
|---|---|---|---|
| 1 | Identical authoritative input → byte-identical review-report | same input, two renders | `review_render_is_deterministic_bytes` |
| 2 | Identical authoritative input → byte-identical evidence-timeline | same input, two renders | `timeline_render_is_deterministic_bytes` |
| 3 | insertion/JSON key order perturbation does not change bytes | same logical map, two entry orders | `canonical_render_ignores_input_key_order` (both projections) |
| 4 | deterministic timeline ordering under equal timestamps | events same timestamp, distinct sequences | `timeline_orders_by_sequence_then_digest_not_timestamp` |
| 5 | semantic source change alters the expected digests | changed workflow def changes `sourceDigest`; changed report payload changes `projectionDigest`; changed policy changes `disclosurePolicyDigest` | `digest_domain_separation_is_respected`, `policy_digest_shape_check` |
| 6 | projection tampering fails closed | flip a payload byte / envelope metadata byte | `verify_review_projection_bytes_rejects_tampering`, `verify_timeline_projection_bytes_rejects_tampering` |
| 7 | unsupported versions fail closed | `projectionVersion`/`schemaVersion` outside allow-list | `unsupported_projection_schema_version_fails`, `unsupported_projection_version_fails` |
| 8 | unknown fields / duplicate keys fail closed | dup key in envelope, unknown top key, unknown payload key | existing envelope semantics: `parse_envelope_bytes_rejects_*`; payload-level `deny_unknown_fields` rejection |
| 9 | projection cannot add authority | renderer ignores an invented `verdict: "approved"` source extension | `projection_cannot_widen_authority_or_invent_verdicts` |
| 10 | withheld information uninferable through authorized fields | disclosure applied; no removed bytes leak keys | `policy_drawn_report_carries_no_withheld_issue_content`, `timeline_payload_no_leak_of_withheld_payload_details` |
| 11 | withheld ≠ absent | withheld item produces OmissionView; absent source produces `unavailable` | `omissions_distinguish_withheld_unavailable_outOfScope` |
| 12 | composite/private boundaries preserved | finding inside withheld composite → parent scope only | `private_boundary_findings_are_generalized` |
| 13 | review findings reference evidence deterministically | same evidence_reference list through policy-equal render | `review_findings_evidence_references_sorted_and_stable` |
| 14 | dangling/inconsistent evidence references fail | evidence_references refers to a non-present event digest | `review_against_source_fails_on_dangling_evidence_reference` (SOMA-CMP-0004/PROJ-0012) |
| 15 | structural verifier behavior pinned per projection | fixed valid envelope bytes parse, fixed invalid bytes reject | `verify_review_projection_bytes_*` / `verify_timeline_projection_bytes_*` |
| 16 | against-source recomputation pinned per projection | fresh render identity vs supplied envelope | `verify_review_against_source_*` / `verify_timeline_against_source_*` |
| 17 | projection editing cannot mutate canonical/runtime authority | no public warp path from envelope to AST/DB mutation; public surface has no edit functions | compile-level: no exported function name matches *mutate*/save/persist; test pin |
| 18 | existing Slice-1/Slice-2 projection tests remain green | existing tests under `tests/projection_conformance_tests.rs` continue PASS | (gate) |
| 19 | counts without source render as unavailable | absent summary ⇒ `totalIssues: null`, omission marker, NOT zero | `summary_counts_are_absent_not_zero_when_source_absent` |
| 20 | occurrence check: same `(sequence, semantic_digest)` in one run ⇒ SOMA-CMP-0011 | crafted duplicate document fails | `duplicate_event_identity_is_hard_error` |
| 21 | `LegacyUnverified` provenance renders literally as `legacy-unverified` | feed provenance state variant | `legacy_provenance_state_is_preserved` |
| 22 | unknown gate verdict never upgraded | gate absent source; check rendering of `unavailable` | `unavailable_verdict_is_distinguishable_from_approved` |

## 11. Implementation Task Plan (TDD)

Tasks must be executed in dependency order; each task must be GREEN (cargo test local) before the next begins. Every task has explicit dependency tags. {All tests below are ``RED then GREEN`` under the standard gdb–lite protocol — first attempt is RED.}

### Task A — Review payload types + disclosure policy skeleton
Deps: none
Files: new `src/workflow/projection/review.rs`; register `pub mod review` in `src/workflow/projection/mod.rs`.
RED: `review_payload_struct_definition_exists_and_derives_serde` — compile error (module absent).
GREEN min: literal definitions per §4.1 (all fields `deny_unknown_fields`, camelCase, serde defaults where option-family).
Negative tests: unknown key at payload root; `reviewSchemaVersion` non-equal canonical string.
Invariant: no accessor/writer for downstream edit surfaces exists here.

### Task B — Review source gather + deterministic ordering (render)
Deps: A
Files: `src/workflow/projection/review.rs`; no model changes elsewhere.
RED: `render_review_projection_produces_deterministic_byte_layout`
GREEN min: `render_review_projection(wf: &WorkflowDefinition, facts: ReviewFacts, policy: Option<&ReviewDisclosurePolicy>) -> Result<VersionedProjectionEnvelope<ReviewProjectionPayload>, Vec<Diagnostic>>` with §4.3 total ordering; omission markers per §4.5; summary inheritance vs unavailability per §4.4.
Negative tests: tie-ordering of issues merge-sort; `passed: null` for absent summary; disposition=unavailable when no verdict records; gate order (nodeId, verdict, failureClass, basisEvidenceDigest); every `OmissionView` category exercises `withheld` vs `unavailable` vs `outOfScope`.
Invariant: pure function; input order cannot leak into output.

### Task C — Review structural + against-source verifiers
Deps: B
Files: `src/workflow/projection/review.rs`; wiring in `projection/mod.rs`.
RED: `verify_review_projection_bytes` / `verify_review_against_source` — functions absent.
GREEN min: `verify_review_projection_bytes(raw: &[u8])` steps per §7 (duplicate-key scan → strict envelope parse → version allow-lists → metadata shape checks only for derived values); `verify_review_against_source(envelope, wf, policy)` 8-step flow per §7.
Negative tests: tampered envelope byte footers teardown; wrong `sourceDigest` namespace compare; wrong `disclosurePolicyDigest` ⇒ `SOMA-CMP-0004`; unsupported schema version; non-canonical bytes; appended whitespace; missing `sourceDigest` field; extra envelope key.
Invariant: derived digest fields are shape-checked only on the bytes path; recompute applies only on the against-source path (steps 6–7).

### Task D — Timeline payload types + completeness skeleton
Deps: A (reuse `OmissionView` shape pattern)
Files: new `src/workflow/projection/timeline.rs`; `pub mod timeline` in `projection/mod.rs`.
RED: `timeline_payload_struct_definition_exists_and_derives_serde`.
GREEN min: literal definitions per §5.1/§5.2 (`TimelineScope`, `CompletenessView`, `TimelineEventView`, `DisclosureView` w/ `kind: GraphDisclosureView` fields only when policy allows).
Negative tests: deny-unknown-fields for scope/event; `moreAvailable` encoding; runKey parse failure path.
Invariant: `TimelineEventView` holds no synthesized `id` — values read-through from `WorkEvent`.

### Task E — Timeline render + deterministic ordering (§5.4)
Deps: D
Files: `src/workflow/projection/timeline.rs`.
RED: `render_timeline_projection_orders_by_sequence_then_semantic_digest_not_timestamp`.
GREEN min: `render_timeline_projection(batch: &WorkEventBatch, scope: &RunKey, policy: Option<&TimelineDisclosurePolicy>) -> Result<VersionedProjectionEnvelope<TimelineProjectionPayload>, Vec<Diagnostic>>`; page/page-boundary `CompletenessView` population; tie-break rules; duplicate identity; LegacyUnverified passthrough; group ordering stable.
Negative tests: same timestamp different sequences → sequence decides; reversed input event order yields same bytes; `parents` cycle does not hang; parent id missing ⇒ SOMA-CMP-0010; duplicate `(sequence, semanticDigest)` ⇒ SOMA-CMP-0011; crossing-run inclusion only within `scope: TimelineScope`.
Invariant: no recursion over `parents`; no `Instant::now` / `SystemTime::now`; no `HashMap` iteration over fields in any rendering loop.

### Task F — Timeline structural + against-source verifiers
Deps: E
Files: `src/workflow/projection/timeline.rs`; wiring in `projection/mod.rs`.
RED: following verifier identities absent.
GREEN min: `verify_timeline_projection_bytes` / `verify_timeline_against_source` mirror Slice-2 graph verifier structure; derived `disclosurePolicyDigest`/`eventStreamDigest` shape-checked on bytes path; rebased `..._against_source` recomputes them (§7/§3.4); identity/re-render path pins fresh-render equals supplied envelope.
Negative tests: envelope byte flip ⇒ PROJ-0001/SOMA-CMP-0004 split correctly; rewired `sourceDigest` namespace ⇒ PROJ-0002; policy-mismatched render ⇒ SOMA-CMP-0004; nonce/random salt path absent.

### Task G — Diagnostics registration + invalid-fixture set
Deps: C, F
Files: `vendored/soma/v1.1/diagnostics.json` (add `SOMA-CMP-0010`, `SOMA-CMP-0011`, `PROJ-0010/0011/0012` entries matching the categories+messages; `Diagnostic::category_for` must resolve each); new `tests/fixtures/slice3/*` invalid envelopes + valid goldens.
RED: diagnostic category resolution for the proposed code list fails; fixture parse expected-fail mismatch.
GREEN min: minimal schema for each new code; goldens: `valid/review-report*.json`, `valid/timeline*.json` (matching canonical renders), `invalid/*` cases per §10.
Negative tests: unknown code family in fixtures ⇒ no diagnostic; invalid category mapping ⇒ conservative fallback; goldens byte-locked per golden files semantics for Slice 2 (referenced).
Invariant: every new code appears in diagnostics.json and resolves via `category_for`; no uncategorized SOMA-CMP/PROJ codes.

### Task H — Review/Timeline fixtures + golden locks
Deps: C, F, G
Files: `tests/fixtures/slice3/valid/*.json`, `tests/fixtures/slice3/invalid/*.json`; new test file `tests/review_timeline_projection_conformance_tests.rs` (split optional: two files).
RED: fixture count mismatch; golden digest drift.
GREEN min: per-item rename/path staging, byte-identical goldens against renders, per §10 matrix items; both envelope parses go through existing `parse_envelope_bytes`-compatible flow.
Invariant: path mapping stable: no fixture path contains `TODO`.

### Task I — Envelope/re-render parity + no-op wire invariants (wholesale test sweep)
Deps: H
Files: `tests/projection_conformance_tests.rs` (extend only with a new test function for Slice-3 byte-parity — no edits to existing slices).
RED: cross-slice parity test absent ⇒ green on adding under new name only.
GREEN min: prove envelope canonical bytes of a Slice-1/2/3 envelope parse + re-render identically (reusing existing `try_canonical_bytes`); slice-3 fixtures are byte-stable against render inputs.
Invariant: no drive into Slice-1/2 golden files; no production code change needed for green.

### Task J — Doc/change-record completion (post-GREEN patch)
Deps: I
Files: `CHANGELOG.md` Slice-3 bullet; `specs/loop-engineering/changes/2026-10-XX-e4-x07-slice3-review-evidence-projections.md` new change record; optional stub of plan/task list snapshots in the referenced file.
RED: change-record file absent (as a worked artifact).
GREEN min: mirror Slice-2 change record skeleton.
Invariant: not part of the implementation GREEN gate; docs PR separate, same operator-reviewed path as `docs/post-241-e4-x07-slice2` PR #242 (docs-state node).

---

## 12. Final implementation gates (required by all later implementation candidates)

- G1: `cmd /c "cd /d E:\Projects\PrometheOS-Lite-e4x06-slice1 && set CARGO_TARGET_DIR=E:\Projects\PrometheOS-Lite\.cargo-target&& cargo fmt --all -- --check"`
- G2: same wrapper + `cargo clippy --offline --all-targets --all-features -- -D warnings`
- G3: same wrapper + `cargo test --offline --all-targets --all-features -- --test-threads 4`
- G4: same wrapper + `cargo test --offline --doc --all-features`
- Plus repository-owned exact-head: `python scripts/local_ci.py run --suite core|platform|smoke` → `python scripts/local_ci.py verify --commit <sha> .local-ci/evidence/<sha>/windows-{core,platform,smoke}.json`
- Evidence digests recorded AT the exact candidate head only; retry/deviations disclosed honestly. No hosted CI as authority.
- No absorption of #132/#217/#163/model-native/graph-projection semantics by this slice.

## 13. Self-Review Checklist (executed post-draft; documented honestly)

- [#243 acceptance vs spec] every #243 section mapped: recon(§2), shared envelope(§3),review(§4), timeline(§5), disclosure(§6), verifiers/diagnostics(§7, §9), RED→GREEN(§10), tasks(§11), gates(§12) — done in §1/§2 summaries.
- Every MUST has an acceptance row in §10 or explicit test target in §11 ✅.
- No canonical-authority duplication: review/timeline sections state WorkflowDefinition/AST authority is untouched ✅.
- No Slice-1/2 contract contradiction: envelope machinery, `source_digest_of` semantics, GraphDisclosurePolicy family semantics reused ✅.
- No graph/private-boundary regression: boundaries are generalized/non-cascading via §6.3/§6.4 ✅.
- No compiled-plan scope creep ✅; no #132/#217 scope creep ✅; no model-native ✅.
- No nondeterministic/timestamp-only ordering: §5.4 pins `(sequence, semanticDigest)` and forbids timestamp-as-key ✅.
- No ambiguous recompute contract: derived digest shape-check vs source-based recompute is strictly split (§3/§7/§16 matrix) ✅.
- No placeholder tasks in §11; every task has dependencies/files/RED/GREEN/negatives/invariants ✅ (Doc task J noted as docs-state only).
- Type/field names internally consistent throughout ✅ (deterministic ordering keys, field casings verified against recon findings).

## 14. Assumptions / Design Decisions Requiring Human Step Adjudication

1. `ReviewProjectionPayload` sources `ReviewReport` + `gate` records only for **existing authoritative data** and declares the rest unavailable. If downstream consumers expect review projections to include AST-derived "gate checks evaluation" fields that don't yet exist, follow-up scope (separate issue), not this slice.
2. Timeline event identity maps `WorkEvent.event_type` strings (open lowercase) — mapping categories `context/evidence/decision/lifecycle` from `src/work/soma_projection.rs:82` reused as-is; no new event categories introduced.
3. `ProvenanceEnvelope` is copied by reference summary only (principals → abstract) until a disclosure policy authorizes breakdown.
4. Diagnostic additions (`SOMA-CMP-0010/0011`, `PROJ-0010/0011/0012`) are proposals pending plan approval; if existing families suffice, they fold without new codes.
5. The `DispositionView` role list (`approve/changes_required/reject/unavailable`) deliberately excludes `approved`, matching Slice-2's point-in-name discipline; canonical "approved" string never invents consent.
6. Timestamp: `WorkEvent.timestamp` retained byte-exact as stored; never parsed/re-rendered as identity. Tie-breaker is `(sequence, semanticDigest)`. Any true chronology projection would require explicit new authoritative evidence — not invented here.

## 15. Summary Return Block (requested by directive)

- Exact baseline: `main@66d336212701a8e3ce89c3080bef94d71532db48` (post PR #242 merge #66d3362; local clone checked out on `docs/e4-x07-slice3-spec` branch from main tree).
- Drafted spec path: `docs/architecture/e4-x07-slice-3-review-evidence-projections.md`.
- Recon inventory: §2 table (source concept → type → projected → disclosure → digest binding).
- Acceptance-matrix count: 22 rows (+4 beyond the 18-item directive minimum).
- Implementation-task count: 10 (Tasks A–J).
- Design decisions requiring Human Step adjudication: §14 (6 items).
- Diff/stat: docs-only; adds this single file. `git diff --stat` = 1 file created; `git status` will show only the new spec as untracked → will be committed on the docs branch.
- Clean-tree status: spec artifact is the only pending change; no production files modified.
- Human-independent-review target: commit at `docs/e4-x07-slice3-spec` HEAD; review the whole doc for normative consistency. See the NEXT GRAPH EDGE in the directive for the review checklist.

**STOP. No production implementation.**
