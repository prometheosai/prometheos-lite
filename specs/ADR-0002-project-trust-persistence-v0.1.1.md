# ADR-PT-001 (DRAFT v0.1.1) — Project Trust Persistence

**Status:** Proposed (draft for SPEC-0001 §5; Q3 resolved as "ADR required").
**Scope:** repository normative record. **No implementation authorized.**

Revision history:
- v0.1 (2026-10-23) — initial draft, reviewed alongside SPEC-0001 v0.2.
- v0.1.1 (2026-10-23) — independent-review amendment: `TrustGrantV1` gains a
  `validity` field so §4's "grants can expire independently of trust" is represented
  in the schema, aligning with SPEC-0001 §3.2 grant validity.

---

## Context

SPEC-0001 v0.2 makes project trust a loading-time precondition — a resolver input that
can only reduce authority. Trust must be durable, revocable, human-attributable, and
**not conflated with permission grants**. This ADR specifies the trust record's
identity and lifecycle. Storage choice is deliberately narrowed but not fully decided
(see Open points).

## Decision (normative requirements)

### 1. Trust record identity (minimum tuple)

```text
TrustGrantV1 {
  principal        PrincipalId        // OS-authenticated human principal (reuse existing identity path)
  project_identity CanonicalProjectId // canonical, non-path-derived project identity (see §2)
  boundary         ProjectBoundary    // the top-level boundary class (e.g., repository root, config dir, workspace)
  policy_version   PolicyVersion      // trust policy version that produced/validated the grant
  grant_epoch      GrantEpoch         // monotonic grant generation for the project identity
  trusted_classes  Vec<ConfigClass>   // the specific classes of project-sourced configuration trusted
  attribution      Attribution        // who granted (principal + channel), creation time, review context
  revocation       RevocationState    // active | revoked { time, by, reason }
  validity         Validity           // expires_at | unbounded; expired grants fail
                                      // their trust precondition without revocation,
                                      // aligning with SPEC-0001 §3.2 (GrantValidity)
}
```

### 2. Canonical project identity (relocation/replacement semantics)

Canonical project identity MUST be derived from durable, non-path properties, and MUST
NOT automatically transfer under the three forbidden transitions:

| Transition | Behavior |
|---|---|
| Directory replaced (content/config swapped at same path) | Trust does NOT transfer; stale epoch detected (baseline binding bound at grant time); fresh authorization required |
| Repository relocated (moved/detached to a different identity) | Trust does NOT auto-follow; grant remains bound to the original identity; relocation presents a stale-identity decision requiring explicit re-grant |
| Principal change | Trust is principal-bound; a different principal starts from the untrusted state |

Identity derivation (working proposal, to be fixed in design):

```text
CanonicalProjectId = digest( canonical origin identity if present  (VCS remote canonical URL fingerprint)
                             ||  fallback: workspace-root content binding )
                     + digest( boundary normalization rules version )   // version-bound, so rule changes invalidate old grants
```

Detection obligations at load time:

- **D1 stale path:** the boundary content binding recorded at grant time no longer
  matches ⇒ grant is invalid (not merely suspicious).
- **D2 relocation/origin drift:** recorded identity ≠ presented identity ⇒ deny + typed
  diagnostic; re-grant is a new human decision.
- **D3 re-vouching:** re-authorization always creates a **new grant epoch**; epochs are
  never reused; a fresh epoch invalidates decisions that depended on the prior one
  (SPEC-0001 test `trust_change_invalidates_stale_decision`).

### 3. Attribution and revocation

- Grants are human-attributable records; machine actors cannot author them (mirrors the
  gate law's actor-unrepresentability approach).
- Revocation is a durable mutation: `revocation: revoked { time, by, reason }`; the
  grant itself is never deleted (append/annotate discipline — supports the authority
  ledger's audit requirements without rewriting history).
- Revocation (or epoch change) invalidates previously issued authority decisions that
  depended on the grant (carried from SPEC-0001 §5 T1).

### 4. Storage

Requirements (binding): durable, crash-safe, revocable without deletion, attributed,
enumerable for audit, and project-scope separable. Working proposal: reuse the existing
durable record infrastructure used for human gate decisions / WorkContext persistence
rather than a new store, **provided** the trust record keeps a distinct type and never
inherits grant semantics from permission stores. Final placement is an implementation
decision with these requirements as acceptance constraints.

Trust records are storage-separate from permission grants: one can revoke trust
entirely while grants persist (they will simply fail trust-preconditioned resolutions);
and grants can expire independently of trust (time-bounded expiry is carried by the
`validity` field of `TrustGrantV1` §1: an expired grant fails its trust precondition
without needing a revocation record, while revocation remains the explicit durable
mutation for human-initiated withdrawal).

## Open points (for design review)

1. Derivation for **non-VCS workspaces** (no origin): fallback binding quality and its
   revocation consequences need a design pass.
2. Storage placement and schema versioning details (record type, indices, migration).
3. Presentation of trust state in the human UI (which classes, how revocation appears,
   how re-grant prompts display epoch/fresh state) — UX design, not specified here.
4. Interaction with repo binding states already recorded by provenance machinery
   (`Bound | Dirty | Unbound` detection existing in `src/work/provenance.rs`) —
   candidate to reuse binding signals; needs explicit design mapping.

## Consequences

- Adds a durable, auditable precondition layer that cannot be silently re-obtained
  across the three forbidden transitions.
- Costs: an additional record type + a load-time verification step; grant friction per
  project/class (deliberate).

## Traceability

- Control-gate Q3 decision + ADR request (reinforced by SPEC-P2-01).
- SPEC-0001 v0.2 §5 (T1–T3) consumes this ADR as its trust-state specification source.
