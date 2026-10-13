# Change: E4/X05 Slice 1 — typed suspension points + resume tokens (durable, fail-closed)

**Issue:** #162 · **Depends on:** #159 ✅, #113 ✅, #123 ✅, #151 ✅ (all closed). Vendored `soma/v1.1` bundle pinned, read-only.
**Builds on:** `src/workflow/graph_state.rs` (durable state law), `src/workflow/graph_resume.rs` (#123 resume/cancel gates), `src/workflow/memory_contracts.rs` (evidence references), `src/workflow/soma/{canonical,mod}.rs` (canonical digests, diagnostics, remediation).

## Objective

Give review-gated (human) or operator-requested graph runs a FIRST-CLASS durable
suspension that does not hold a live worker, plus a resume token contract that
fails closed on expiry, tamper, staleness, or version drift — mapping the
vendored `spec/soma/v1.1` `SuspensionPoint` / `ResumeToken` shapes without
inventing semantics. The existing #123 run law stays intact: a token never
bypasses checkpoint integrity, stale repo revision, or replay authorization.

## Deliverables

1. **`src/workflow/graph_suspend.rs`** — new module:
   - `SuspensionPointV1` / `ResumeTokenV1` mirroring the vendored v1.1
     schemas exactly (`camelCase`, `deny_unknown_fields`): suspension
     `{id, schemaVersion, workflowVersion, revision?, provenance{trigger},
     expiration?, stateDigest, planDigest, authoritySnapshotDigest,
     resumability{operationId, inputsDigest}}`; token `{id,
     suspensionPointId, suspensionDigest, workflowVersion, schemaVersion,
     authoritySnapshotDigest, planDigest, bindingDigest, revision?,
     expiration?}`. `bindingDigest` = canonical digest over the binding
     tuple (`suspensionPointId`, `suspensionDigest`, `planDigest`,
     `authoritySnapshotDigest`, `workflowVersion`, `schemaVersion`,
     `revision`), self-excluded.
   - `suspend_run(state, params)`: refuses a terminated run; validates
     strict `MAJOR.MINOR.PATCH` and RFC 3339 expiry; seals state; journals
     a suspension evidence entry (`artifactKind = "suspension-point"`,
     `eventDigest = digest(suspension doc)`, `artifactDigest = the
     pre-journal sealed state digest the suspension binds`); reseals so the
     exported checkpoint round-trips unchanged through import.
   - `issue_resume_token(suspension, id, expiration?)`: computes
     `suspensionDigest` and `bindingDigest`; expiry validated as RFC 3339.
   - `resume_with_token(checkpoint, suspension, token, params)` — the
     fail-closed ladder, in declared order (first failure wins):
     1. version gate (`SOMA-RES-0004`) — contract versions must equal
        `1.0.0`;
     2. token↔suspension alignment + token self-binding recompute
        (`SOMA-RES-0003`);
     3. expiry gate (`SOMA-RES-0002`) — suspended-point and token
        expirations checked against an injected RFC 3339 clock;
     4. staleness (`SOMA-RES-0001`) — resume-time `planDigestNow`,
        `authoritySnapshotDigestNow`, and the run-state digest must equal
        the suspension's bindings. The state binding additionally requires
        the imported checkpoint to carry EXACTLY ONE suspension journal
        entry whose `eventDigest == digest(suspension doc)` and
        `artifactDigest == suspension.stateDigest`; the pre-suspension
        digest is reconstructed by removing exactly that entry.
     5. delegation to `graph_resume::resume_run` — stale repo revision,
        replay authorization, termination all still gate independently.

3.1. **Review repairs (independent review of head `bf8ba57`, 4×P1):**
   - **Expiration is cryptographically bound.** `bindingDigest` covers the
     binding tuple `{suspensionPointId, suspensionDigest, planDigest,
     authoritySnapshotDigest, workflowVersion, schemaVersion, revision,
     expiration}` with absence explicit (empty string): removing or
     extending `expiration` invalidates the binding and refuses with
     `SOMA-RES-0003`.
   - **Pre-suspension checkpoints refused.** The staleness step now
     REQUIRES exactly one matching suspension journal entry and validates
     its event/artifact digests before reconstructing the pre-suspension
     digest; a missing, duplicated, or tampered entry fails with
     `SOMA-RES-0001`.
   - **Typed digest validation at the write boundary.** `suspend_run`
     refuses malformed `planDigest` / `authoritySnapshotDigest` /
     `inputsDigest` (must be `^[0-9a-f]{64}$`, the vendored pattern)
     before any document is created.
   - **One version policy.** Creation, issuance, and resume all support
     exactly `SUSPENSION_SCHEMA_VERSION` (`1.0.0`): `suspend_run` and
     `issue_resume_token` refuse unsupported workflow/contract versions
     instead of creating documents the resume ladder could never accept.
   - New regression controls: expiration removal/extension invalidates
     binding; pre-suspension checkpoint substitution, tampered/duplicated
     journal entries, malformed digest fields, and
     create-then-resume version consistency; plus overlapping-invalidity
     tests proving the declared ladder precedence (version → tamper →
     expiry → staleness).
     Token refusals surface as `Vec<Diagnostic>` with the vendored
     `SOMA-RES-*` codes plus remediation via the existing enrichment;
     delegated run-law refusals surface as the underlying typed error.
   - `ResumeWithTokenError::{Refusal, Underlying}` keeps the two failure
     families distinct (honest typing; no invented codes on run-law errors).

2. **`src/workflow/mod.rs`** — register `pub mod graph_suspend;` (1 line).

3. **`tests/graph_suspend_conformance.rs`** — 21 conformance tests:
   - positive: valid suspend → journal → resume round-trip; vendored wire
     shape (member set, camelCase, unknown-member refusal); deterministic
     byte-identical suspension/token digests for identical inputs;
     `digest()` equals the canonical digest of the document.
   - negative (fail-closed): tampered token binding; token minted for a
     different suspension; expired token (and a pre-expiry acceptance
     control); unsupported contract version; stale plan digest; stale
     authority snapshot digest; checkpoint moved since suspension; suspend
     of a terminated run refuses.
   - integration: a valid token still passes through the replay
     authorization gate (refuses without `ReplayAuthorization`, resumes
     with it) — suspension tokens never bypass the #123 law.

4. **Local gate wiring** (`scripts/local_ci.py`) — new core check
   `graph suspension` → `cargo test --test graph_suspend_conformance
   --quiet` in BOTH `SUITE_SPEC["core"]` and `rust_core()`
   (identical normalized command, per the #225 evidence contract).

## Acceptance mapping (#162)

| #162 acceptance | This slice |
|---|---|
| Human review suspends without holding a live worker | `suspend_run` + journaled suspension evidence (worker released; run persists) |
| Compatible provider/harness replacement can resume deterministically | binding includes authority snapshot + plan digests; replacement provenance stays on #123 `implementationChanges`; determinism test pins byte-identity |
| Expired, stale, or incompatible resume tokens fail closed | ladder steps 1/3/4 (`SOMA-RES-0002/0001/0004`) with negatives |
| Outcome and suspension events are journaled and replayable | suspension journal entry travels in `evidenceRefs`; replay via checkpoint round-trip test |
| (pre-existing, verified) accepted outcome variants; failure/skip cannot coerce to success; deterministic compilation | unchanged: `SOMA-OUT-0001/0002` audit + runtime routing (documented in the #162 reconciliation comment) |

## Out of scope

- #163 X06 runtime-binding slice that consumes these contracts inside the
  governance permit (returns to #163 after #162).
- Runtime `Skipped` outcome routing (Lite has no skip-routing semantics;
  coverage is declarative at the SOMA contract/audit level — documented).
- Vendored `soma/**` changes (read-only) and any dependency changes.
- #217 untouched.

## Verification

- `cargo fmt --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo test --all-targets --all-features --
  --test-threads 4` (full suite green), `python
  scripts/test_local_ci_verify.py`.
- Repository-owned gates at the exact candidate commit: `python3
  scripts/local_ci.py run --suite core`, `--suite platform`, `--suite
  smoke`, then `verify`; evidence attached to the PR.
- No GitHub Actions, hosted runners, paid services, or third-party status
  applications; no autonomous merge.
