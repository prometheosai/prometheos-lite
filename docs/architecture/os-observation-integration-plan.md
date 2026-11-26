# OS observation boundary — Lite-owned implementation plan

Status: **plan only**. No OS adapter, no runtime execution, no protected
mutation, and no authenticated transport is implemented or authorized by this
document. Source-trust clearance for OS integration remains an owner/Principal
decision (see `docs/evidence/source-trust/candidate-4ce9529.json`).

## 1. Existing foundations (verified at candidate `4ce9529`)

| Capability | Where | What it already provides |
|---|---|---|
| Work-event model | `src/workflow/soma/event.rs`; vendored SPEC 006 fixtures | `WorkEvent` / `WorkEventBatch`, canonical bytes, `semanticDigest`, audited projection |
| Durable read projection | `src/workflow/projection/*`, #132 Slice 1B | journal-only source of truth; `ProvenanceState::Verified` read gate; rebuildable digest-locked artifacts |
| Cursorable observation | #132 Slice 2, `src/api/work_contexts.rs` | `GET /work-contexts/:id/work-events?after=&limit=`, `X-Next-Cursor` (always present), `X-More-Available`, ETags binding body + paging metadata |
| Capability negotiation | #132 Slice 3, `src/api/runtime_capabilities.rs` | `GET /runtime/capabilities` (frozen SPEC 007 `RuntimeCapabilitySet`); strict negotiation over requirements + authority pair; capability metadata is **never** authority |
| Isolation evidence shape | #348, `tests/isolation_evidence_contract.rs` | fail-closed, structural-only hosted-runner evidence contract; no promotion to authentication |

These are useful foundations but do **not** establish source trust, an
authenticated writer/reader identity, or an authenticated transport.

## 2. Gaps that block OS integration

1. No client enrollment: the experimental API identifies callers with a caller
   supplied `UserIdentityQuery` (`user_id`), not an authenticated principal.
2. No authenticated transport: no verifiable client credential, no expiry, no
   revocation, no per-client scope binding.
3. No explicit supported/unsupported version negotiation at the observation
   boundary (SPEC 007 negotiation is a separate, unauthenticated compute
   surface).
4. No denial taxonomy for malformed or unauthorized observation requests.
5. No documented reconnect/duplicate/missing-event contract for OS consumers.
6. No hard separation between observation and execution authority.

## 3. Target contract (versioned, read-only)

Wire contract: `prometheos.os-observation.v1`. Every response carries the
contract version and the candidate source revision it was produced from.

### 3.1 Authenticated client enrollment (gated)

- Enrollment issues a client-bound credential with: subject, owner binding from
  PR #340's trusted owner registry, `scopes = ["observe:work-events",
  "observe:capabilities"]`, `issued_at`, `expires_at`, `revoked = false`, and a
  monotonically increasing generation.
- Enrollment is an owner-controlled mutation. It is **not** implemented in this
  plan's eligible slice; it requires the PR #341 owner-registry acceptance and
  a clean replacement writer decision.
- Credentials are bearer tokens whose validity is checked on **every** request.
  No positive cache that can outlive revocation.

### 3.2 Read-only observation permissions

- An enrolled OS client may only call the GET routes enumerated in §3.3.
- Any request outside the observation route family is refused with a
  distinguished `UNAUTHORIZED_SCOPE` result. There is no route by which an
  observation credential can reach run, cancel, approve, patch, provision, or
  artifact-write handlers.
- The server derives authority from the enrolled scope set, never from request
  fields, headers, or the capability declaration.

### 3.3 Routes and version negotiation

- `GET /runtime/capabilities` — already present; when reached through the
  observation boundary it must additionally return `supported_versions` and
  `source_revision`. Capability metadata remains metadata only.
- `GET /work-contexts/:id/work-events?after=&limit=` — existing cursorable
  projection, reused verbatim; responses are canonical projection bytes, not
  wrappers.
- Version negotiation: the client sends `PrometheOS-Observation-Version`.
  - Supported version → `200` with the negotiated version echoed.
  - Unsupported or missing version → `400` with `supported_versions`; never a
    silent downgrade and never a default.
  - `source_revision` mismatch with the client's pinned candidate → `409`,
    not a partial stream.

### 3.4 Revocation and expiry

- `revoked = true` or `now >= expires_at` → `401` on every route, including
  in-flight cursor continuation. Revocation is permanent for that generation.
- A rotated generation invalidates the previous generation's outstanding
  cursors; a client must re-enroll and re-negotiate.

### 3.5 Malformed and unauthorized refusal

- Fail closed with strict parsing (`deny_unknown_fields` at every layer).
  Malformed cursors, unknown versions, bad limits, and unknown context kinds
  return `400` with a machine-readable reason; they never return an empty but
  apparently successful page.
- Un enrolled, unknown, revoked, expired, or wrong-scope clients return `401`
  or `403` without revealing whether a context exists.
- Journal read-gate failures return `500` carrying the gate's own detection
  text and no partial artifact (existing Slice 2 behaviour).

### 3.6 Connection loss, reconnection, duplicates, missing events

- The cursor is the last returned sequence number (`X-Next-Cursor` is always
  present, including empty pages). Reconnection resumes with `after=<cursor>`.
- Duplicate delivery: a replayed `after` returns the same byte-deterministic
  artifact and ETag; clients deduplicate by event sequence + semantic digest.
- Missing events: sequence gaps are a `409` integrity refusal, never a silent
  skip. A full re-projection from the journal is the only recovery path.
- Truncated/timed-out responses are retried with the caller's last durable
  cursor; no server-side streaming state is required.

### 3.7 No authority escalation

- Observation responses contain no tokens, capabilities-as-authority, write
  grants, or execution commands.
- The observation module must not import run/cancel/approve/patch/provision
  handlers. This is an enforced boundary, documented as an invariant and
  covered by a regression check before any transport is enabled.

## 4. Eligible Lite-owned slice (this dispatch)

Implementable without privileged acceptance or source-trust clearance:

1. A `prometheos.os-observation.v1` request/response **contract module** with
   strict parsing, version negotiation decisions, refusal taxonomy, and
   cursor/reconnect semantics — library-only, no server binding, no transport.
2. Positive/negative/failure tests for each §3 rule, including the
   no-escalation invariant and "reject if clearance is claimed".
3. This plan document.

Explicitly **not** in scope: any OS adapter, any authenticated transport
binding, any credential issuer, any execution or protected mutation, any
production deployment, and any claim that observation acceptance grants
source-trust clearance.

## 5. Gate before any transport implementation

- Source-trust evidence package validated at an exact candidate SHA and owner/
  Principal acceptance recorded (not self-granted).
- PR #341 Stage 2 Windows owner-registry acceptance completed on an authorized
  GitHub-hosted runner under separate approval; no physical-host effects.
- Authenticated enrollment and least-privilege replacement writer established
  by the owner.
- Exact-head repository-native verification: `cargo fmt --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and
  `python scripts/local_ci.py run --suite core|platform|smoke`.

## 6. Verification criteria for the eligible slice

- Supported version accepted; unsupported/missing version refused.
- Expired/revoked/wrong-scope credential refused on every route.
- Malformed cursor/kind/limit refused without a false empty success.
- Reconnect with last durable cursor → identical canonical bytes and ETag.
- Sequence gap → integrity refusal, never a skip.
- Observation module has no path to execution or protected mutation handlers.
- No test may set a READY/cleared state or claim Stage 2 acceptance.
