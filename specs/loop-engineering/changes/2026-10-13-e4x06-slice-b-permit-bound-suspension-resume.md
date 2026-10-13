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
  `graph_suspend_conformance` (21) and `permit_suspension_binding` (9).
- Repository-owned gates at the exact candidate commit: `python3
  scripts/local_ci.py run --suite core`/`platform`/`smoke`, then
  `verify`; evidence attached to the PR.
- No GitHub Actions; no autonomous merge.
