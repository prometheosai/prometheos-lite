# E5/I04 — Security review, evidence audit, and independent review nodes

Issue: #128 (`[E5/I04] Implement security review, evidence audit, and
independent correctness review nodes`).
Status: implemented; merged via PR #207 (commit `6e11a72`) on
`main` on 2026-09-03.

## Goal

Complete the E5/I04 governed-review half of the node library: three
Lite-owned, deterministic, read-only nodes that produce a typed
`ReviewKind` verdict (`approve | changes-required | reject`) for a
candidate. Review is a classification; it NEVER produces an
"apply"/"merge"/"deploy" signal. Apply/merge remain operator/elevated
actions whose inputs are the emitted verdicts plus operator-supplied
human authority.

## What changed

### New module — `src/workflow/node_review.rs`

- Three capabilities registered through `review_registry()`:
  - `security-review` — pattern-based scan of the candidate for risky
    paths, dangerous commands, secret/credential exposure, new
    dependency introductions, and ref ↔ manifest content-digest
    mismatches. Deterministic, no model.
  - `evidence-audit` — fail-closed check that the expected durable
    evidence artifacts (worktree ref + content digest, change
    records, validation runs, prior review verdicts) are present and
    well-formed. Returns `reject` if any required artifact is missing
    or inconsistent.
  - `independent-review` — composes a single `approve | changes-
    required | reject` verdict from the upstream security and audit
    kinds. The most-severe wins; no escalation of upstream kinds.
- Typed contracts added:
  - `ReviewKind` — `approve | changes-required | reject` (kebab-case serde).
  - `SecurityFindingV1` — `category`, `severity`, `message`, optional
    `evidence`. `severity` is a string so new severities are
    backward-compatible.
  - `SecurityReviewResultV1` — `kind`, `findings: Vec<SecurityFindingV1>`,
    canonical `resultDigest`.
  - `EvidenceAuditFindingV1` — `kind`, `artifact`, `expected`,
    `observed`.
  - `EvidenceAuditResultV1` — `kind`, `missing: Vec<String>`,
    `inconsistencies: Vec<String>`, canonical `resultDigest`.
  - `IndependentReviewResultV1` — `kind`, `reasons: Vec<String>`,
    optional `securityDigest` + `auditDigest` (for evidence linkage),
    canonical `resultDigest`. Carries NO authorization field.
  - `SecurityReviewRequestV1`, `EvidenceAuditRequestV1`,
    `IndependentReviewRequestV1` — typed input contracts.
  - `IntroducedDependencyV1` — typed entry for the candidate's
    introduced dependencies.
- `review_node_manifest(node_id)` — builds a `NodeManifestV1` for
  each capability. All three manifests declare
  `writableScopes: []` (review is read-only with respect to the
  source repository and the journal).
- 25 in-module unit tests covering:
  - paths: absolute `repoRoot` required; empty `repoRoot` rejected.
  - commands: disallowed binary, `git` escape flags, `..` path-
    traversal, all flagged with the right severity.
  - dependencies: introduced dependency → `changes-required`.
  - secrets: PEM / OpenSSH / AWS / GitHub / Slack credentials, all
    → `critical` → `reject`.
  - Cargo.toml / Cargo.lock diff → `changes-required`.
  - authority: ref ↔ manifest content-digest mismatch → `critical` →
    `reject`; matching digests → `approve`.
  - evidence-audit: missing / malformed / unexpected malformed /
    well-formed all-present / empty `expected` (hard-fail).
  - independent-review: 9 composition pairs enumerated, all verified
    to produce the expected most-severe kind. Reasons always include
    "review does not authorize apply or merge".

### Module wiring — `src/workflow/mod.rs`

- Added `pub mod node_review;` (sibling of `node_validation`).

### Conformance — `tests/node_library_conformance.rs` (+5 tests)

Drives the three new capabilities through the generic `NodeRunner`
(same machinery as the E5/I01-I03 nodes). Tests prove:

- `security_review_rejects_critical_secret_in_candidate_diff` — a PEM
  private key in the candidate diff emits a critical `secrets`
  finding and forces `reject`.
- `security_review_flags_introduced_dependency_as_changes_required`
  — a new `tokio` 1.2.3 entry forces `changes-required`, not critical
  (operator must confirm).
- `security_review_rejects_git_command_with_external_git_dir` — a
  `git --git-dir <X>/.git --work-tree <X> rm <f>` command is rejected
  at plan time, before any process is spawned.
- `evidence_audit_rejects_when_expected_artifacts_missing` — when one
  of three expected artifacts is absent from `observed`, the audit
  returns `reject` and lists the missing entry.
- `independent_review_composes_security_and_audit` — the 6 non-
  symmetric composition pairs are enumerated; for every case the
  `reasons` field includes the literal phrase "review does not
  authorize apply or merge".

### Documentation — `CHANGELOG.md`

- Added an `## Unreleased` entry recording the E5/I04 work, the new
  module, the contract surface, and the test counts (980 lib + 18
  lib-conformance + 30 impl-conformance + 2 kit tests passing on
  this branch).

## Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0.
- `cargo test --lib -- --test-threads=4` — **980 passed**, 0 failed,
  1 ignored (was 955 before; +25 review unit tests).
- `cargo test --test node_library_conformance` — 18 passed, 0 failed
  (was 13 before; +5 review conformance tests).
- `cargo test --test node_implementation_conformance` — 30 passed,
  0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0 failed
  (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency changes).
- CI on PR #207: 13/13 green on the content head.

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent inspected the diff,
ran every verifier command, and walked the comparative control gate
bullet-by-bullet. Verdict: **APPROVE** with concrete evidence per
bullet. The reviewer noted that the minimality budget (200 net LOC
default) is exceeded (~1380 net LOC in the new module) but the new
module is cohesive and the conformance additions are necessary to
satisfy the conformance-suite acceptance bullet. Documented in the
PR body.

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all 1000
  baseline tests still pass.
- Stable alpha scope unchanged: review nodes are experimental
  capabilities, declared with `writableScopes: []`; they cannot
  write to the source repository or to the journal beyond the
  standard evidence retention.
- `prometheos work` behavior unchanged: this PR only adds
  `lite.node`-family capabilities consumed by the NodeRunner; it
  does not touch the workbench, intake, discovery, planning,
  implement, repair, validation, or diagnostic node surface.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry in
  `CHANGELOG.md`, which is the canonical record of recent work.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).

## Non-goals (explicit)

- No LLM-driven review. All three nodes are deterministic
  pattern/rule-based. Model-backed review is out of scope for this
  issue.
- No new `prometheos work` command or subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop. The review nodes only classify; they do
  not perform any state mutation.
- No new dependency. The implementation uses only existing
  infrastructure: `anyhow`, `serde`, `serde_json`, `sha2` (already
  in the tree).
- No benchmark, conformance-fixture, or external-pilot work. E7
  issues remain on the queue and are explicitly deferred.
