# Source-trust evidence package — candidate `4ce9529`

Human-readable rendering of
`docs/evidence/source-trust/candidate-4ce9529.json`. This package is a
**documentary, structural** record. It does not authenticate a publisher,
establish credential containment, grant source-trust clearance, grant runtime
authority, or authorize a merge. `SECURITY_CLEARANCE: NOT SELF-GRANTED`.

**Historical/current separation (S5):** this package is immutably pinned to
candidate `4ce9529` (tree `306a453a`). At repair time current `main` was
`770e68f5bb1163fdcf61c84f98b72a394840d390`. The evidence and the recorded
artifact digest apply **only** to the historical candidate and are not a
statement about current HEAD or any new build. No source-trust clearance is
granted for either revision.

## Identity and scope

| Field | Value |
|---|---|
| PROJECT | `PROMETHEOS_LITE_ONLY` |
| CANDIDATE_SOURCE_SHA | `4ce9529a7b0f1a08084e0de1c7a1440e1de90e26` |
| CANDIDATE_TREE_SHA | `306a453a64a769c4fc4d156802e22de8ef3c91ea` |
| BASE_MAIN_SHA | `4ce9529a7b0f1a08084e0de1c7a1440e1de90e26` |

The candidate is the current protected `main` tip at reconciliation time. It
was selected because it is the reviewed, protected baseline that carries the
repository-native verification and security gates — not because a clean build
or a clean-looking tree proves trust.

## Source and ancestry

| Field | Verdict | Basis |
|---|---|---|
| SOURCE_ANCESTRY_VERDICT | VERIFIED | `git merge-base --is-ancestor` returned non-zero for `6d6fb679`, `14b8378` and `468017b`; no suspect commit reachable from candidate |
| KNOWN_SUSPECT_CONTENT_IN_CANDIDATE | VERIFIED | `frontend/postcss.config.js` blob `33ad091d26d8a9dc95ebdf616e217d985ec215b8` (82 bytes) — the independently recorded trusted blob; payload-shaped blobs `6f626195…` and `069bcc5d…` excluded |
| KNOWN_SUSPECT_ANCESTRY | VERIFIED | same ancestry enumeration; `observations.suspect_commits_in_ancestry_empty = true` |
| DEPENDENCY_AND_BUILD_INPUT_REVIEW | VERIFIED (scoped) | tracked executable-configuration pins reviewed (`scripts/reviewed_config_digests.json`), one reviewed workflow only (`contents: read`, no secrets), no dependency lifecycle hooks in `frontend/package.json`, no new dependency introduced by this work |

Residual: the lifecycle-script assertion is a narrow, read-only cross-check
against the tracked `frontend/package.json` (scripts: `build`, `dev`, `lint`,
`start`; no lifecycle hooks). It is **not** supply-chain verification: full
independent transitive provenance (SBOM/signature) remains UNKNOWN. GitHub
reported 10 open Dependabot advisories on the default branch (1 high, 6
moderate, 3 low) at PR publication time; dependency changes require explicit
owner approval and were not made.

The validator independently enumerates the real `.github/workflows` directory
and refuses symlinks, non-file entries, unreadable inventories and unexpected
workflow files. Git ancestry/tree/blob queries fail closed: a missing or
unreadable Git fact is an explicit finding, never an empty clean history.

## Credential and push-actor evidence

| Field | Status | Meaning |
|---|---|---|
| AUTHENTICATED_PUSHING_ACTOR | BLOCKED | No authenticated audit-log attribution exists in accessible evidence. Git author text `PrometheOS Agent <agent@prometheos.ai>` is editable metadata, not pusher proof. |
| WRITER_ISOLATION | BLOCKED | No verified implicated writer identity; nothing isolated. |
| CREDENTIAL_ROTATION | BLOCKED | No credential value handled, rotated or revoked. |
| EXPOSURE_REVIEW | BLOCKED | No audit/workflow/artifact access sufficient to determine whether payload executed. |
| ADMINISTRATOR_RECEIPT | NOT_RUN | No sanitized administrative receipt produced. |

Issue #297 disposition: **archived at owner direction** (2026-11-20). The owner
attributed historic writes to a ChatGPT-connected connection; this is owner
testimony, not authenticated attribution. Per the archival instruction, this
package does **not** reopen that investigation. Reopening or accepting its
residual risk is a separate owner choice. Closing #297 was not security
clearance.

