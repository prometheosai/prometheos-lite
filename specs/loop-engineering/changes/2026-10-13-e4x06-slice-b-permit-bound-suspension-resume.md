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
   - `suspend_run(state, id, trigger, operation_id, inputs_digest, now,
     revision?, expiration?)` — emits a vendored-shaped
     `SuspensionPointV1` bound to `plan_identity()` and this permit's
     authority snapshot via the X05 `graph_suspend::suspend_run`
     (write-boundary validation inherited: 64-lower-hex digests, RFC 3339
     expiry, terminated-run refusal).
   - `resume_run(checkpoint, suspension, token, manifest,
     portable_digest, repo_revision_now, now, replay?)` — delegates to
     `resume_with_token` with `planDigestNow = plan_identity()` and
     `authoritySnapshotDigestNow = authority_snapshot_digest()`. Any drift
     from the issuing permit fails closed (`SOMA-RES-0001`); tamper
     (`SOMA-RES-0003`), expiry (`SOMA-RES-0002`), and version
     (`SOMA-RES-0004`) ladder steps apply before the #123 run law takes
     over; replay authorization remains untouched.

3. **Conformance tests** (`tests/permit_suspension_binding.rs`, 9 tests):
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
     authorization; resumes and journals with authorization).

4. **Local gate wiring** (`scripts/local_ci.py`) — new check
   `permit suspension` → `cargo test --test permit_suspension_binding
   --quiet` in BOTH `SUITE_SPEC["core"]` and `rust_core()` (identical
   normalized commands per the #225 evidence contract).

## Slice B repairs (P1-A / P1-B) — completed in this candidate

### P1-A — permit-only governed boundary

- Sealed to `pub(crate)` (was `pub`): `graph_suspend::suspend_run`,
  `graph_suspend::resume_with_token`, `graph_suspend::AuthorityBinding`
  (type), `graph_suspend::SuspendParams`,
  `graph_suspend::ResumeCheckParams`, and the `AuthorityBinding::new`
  constructor. The `#[cfg(test)]` `for_test` constructor is additionally
  `#[cfg(test)]`-gated, so it never exists in production builds.
- `GovernancePermit::{suspend_run, resume_run}` are therefore the ONLY
  externally reachable governed suspension/resume path; the standalone
  X05 implementation is preserved in-crate and exercised through that
  boundary (integration tests already drive the permit surface).
- `issue_resume_token` stays public: it is a contract-level helper over
  an already permit-issued suspension document (it cannot transition a
  run, and it carries no caller-controlled digests — binding members come
  from the suspension the permit sealed).
- Four compile-fail doctests prove external callers cannot: invoke raw
  suspension; invoke raw resume; construct a caller-controlled
  `AuthorityBinding`; bypass the reviewed permit by hand-assembling a
  `GovernancePermit`. They run under `cargo test --doc --all-features`,
  which is registered in BOTH authoritative evidence-runner
  specification points (`SUITE_SPEC["core"]` and `rust_core()` in
  `scripts/local_ci.py` — no gate additions needed).
- Honest surface statement: **no API/CLI suspension endpoint exists
  today**; the library permit boundary IS the externally reachable
  governed transition surface. No API/CLI integration is claimed.

### P1-B — isolated authority binding + snapshot digest pins

- New in-crate control
  `authority_snapshot_only_drift_refuses_on_reviewed_plan_held_equal`:
  a REAL reviewed `GovernancePermit` issues the suspension; plan identity
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
| Fail-closed bypass regressions | foreign-permit, authority-drift, tamper control rows |
| (pre-existing) runtime refuses plans whose digest differs from reviewed plan | unchanged: `permit_issue_requires_exact_reviewed_identity` etc. stay green |

## Out of scope

- Composite runtime dispatch (still typed refusal).
- Runtime `Skipped` routing semantics (declarative only).
- Vendor changes (vendored `soma/**` read-only), dependency changes.
- #217 untouched.

## Verification

- `cargo fmt --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo test` (full), focused suites:
  `graph_suspend_conformance` (20) and `permit_suspension_binding` (9).
  NOTE: there is no `graph_resume_conformance` test target anywhere in
  the repository (verified against `Cargo.toml` and all tracked files);
  resume-ladder coverage lives in `graph_suspend_conformance` (resume
  rows), the in-crate `graph_resume` module tests (8, under `--lib`),
  and `permit_suspension_binding`.
- Repository-owned gates at the exact candidate commit: `python3
  scripts/local_ci.py run --suite core`/`platform`/`smoke`, then
  `verify`; evidence attached to the PR.
- No GitHub Actions; no autonomous merge.
