# SPEC-0001 (DRAFT v0.3) — Unified Authority Resolution and Project Trust Boundary

**Lifecycle:** Post-control-gate repair + independent-review amendments. Normative
specification **not yet approved for promotion**. Implementation / merge / release
promotion remain **HOLD**. v0.2 addressed all seven findings (SPEC-P1-01…05,
SPEC-P2-01…02); v0.3 applies the four amendments required by the designated
independent reviewer (separate session) after digest verification.

Version history:
- v0.1 (2026-10-23) — initial draft, control-gate input.
- v0.2 (2026-10-23) — control-gate repair: canonical decision contract, vocabulary
  adapters, execution-boundary invariant, TOCTOU/revocation rules, Q1–Q6 resolutions,
  migration plan without broad compatibility grants, evidence-ledger requirements,
  seventeen-target acceptance matrix, mechanical call-site guard.
- v0.3 (2026-10-23) — independent-review amendments: header finding-range corrected
  (`SPEC-P1-01…05, SPEC-P2-01…02`); §3.1 `DenialEvidence` minimum field enumeration
  added; §9 platform-obligation list clarified to include test 5.

---

## 0. Evidence traceability (v0.1 → review → v0.2)

| Finding | Disposition | Repaired in |
|---|---|---|
| SPEC-P1-01 §5 lacks canonical grant/deny semantics | Repair | §3, §4 |
| SPEC-P1-02 Revocation and TOCTOU unspecified | Repair | §4.4–§4.5 |
| SPEC-P1-03 Default-write migration could preserve broad authority | Repair | §7.1 |
| SPEC-P1-04 Call-site completeness not mechanically established | Repair | §8, §7.3 |
| SPEC-P1-05 Evidence sampling could undermine accountability | Repair | §6 |
| SPEC-P2-01 Trust identity/storage binding | Repair (ADR required) | §5 + ADR-PT-001 |
| SPEC-P2-02 Credential allowlist vs process isolation | Clarify | §4.6 |

## 1. Purpose (unchanged in intent)

Establish one consistent authorization boundary before expanding execution surfaces
(editing, LSP adapters, parallel workers, plugin/config loading, debugging sessions).

**Consolidation, not a new subsystem:** the three existing vocabularies
(`ToolPolicy`, `PermissionLedger`, `TrustRegistry`) plus SOMA authority profiles become
**constraint adapters** into a canonical decision contract. They lose their role as
independent conceptual grants.

## 2. Verified current state (carried from v0.1, unchanged table omitted here)

The v0.1 §2 evidence table remains normative input and is not restated; the verified
facts are: three disjoint vocabularies with unequal call-site resolutions
(`src/tools/permissions.rs`, `src/harness/permissions.rs`, `src/tools/trust.rs`,
`src/flow/intelligence/tool.rs:452`, `src/tools/context.rs`); `PermissionLedger::with_defaults()`
write-open defaults; public `ToolPolicy::permissive()`; `GovernancePermit` exclusive
NodeRunner authorization; durable `lite.hdecision.v1` gate law; SOMA declared/effective
authority with widening check; `PathGuard` containment; OS-authenticated human principal.

**Addition (Q1 context):** no claim of an exploitable bypass is made; the write-open
default's *consumers* are an open inventory item (§7.1, step 1) — the migration is a
posture correction, not a verified incident.

## 3. Canonical decision contract (repairs P1-01)

### 3.1 The contract

```text
enum AuthorityDecision {
    Allow(ScopedGrant),
    Deny(DenialEvidence),
}
```

- **Allow and Deny are the only verdicts.** There is no "escalated" verdict in the
  contract: an escalation is a *separate request record* routed to the human-gate
  process (§4.7); it authorizes nothing by itself.
- Every decision carries a **decision identity** (deterministic id + digest over the
  full decision content) bound to the inputs that produced it.
- **DenialEvidence minimum shape.** A `Deny` carries, minimally: a typed deny `reason`
  (including at least `adapter_conflict`, `missing_adapter_input`,
  `resource_identity_unverifiable`), a `decision_ref` (decision id + digest, linking to
  the ledger record per §6), the principal/resource/scope identity of the refused
  effect, and the offending adapter set where an adapter conflict produced the denial.

