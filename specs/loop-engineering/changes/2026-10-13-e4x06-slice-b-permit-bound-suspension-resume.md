# Change: E4/X06 Slice B — permit-bound suspension/resume (typed outcomes wiring)

**Issue:** #163 · **Depends on:** #162 ✅ (merged #247), #160 ✅ (merged #246), #113/#123/#151 ✅.
**Builds on:** `src/workflow/graph_suspend.rs` (#162 contracts), `src/workflow/governance_permit.rs` (#163 Slice 1), `src/workflow/governance.rs` (compiled authority graph).

## Objective

Bind the durable suspension/resume contracts (X05) into the SEALED
governance permit so that only the reviewed plan + compiled authority
state may suspend or resume a governed run. A token issued under any
other plan identity or authority snapshot fails closed before the run
law executes; the permit-only dispatch boundary of Slice 1 is preserved.

TypedOutcome binding note (documented, not new semantics): outcome typing
is already compile-bound (serde enum + `SOMA-OUT-0001/0002` accept-set
audit over vendored shapes) and dispatch-bound (permit-only `NodeRunner`
+ journaled `RouteDecisionV1` on terminal categories). This slice adds NO
new outcome claims; it binds suspension/resume artifacts to the reviewed
plan, closing the remaining X06 runtime row without inventing runtime
`Skipped` routing semantics.

## Deliverables

1. **`CompiledAuthorityGraph::snapshot_digest()`** (`src/workflow/governance.rs`)
   — canonical digest over `{workflow_id, workflow_version,
   workflow_authority (ceiling), constraints, operations[{op_id,
   effective}]}`. Deterministic per compilation; a change in the declared
   authority or ceiling changes the digest.

2. **Permit-bound helpers** (`src/workflow/governance_permit.rs`):
   - `authority_snapshot_digest()` — pure getter over the compiled graph
     (recomputed on call; cheap and honest).
   - `suspend_run(state, manifest, id, trigger, operation_id,
     inputs_digest, now, revision?, expiration?)` — emits a vendored-shaped
     `SuspensionPointV1` bound to `plan_identity()` and this permit's
     authority snapshot via the X05 `graph_suspend::suspend_run`
     (write-boundary validation inherited: 64-lower-hex digests, RFC 3339
     expiry, terminated-run refusal). P1-B: fails closed before journaling
     unless the operation is a reviewed step of this permit, a node of
     the supplied manifest, on the run's live frontier, and the supplied
     manifest is the run's pinned graph revision.
   - `resume_run(checkpoint, suspension, token, manifest,
     portable_digest, repo_revision_now, now, replay?)` — delegates to
     `resume_with_token` with the permit's own identity, authority
     snapshot, and authorized steps. Any drift
     from the issuing permit fails closed (`SOMA-RES-0001`); tamper
     (`SOMA-RES-0003`), expiry (`SOMA-RES-0002`), and version
     (`SOMA-RES-0004`) ladder steps apply before the #123 run law takes
     over; replay authorization remains untouched. P1-B: the ladder
     revalidates the resumability operation against the resuming permit,
     the supplied manifest, and the imported run's live frontier
     (`SOMA-RES-0001` at `/resumability/operationId`).

3. **Conformance tests** (`tests/permit_suspension_binding.rs`, 18 tests):
   - deterministic snapshot digest across identical issuances;
   - permit-bound suspension binds plan identity + authority digest;
   - byte-deterministic suspension/token digests (injected clock);
   - suspend→resume round-trip under the issuing permit;
   - other-plan-identity permit refuses resume (`SOMA-RES-0001`);
   - changed-authority permit refuses resume (`SOMA-RES-0001`);
   - tampered suspension under the issuing permit refuses
     (`SOMA-RES-0003`);
   - tampered token (extended expiration, un-rebound) refuses
     (`SOMA-RES-0003`);
   - permit-bound resume preserves the #123 replay gate (refuses without
     authorization; resumes and journals with authorization);
   - P1-B creation refusals: operation unknown to the permit; operation
     absent from the supplied graph manifest; operation outside the live
     frontier; manifest that is not the run's pinned revision;
   - P1-B zero-mutation proof across every refusal class (no evidence
     journaled; seal and state digest byte-identical);
   - P1-B resume-side revalidation refusals over internally coherent
     rewritten-resumability artifacts: operation unknown to the resuming
     permit; operation absent from the supplied manifest; operation not
     on the imported run's live frontier; plus the plain (non-recohered)
     tamper that the revalidation catches before the journal digest
     check.