## Build and test

| Field | Value |
|---|---|
| BUILD_COMMAND | `python scripts/local_ci.py run --suite core` |
| BUILD_INPUTS | `Cargo.lock`, `Cargo.toml`, `requirements-local-ci.txt`, `frontend/package-lock.json`, `frontend/postcss.config.js`, `.github/workflows/pr341-windows-acceptance.yml`, `scripts/reviewed_config_digests.json` |
| BUILD_ARTIFACT_SHA256 | `27a8c5b8388b0fd01be71d475a8c94665e08f8a025b08203f1f23ea0388de156` |
| BUILD_ENVIRONMENT | Windows local host, cargo/rustup, Python 3.14.10, `.cargo-target` target-dir; no paid/external service |
| BUILD_REPRODUCIBILITY | NOT_RUN (one build only; byte-identity to an independently trusted release artifact not established) |
| TEST_RESULTS | PARTIAL — see below |

**Actually run at candidate `4ce9529`:** `cargo fmt --all -- --check` exit 0;
`cargo clippy --all-targets --all-features -- -D warnings` exit 0;
`cargo test --doc --all-features` 4 passed / 0 failed;
`cargo build --release --all-features` exit 0 (separate rerun, 28m04s) producing
the digest above.

**NOT RUN:** `cargo test --all-targets --all-features`, the remaining focused
core checks, and local_ci `platform`/`smoke`. The repository-native core run
was terminated by the 3600-second harness limit during its release-build check
and emitted no evidence record. No full-suite pass is claimed.

A SHA-256 digest proves byte identity relative to the build that produced it.
It does **not** prove publisher identity or reproducibility against an
independently trusted reference. No signature or attestation is asserted here.

## Verdict fields

| Field | Status |
|---|---|
| SOURCE_INTEGRITY_STATUS | BLOCKED |
| SECURITY_CLEARANCE | BLOCKED — **NOT_SELF_GRANTED** (owner/Principal decision) |
| RUNTIME_AUTHORITY | BLOCKED — NOT_GRANTED |
| PRINCIPAL_REVIEW_REQUIRED | true |

## Automated negative controls

`tests/source_trust_evidence.rs` validates this package and rejects: known
suspect artifacts; mismatched source/build-revision binding; workflow permission
expansion; extra build hooks/workflows; dependency lifecycle scripts; missing
provenance; incorrect artifact digests; unattributed clearance; quarantined
revisions; conflicting clearance/exclusion rules; false READY states; and
runtime-acceptance claims without a runtime test. None of these checks grants
clearance.

## Owner-action checklist (administrator-only, NOT performed here)

1. From a trusted org/account admin session, attribute the two suspect pushes
   (`6d6fb679`, `14b8378`) from authenticated audit records; do not infer from
   Git author strings.
2. Identify the actual writing application/user/token from audit + local agent
   credential records.
3. Isolate **only** the confirmed implicated writer.
4. Rotate or revoke **only** credentials demonstrated implicated or reasonably
   exposed, under owner authorization; preserve the current connected OAuth
   grant unless it is actually implicated.
5. Review Actions jobs, artifact access and outbound/network logs; decide any
   downstream secret rotation from actual exposure.
6. Decide disposition of suspect refs and the contaminated `v1.7.0` tag without
   destroying preserved forensic bundles/hashes.
7. Establish a verified clean least-privilege replacement writer.
8. Post a sanitized, attributable administrative receipt with exact identifiers
   and timestamps.

If the owner elects to keep #297 archived, record that decision explicitly as
acceptance of residual risk; do not treat archival as remediation.

Exact dispatch input for the authorized, configured manual Windows acceptance
workflow (not dispatched here): workflow
`.github/workflows/pr341-windows-acceptance.yml` on `main@981c3995` (merged via
#346), `workflow_dispatch` only, `contents: read`, pinned checkout action
`11bd7190…`, exact source `37a7ae33…`, no secrets. It is supplemental unit /
identity-separation evidence only; Stage 2 privileged acceptance remains NOT
RUN. No physical-host, BIOS, reboot, or protected ProgramData effect was
performed.