### 3.2 ScopedGrant (what an Allow means)

An Allow may authorize only what the grant binds:

```text
struct ScopedGrant {
    principal: PrincipalId,                // authenticated principal (reuse human_principal / provenance classes)
    resource: CanonicalResource,           // canonical resource identity + the permitted operation on it
    operation: Operation,                  // the operation actually to be performed
    scope: ScopeChain,                     // project scope + workflow scope + fully qualified nested scope identity
    capabilities: CapabilityBounds,        // requested / declared / effective limits (three fields, effective ⊆ declared)
    permit: Option<PermitRef>,             // identity of the governing workflow permit, where required
    policy: PolicyContext,                 // policy version + trust epoch + revocation state at decision time
    decision: DecisionRef,                 // decision_id, decision_digest, evidence reference
    validity: GrantValidity,               // expires_at | single_use | revalidate_at_effect (mandatory default: revalidate at effect)
}
```

Binding rules:

- **G1.** A grant never authorizes an operation the governing workflow `GovernancePermit`
  prohibits; permit authority is an upper bound the resolver may only narrow.
- **G2.** Grants are single-use or bounded in time by default; a previously issued grant
  is never a reusable unconstrained capability token.
- **G3.** A grant binds the canonical resource identity at decision time; it does not
  bind "the path string".

### 3.3 Vocabulary adapters (legacy systems become inputs)

Each legacy system maps its native state into **constraint inputs** via deterministic,
pure adapters:

| Adapter | Source (verified) | Constraint semantics |
|---|---|---|
| `ToolPolicyAdapter` | `src/tools/permissions.rs` | Maps allowed permissions / require-approval / restricted write paths into capability constraints |
| `PermissionLedgerAdapter` | `src/harness/permissions.rs` | Grants/denials become allow/deny constraints on (permission, scope) |
| `TrustRegistryAdapter` | `src/tools/trust.rs` | Tool-source trust into preconditions (unknown source → deny, preserving its fail-closed behavior) |
| `SomaAuthorityAdapter` | `src/workflow/soma/profile.rs` | Declared vs effective authority; `authority_widened` check becomes authoritative here |
| `PermitAdapter` | `src/workflow/governance_permit.rs` | Governing permit reduces available capability bounds |
| `TrustStateAdapter` | project-trust state (§5, ADR-PT-001) | Untrusted/not-established project state denies adoption of project-sourced capabilities |

Composition rules (normative):

- **A1 Conjunction:** Allow requires *all* applicable adapters to constrain-allow; the
  effective capability bound is the **most restrictive** intersection.
- **A2 Conflict ⇒ Deny:** contradictory legacy policies produce a typed Deny carrying a
  conflict diagnostic (which adapters, which conflict). No silent resolution toward
  either side.
- **A3 Missing input ⇒ Deny:** any absent/unresolvable adapter input denies (typed
  reason), never partial-default.
- **A4 Adapters constrain, never grant:** an adapter cannot produce an allowance wider
  than its native state's most-restrictive reading. Adapters MUST NOT be implemented as
  "pick the union".
- **A5 Determinism:** identical context ⇒ identical decision digest; adapter order never
  changes the outcome.

### 3.4 Permit relationship (unchanged boundary)

`GovernancePermit` remains the **exclusive NodeRunner authorizer** with its protected
construction, private fields, no-builder law, and compile-fail proof, unchanged. This
contract does **not** create a second permit. The resolver sits downstream of permits at
workflow boundaries (narrow-only), and serves as primary authority at boundaries that
are not permit-gated (tools, spawn, API, CLI, memory/debug surfaces).

## 4. Execution-boundary invariant (repairs P1-02, P1-04)

> **EBI (normative).** Every consequential operation must pass an authority decision at
> its actual effect boundary. No adapter, cache, resumed workflow, alternate API route,
> or legacy policy constructor may bypass or broaden that decision.

### 4.1 Effect boundaries (enumerated)