4. **Local gate wiring** (`scripts/local_ci.py`) — new check
   `permit suspension` → `cargo test --test permit_suspension_binding
   --quiet` in BOTH `SUITE_SPEC["core"]` and `rust_core()` (identical
   normalized commands per the #225 evidence contract).

## Slice B repairs (P1-A / P1-B) — completed in this candidate

Review history: the first candidate sealed the raw surface to
`pub(crate)` behind a crate-private `AuthorityBinding`. The independent
review of PR #252 accepted the reconstruction but blocked it on two P1
runtime-binding gaps, both repaired below: (1) a `pub(crate)` binding is
forgeable by ANY production module inside the crate; (2) a permit was
not bound to the operation it suspended.

### P1-A — in-crate permit-gated boundary

- The raw transitions `graph_suspend::suspend_run` and
  `graph_suspend::resume_with_token` REQUIRE a real `&GovernancePermit`
  and read the reviewed plan identity and compiled authority snapshot
  from it (the permit's fields are private; its only constructor is
  reviewed issuance). No transition accepts digest material at all, so
  no module — inside the crate or out — can forge plan/authority
  digests; the reviewer-blessed "internal primitive requires a real
  `&GovernancePermit`" shape.
- The forgeable surface was DELETED in toto: `AuthorityBinding` (type,
  `new`, `for_test`) and the `SuspendParams`/`ResumeCheckParams` input
  bundles no longer exist. The digest-parameterized cores
  (`suspend_run_bound`/`resume_with_token_bound`) are private to the
  `graph_suspend` module and exist only so the in-crate
  staleness-isolation controls can vary plan and authority digests
  independently; production transitions always enter through the
  permit-gated functions.
- In-crate architecture/policy regression
  `raw_transition_surface_stays_permit_gated_in_crate` (external
  compile-fail doctests cannot observe crate-internal visibility): reads
  the production source of both modules and fails if a forgeable binding
  constructor or a raw digest-bearing transition surface becomes
  production-visible again (no `AuthorityBinding`/`SuspendParams`/
  `ResumeCheckParams`, no fully-public transitions, the crate-visible
  transitions take `permit: &GovernancePermit` and no `plan_digest`/
  `authority_snapshot_digest` parameters, the digest cores stay
  module-private, the public permit surface stays public).
- `GovernancePermit::{suspend_run, resume_run}` remain the only
  externally reachable governed suspension/resume path; the standalone
  X05 ladder is preserved in-crate and exercised through that boundary.
- `issue_resume_token` stays public: it is a contract-level helper over
  an already permit-issued suspension document (it cannot transition a
  run, and it carries no caller-controlled digests — binding members come
  from the suspension the permit sealed).
- Four compile-fail doctests prove external callers cannot: invoke raw
  suspension; invoke raw resume; name the (removed) forgeable binding;
  bypass the reviewed permit by hand-assembling a `GovernancePermit`.
  They run under `cargo test --doc --all-features`, which is registered
  in BOTH authoritative evidence-runner specification points
  (`SUITE_SPEC["core"]` and `rust_core()` in `scripts/local_ci.py` — no
  gate additions needed).
- Honest surface statement: **no API/CLI suspension endpoint exists
  today**; the library permit boundary IS the externally reachable
  governed transition surface. No API/CLI integration is claimed.

### P1-B — operation binding (creation gates + resume revalidation)

- Creation (`GovernancePermit::suspend_run`, which now also takes the
  run's graph manifest) fails closed BEFORE any state mutation unless
  the suspension operation is:
  1. authorized by the permit — a step of its reviewed plan
     (`execution_graph().steps`);
  2. a node of the supplied graph manifest;
  3. on the run's live frontier (eligible to execute now — neither
     completed nor future-locked);
  and the supplied manifest is the exact graph revision the run pinned
  at open (`state.graph_manifest_digest`). A refused suspension
  journals nothing and leaves the run state byte-identical (proven for
  every refusal class by `refused_suspension_mutates_no_journal_or_state`).
- Resume revalidates the same binding inside the X05 ladder at the
  staleness stage, preserving the declared ladder precedence (version →
  tamper → expiry → staleness → run law): the resumability operation
  must be a step of the resuming permit and a node of the supplied
  manifest (before import), and after the imported state is proven
  unmoved, on its live frontier. Every refusal is `SOMA-RES-0001`
  pointing at `/resumability/operationId`, so a coherent legacy or
  rewritten-resumability artifact for a nonexistent or unrelated
  operation can never resume.
- Required negative regressions (integration, `permit_suspension_binding`):
  unknown-to-permit operation; operation absent from the graph manifest;
  operation outside the live frontier (creation + resume sides, with the
  specific message asserted per gate); tampered resumability operation
  (internally coherent artifacts whose journal is re-bound over the
  rewritten document — the exact shape pre-repair code could produce —
  plus the plain tamper the revalidation catches before the journal
  digest check); zero journal mutation after refused suspension.

### P1-B (first round) — isolated authority binding + snapshot digest pins

- In-crate control
  `authority_snapshot_only_drift_refuses_on_reviewed_plan_held_equal`
  (now driven through the module-private digest core with the real
  permit's authorized steps): a REAL reviewed `GovernancePermit` issues
  the suspension; plan identity
  is held EXACTLY equal to that permit while ONLY a different valid
  authority snapshot digest is presented. Before the refusal, the test
  proves the suspension, token, evidence journal, and checkpoint are
  internally coherent (exact one journal entry binding document +
  pre-suspension seal; byte-deterministic token; checkpoint round-trip
  reproducing the seal; positive control — the issuing permit resumes
  successfully). The drift refusal is exactly `SOMA-RES-0001` with
  diagnostic source `/authoritySnapshotDigest`, asserted NEVER
  `/planDigest` (the plan-only control symmetrically points at
  `/planDigest`).
- `CompiledAuthorityGraph::snapshot_digest()` now documents every bound
  member (`workflowId`, `workflowVersion`, `workflowAuthority` ceiling,
  `constraints` in order, ordered `operations[{opId, effective}]` with
  the complete effective profile) and WHY the retained `body` is excluded
  (body semantics are already sealed through `workflowDigest` and the
  reviewed plan identity `canonicalization.sha256`; every
  authority-relevant projection of the body IS bound).
- New sensitivity pins (in-crate): ceiling change, per-operation
  effective-authority members (looped over every operation),
  governance-constraint change, operation identity rename, and pure
  operation-order swap each move the digest; identical compilations
  digest equally (determinism pin).

## Acceptance mapping (#163)

| remaining #163 row | this slice |
|---|---|
| TypedOutcome / SuspensionPoint / ResumeToken runtime binding into sealed plan + permit | suspension + token bound to `plan_identity()` + authority snapshot; outcome typing documented as already compile/dispatch-bound |
| Deterministic suspension/restart/resume | byte-determinism test; round-trip under permit; replay gate preserved |
| Fail-closed bypass regressions | foreign-permit, authority-drift, tamper control rows; in-crate bypass policy regression; operation-binding creation + resume rows (unknown-to-permit, absent-from-manifest, outside-frontier, tampered resumability, zero journal mutation) |
| (pre-existing) runtime refuses plans whose digest differs from reviewed plan | unchanged: `permit_issue_requires_exact_reviewed_identity` etc. stay green |

## Out of scope

- Composite runtime dispatch (still typed refusal).
- Runtime `Skipped` routing semantics (declarative only).
- Vendor changes (vendored `soma/**` read-only), dependency changes.
- #217 untouched.

## Verification

- `cargo fmt --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo test` (full), focused suites:
  `graph_suspend_conformance` (20) and `permit_suspension_binding` (18).
  NOTE: there is no `graph_resume_conformance` test target anywhere in
  the repository (verified against `Cargo.toml` and all tracked files);
  resume-ladder coverage lives in `graph_suspend_conformance` (resume
  rows), the in-crate `graph_resume` module tests (8, under `--lib`),
  and `permit_suspension_binding`.
- Repository-owned gates at the exact candidate commit: `python3
  scripts/local_ci.py run --suite core`/`platform`/`smoke`, then
  `verify`; evidence attached to the PR.
- No GitHub Actions; no autonomous merge.