| Boundary | Consequential operations | Enforcement point |
|---|---|---|
| Filesystem mutation | create/write/delete/rename/symlink | immediately before the effecting syscall/I/O call |
| Child process creation | any spawn | at spawn: `argv/env/cwd` restricted by the active grant; see §4.6 |
| Workflow effects | node/leaf effects in graph execution | downstream of `GovernancePermit`; permit-narrowed decision per effect |
| API operations | consequential REST/WS operations (generate through FlowExecutionService discipline) | at the route's effect boundary |
| CLI verbs | approval recording, state-changing verbs | at the effect |
| Credential use | provider calls with secrets | at the secret-restoration/proxy point (scoped, recorded) |

### 4.2 Time-of-check/time-of-use for filesystem effects

Normative requirement: the canonical resource identity established at decision time
MUST be **revalidated at effect time**, and the effect is refused if identity changed
(symlink swap, directory entry replacement, relocation).

Candidate primitives (implementation design decision, to be fixed in the implementation
plan; the invariant is authoritative, the primitive choice is not):

1. Anchor the parent directory handle before check, perform the mutation against a
   path resolved under that anchor; refuse if the canonical parent no longer matches.
2. Open-by-identity with reparse-point/symlink-identity verification where the platform
   offers it (compare open-handle identity vs decision's resource identity).
3. Verify-open then rename-into-place discipline for staged writes.

Where the platform cannot provide safe enforcement, the boundary is **fail-closed:
refuse the effect** with a typed denial (`resource_identity_unverifiable`).

### 4.3 Revocation

- **RV1.** Revocation events are durable records (like trust revocation, §5/ADR).
- **RV2.** A pending (authorized, not yet executed) effect whose grant reference is
  revoked MUST be refused at effect time; re-execution requires fresh resolution.
- **RV3.** Cached decisions never outlive their validity constraints; effect-time
  revalidation consults the revocation state (cheap digest/state check), not just the
  cache.

### 4.4 Concurrency

Decision→effect windows under parallel workers keep fully qualified nested scope
identities (existing composite-expansion semantics are preserved: every leaf keeps its
fully-qualified scope identity across execution/routing/evidence/suspension/checkpoint/
resume). A child scope can neither out-live nor out-wide its parent's authority chain
(accepted test: `nested_scope_cannot_widen_permit`).

### 4.5 Workflow resume

Resume re-proves authority from the identity snapshot (per existing graph_resume rules)
and re-resolves at operation boundaries; it never mints grants (carried R6; accepted
tests: `resume_never_expands_authority`, `checkpoint/resume` preservation).

### 4.6 Credentials and child environments (clarifies P2-02)

Two **separate** guarantees, explicitly distinguished:

1. **Environment inheritance restriction (this spec, enforced at spawn):** child
   processes receive an explicit environment allowlist by default; launcher-owned
   credentials are not implicitly inherited. This is enforced in the *spawn
   implementation* (actual `env` passed to the OS), not at command-proposal time, and
   not merely by filtering after the fact. Accepted tests:
   `credentials_excluded_from_children`, `windows_unix_spawn_env_parity`.
2. **Process/network/secret isolation (separate sandbox track, not claimed here):**
   OS-level confinement of the child remains the sandbox track's scope
   (`SandboxPolicy`/runtime). This spec does not claim isolation; it requires only that
   the resolver never *increase* authority in unsandboxed combinations (carried from
   v0.1 §2 note).

Explicit per-operation credential grants (an operation legitimately needing a
credential) are individually recorded and surfaced in approval display text.

### 4.7 Escalation

An escalation is a **request record**, not a decision: it identifies the denied-or-
preventive context, and routes to the human-gate process (existing durable
`lite.hdecision.v1` law). After a human approval is recorded, authority is resolved
**again**:

```text
escalation_request → (human gate) → fresh resolve_authority(context') -> Allow|Deny
```

The human approval is a *new input context* (and new evidence), **never** an implicit
grant, and never converts a previously denied decision into an authorization (accepted
test: `approval_escalation_requires_fresh_resolution`).

## 5. Project trust (repairs P2-01)

Trust semantics (loading-time precondition, never authority — carried from v0.1 §4):
project trust is granted by the human principal to a project scope, is revocable, and
authorizes nothing by itself. Trust state is a **resolver input** (`TrustStateAdapter`)
that can only reduce authority.

**Identity and storage:** specified in **ADR-PT-001 (DRAFT)** — see
`ADR-0002-project-trust-persistence-v0.1.md` in this package (identity tuple
`principal + canonical project identity + project boundary + policy version + grant
epoch`; attribution; creation/revocation state; relocation/replacement/principal-change
non-transfer rules; storage reuse of existing durable infrastructure where suitable
without conflating trust with permission grants).

Binding consequences (normative):

- **T1.** Trust revocation or change invalidates previously issued decisions that
  depended on it (accepted test: `trust_change_invalidates_stale_decision`).
- **T2.** Trust for one project scope never transfers across relocation, directory
  replacement, or principal change without fresh authorization.
- **T3.** Classes of project-sourced configuration being trusted are explicit in the
  record (e.g., flow definitions, skills, hooks), enforced at loading boundaries.

## 6. Authorization evidence requirements (repairs P1-05)

- **E1 Append-only decision ledger.** Every consequential authorization decision —
  Allow **and** Deny — is durably recorded at its effect boundary, in bounded-size
  records with deterministic identifiers (digest-derived), and attributable provenance.
- **E2 No sampling of authoritative decisions.** Sampling/aggregation is permitted only
  for derived telemetry (counts, latency, repeat non-authoritative diagnostics).
  A security audit must be able to account for all individual decisions.
- **E3 Fail-closed evidence.** If the durable evidence store cannot accept a decision
  record, the consequential operation MUST NOT execute (accepted test:
  `evidence_failure_blocks_effect`). The system never silently discards denials to
  keep running; attempt-failures land in non-authoritative telemetry and surface as
  typed failures.
- **E4 Sanitization.** Decision payloads (Allow details and DenialEvidence) are
  surface-sanitized; credential-shaped values are redacted (carried R8; accepted test:
  `denial_evidence_sanitized`).
- **E5 Views, not rewrites.** Aggregated/compressed views may exist for humans; the
  authoritative ledger is append-only and is never rewritten or compacted in place.

## 7. Migration plan (repairs P1-03; resolves Q1, Q2)

### 7.1 Q1 — `PermissionLedger::with_defaults()` (DECIDED: approve with conditions)

Binding decision: *tighten `with_defaults()`; do not preserve project-wide Write in any
new workbench factory merely for compatibility.*

Migration sequence (normative order):

1. **Inventory:** every constructor call site (`with_defaults`, `create_restrictive_ledger`,
   `PermissionLedger::new` + `grant`), classified production / test / fixture /
   experimental, with the actual effects authorized at each site.
2. **Classify effects:** which sites genuinely authorize source-repository Write and
   why; workbench stable-alpha operations (scan, artifact production, approval
   recording) must NOT acquire source-modification authority.
3. **Restrict:** introduce the tightened default; explicit, scope-limited grants only
   where a site's classification demands them (e.g., artifact output directory).
4. **Prove golden path:** stable-alpha workbench flow passes unchanged (accepted test:
   `golden_path_unchanged`).
5. **Prove rejects:** unapproved source writes remain impossible (accepted tests:
   `workbench_defaults_no_source_write`, `unknown_project_state_fails_closed`).

Exception regime: a legacy workflow that genuinely requires a broader grant receives an
**explicit migration exception** with an expiration/removal condition recorded where
the exception lives; **no invisible compatibility grant**.

### 7.2 Q2 — `ToolPolicy::permissive()` (DECIDED: restricted migration)

Binding decision: *production paths cannot instantiate permissive authority; test-only
access preferred; any compatibility exception is explicit, bounded, audited, separately
approved.*

- `permissive()` becomes unavailable in ordinary production builds (test-only fixture or
  `#[cfg(test)]` visibility).
- A runtime flag or environment variable alone is **not** a security boundary (an
  untrusted project can influence both); a development-only capability, if ever needed,
  requires a narrowly defined activation boundary distinct from project-controlled
  inputs, with the activation itself an evidence-recorded decision.

### 7.3 Q4/Mechanical guard — call-site completeness (repairs P1-04)

Mechanical enforcement (moved forward from §8; DECIDED mandatory):

- **MG1 Constructor allowlist:** an allowlist of permitted authorization constructors
  and enforcement entry points, reviewed like pinned digests (existing repository
  pattern: `scripts/reviewed_config_digests.json`); a static check fails verification
  when a new usage appears that is not on the reviewed list.
- **MG2 Resolver coverage manifest:** enumerated consequential entry points map to the
  canonical resolver; the map is checked mechanically so adding a call site without
  resolver wiring fails verification.
- **MG3 Vocabulary creation guard:** introduction of a fourth authority-vocabulary type
  (new Permission/Policy/Trust enum in authority paths) fails the policy check unless
  the reviewed surface list is updated by review.
- Guards live in the repository-native verification system when promoted (core/policy
  suites); no hosted CI.

### 7.4 Q3, Q5, Q6 (decisions recorded)

- **Q3** — DECIDED: ADR required → ADR-PT-001 drafted (§5); storage must be durable,
  revocable, human-attributable; reuse existing storage infrastructure where suitable;
  trust records never conflate with permission grants.
- **Q5** — DECIDED: `FlowExecutionService` boundary preserved as required regression
  boundary; existing API semantics, security tests, verification infrastructure
  unchanged; no hosted CI or dependency additions without separate approval
  (SAFETY_GATE interplay).
- **Q6** — DECIDED (mandatory): both platforms need **actual execution evidence** for
  platform-dependent behavior; no Linux-only graduation of Windows behavior, no
  theoretical cross-platform claims. Test-class distinction is normative in §9.

## 8. Verification plan (repairs P1-04 + Q5/Q6)

Test-class discipline (normative for suite assignment):

| Class | Description | Evidence form |
|---|---|---|
| C1 Policy invariants | Platform-independent resolver/policy logic | Unit/property tests, any platform |
| C2 Process behavior | Spawn, env inheritance | **Real spawn tests, both platforms** (`--require-platform windows` + linux) |
| C3 Filesystem identity | Symlink/path resolution behaviors | **Real fs tests, both platforms** |
| C4 Recovery/durability | Suspension, resume, evidence durability | Platform suite patterns, both platforms for spawn/fs-dependent cases |

A passing policy unit test never substitutes for a real process-spawn or filesystem
test where the behavior is platform-specific.

## 9. Acceptance matrix (final; 17 targets)

Original seven (v0.1 §6) retained; ten mandatory additions from the control gate.
Suite = repository-native verification (`core`/`platform`/`smoke`/policy); Platform =
C1–C4 class requiring real executable evidence.

| # | Test | Proves | Suite | Class |
|---|---|---|---|---|
| 1 | `unknown_project_state_fails_closed` | Untrusted/unknown project state → typed denials, no execution | core | C1 |
| 2 | `no_escalation_via_config_hooks_workers_aliases` | Config/hooks/aliases/worker surfaces cannot widen beyond declared authority | core | C1 |
| 3 | `decision_binding_fields_complete` | Every decision binds principal+scope+operation+resource (+permit/policy/epoch) | core | C1 |
| 4 | `credentials_excluded_from_children` | Child env = explicit allowlist only; no implicit launcher credentials | platform | **C2 both platforms** |
| 5 | `resume_never_expands_authority` | Resume/checkpoint re-proves authority; no grant minting | core + platform | C4 |
| 6 | `denial_evidence_sanitized` | Deny records exist, credential-shaped values redacted | core | C1 |
| 7 | `golden_path_unchanged` | Stable-alpha workbench flow unchanged | smoke + core | C1 |
| 8 | `conflicting_policy_adapters_deny` | Contradictory legacy policies ⇒ Deny with conflict diagnostic; no broader union | core | C1 |
| 9 | `revocation_invalidates_pending_operation` | Revoked grant blocks a pending-not-yet-executed effect | core | C1 |
| 10 | `resource_identity_swap_fails_closed` | Symlink/resource substitution cannot redirect an authorized mutation | platform | **C3 both platforms** |
| 11 | `nested_scope_cannot_widen_permit` | Child execution stays inside fully qualified parent authority scope | core | C1 |
| 12 | `legacy_callsite_cannot_bypass_resolver` | Enumerated consequential entry points all hit canonical authorization | core (static+runtime) | C1 |
| 13 | `evidence_failure_blocks_effect` | Effect blocked when durable decision record cannot be written | core + platform | C4 |
| 14 | `trust_change_invalidates_stale_decision` | Revoked/replaced project trust cannot validate stale decisions | core | C1 |
| 15 | `approval_escalation_requires_fresh_resolution` | Human approval never converts a denial into an implicit grant | core | C1 |
| 16 | `workbench_defaults_no_source_write` | Compatibility defaults do not permit source modification | smoke + core | C1 |
| 17 | `windows_unix_spawn_env_parity` | Both platforms enforce identical child-environment restrictions | platform | **C2 both platforms** |

Platform-evidence obligations: tests 4, 10, 13, 17 carry explicit per-platform evidence
requirements (`--require-platform windows|linux` patterns; unavailable platforms
recorded in review, never silently passed), preserving the standing Windows/Unix
obligations. Test 5 (`resume_never_expands_authority`) is likewise included in the
per-platform obligation set: its resume path is filesystem-dependent (existence and
content of the checkpoint state), so its `core + platform` execution requires real
evidence on both platforms for the fs-dependent segments, on the same terms as tests
4, 10, 13, 17 for the segments that touch real spawn/fs behavior.

## 10. Non-goals / boundaries (v0.2)

- No new authorization subsystem beyond the canonical contract + adapters (the contract
  *is* the consolidation).
- `GovernancePermit`, human-gate law, review states, verification suites, `PathGuard`,
  provenance contracts: unchanged and protected.
- No implementation in this pass; no merge; no release promotion; no product-repo
  changes yet.
- No hosted CI additions; no dependency additions without separate approval.
- No product-repo references to competitive research.
- #132, #217, pending independent reviews, Windows/Unix verification obligations:
  untouched.
- OS-level sandboxing is a separate track; this spec adds no isolation claims.

## 11. Remaining risks and unresolved implementation-level questions (explicit)

1. **Windows TOCTOU primitive selection** — which §4.2 candidate is implementable on
   Windows with the AV/available primitives; final design belongs to the
   implementation plan (fail-closed refusals remain the fallback invariant). Known
   environment hazard: the standing Windows AV-vs-`.git/objects` flake; filesystem
   identity tests must tolerate the documented AV interference protocol.
2. **Call-site inventory completeness** — the §7.1 inventory and MG2 manifest require a
   repository survey this specification does not yet contain (the resolver-drift sites
   verified are `flow/intelligence/tool.rs:452`, `tools/context.rs`, harness ledger
   consumers; the survey must be exhaustive before implementation).
3. **Trust identity without VCS origin** — canonical project identity derivation for
   non-repository workspaces is an open ADR point (ADR-PT-001 §Open).
4. **Evidence ledger placement/scale** — where the append-only ledger lives (existing
   SQLite `RunDb`-class storage vs new store) is an implementation decision; the
   E1–E5 requirements are fixed regardless.
5. **Escalation UX wiring** — routing escalation requests into the existing human-gate
   CLI (`work decide` family) needs a UX/API design pass; the semantics here are
   complete, the ergonomics are not.
6. **Permit-adapter interface stability** — how the permit exposes narrowing metadata
   to the resolver without expanding its public surface (compile-fail law preserved)
   is an implementation unknown.
7. **Revalidation cost** — effect-time revalidation latency on hot paths (harness
   attempt pools) needs measurement in the benchmark plan before any performance
   claims; no claims made.

## 12. Promotion path staged by the control gate

v0.2 → independent re-review → (on approval) documentation-only promotion candidate
through the Loop Engineering Protocol (product-repo SPEC + change record, lineage
section dropped, competitor references absent) → implementation authorization remains a
separate later gate.
