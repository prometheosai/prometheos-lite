# E5/I05 — Documentation-impact and release-preparation nodes

Issue: #129 (`[E5/I05] Implement documentation and release-preparation
nodes`).
Status: implemented; merged via PR #208 (commit `fa305e0`) on
`main` on 2026-09-03.

## Goal

Complete the E5/I05 doc-and-release half of the node library: two
Lite-owned, deterministic, read-only nodes that produce typed
artifacts. `doc-impact` is a read-only classifier (it never writes a
file). `release-prep` is a document producer — it never publishes,
tags, pushes, or merges; merge remains an operator action whose
inputs are the emitted artifacts plus operator-supplied human
authority.

## What changed

### New module — `src/workflow/node_doc_release.rs`

- Two capabilities registered through `doc_release_registry()`:
  - `doc-impact` — pattern-based classifier that maps the candidate's
    changed paths to doc zones (user-guide, architecture,
    api-reference, tutorial, changelog, readme) and emits
    `DocFindingV1` records with `severity = required` (CHANGELOG /
    README) or `severity = recommended` (other zones). The node
    NEVER writes a file; its `constraints` field asserts the read-
    only invariant.
  - `release-prep` — composes a `ReleasePrepResultV1` from upstream
    evidence digests (implementation, validation, audit,
    independent-review, doc-impact). The result carries:
    - `sections: Vec<ReleaseSectionV1>` (Summary, Implementation,
      Validation, Review, Documentation, Change Set), each citing
      the upstream evidence digest.
    - `approvals: Vec<ApprovalRequirementV1>` enumerating the
      operator / security / reviewer / doc sign-offs required
      BEFORE any release action.
    - `cited_digests: Vec<String>` (every digest in input order).
    - `constraints: Vec<String>` asserting no file writes, no git
      operations, no network calls, and that "the operator must
      perform the actual merge, tag, and publish".
    - The struct carries NO field that authorizes publish/merge/
      deploy. The conformance test asserts the wire-format
      absence of `authorized`, `apply`, `merge`, `publish`, `tag`,
      `push`, `deploy`.
- Typed contracts added:
  - `DocCategoryV1` — `user-guide | architecture | api-reference |
    tutorial | changelog | readme | none` (kebab-case serde).
  - `DocFindingV1` — `category`, optional `path`, `message`,
    `severity`.
  - `DocImpactResultV1` — `kind`, `findings: Vec<DocFindingV1>`,
    canonical `resultDigest`.
  - `ReleaseSectionV1` — `heading`, `body`, `evidenceDigests`.
  - `ApprovalRequirementV1` — `role`, `reason`, optional
    `evidenceDigest`.
  - `ReleasePrepResultV1` — `assumedVerdict`, `sections`,
    `approvals`, `citedDigests`, canonical `resultDigest`.
  - `DocImpactRequestV1`, `ReleasePrepRequestV1` — typed input
    contracts.
- `doc_release_node_manifest(node_id, capability)` — builds a
  `NodeManifestV1` for each capability. Both manifests declare
  `writableScopes: []` (the nodes are read-only with respect to the
  source repository and produce no side effects on the journal
  beyond the standard evidence retention).
- 18 in-module unit tests covering: `repoRoot` validation, the
  CHANGELOG / README → `required` mapping, the user-guide →
  `recommended` mapping, diff-line inference, the
  `alreadyAddressed` operator annotation path, the section
  composition for every combination of supplied digests, the
  approval enumeration when `changeSet` contains a `required`
  finding, the `assumedVerdict = ChangesRequired` default when
  omitted, the wire-format absence of every authorization key, the
  read-only invariant, and the registry's capability
  declarations.

### Module wiring — `src/workflow/mod.rs`

- Added `pub mod node_doc_release;` (sibling of `node_review`).

### Conformance — `tests/node_library_conformance.rs` (+3 tests)

Drives the two new capabilities through the generic `NodeRunner`
(same machinery as the E5/I01-I04 nodes). Tests prove:

- `doc_impact_flags_changelog_change_as_changes_required` — a
  CHANGELOG.md change emits a `required`-severity finding, the
  verdict is `changes-required`, and the `constraints` field
  carries "read-only".
- `doc_impact_classifies_user_guide_change_as_recommended_approve`
  — a `docs/guides/...` change emits a `recommended`-severity
  finding, the verdict is `approve`.
- `release_prep_includes_approvals_and_no_authorization_field` —
  the artifact includes all five supplied digests in
  `citedDigests`, the operator / security / reviewer / doc
  approvals are present when required, and the wire format has
  none of `authorized`, `apply`, `merge`, `publish`, `tag`,
  `push`, `deploy` keys.

### Documentation — `CHANGELOG.md`

- Added an `## Unreleased` entry recording the E5/I05 work, the
  new module, the contract surface, and the test counts (998
  lib + 21 lib-conformance + 30 impl-conformance + 2 kit
  tests passing on this branch).

## Composition with E5/I04

`release-prep` accepts the upstream `auditDigest` and
`independentReviewDigest` fields that the E5/I04 review family
(merged via PR #207) produces. The two task families compose:
an operator can chain `security-review` → `evidence-audit` →
`independent-review` → `release-prep` to produce a single
release artifact that cites every upstream evidence digest. The
`release_prep_includes_approvals_and_no_authorization_field`
conformance test exercises this composition when all five
digests are supplied.

## Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` —
  exit 0.
- `cargo test --lib -- --test-threads=4` — **998 passed**, 0
  failed, 1 ignored (was 980 before; +18 doc-release unit tests).
- `cargo test --test node_library_conformance` — 21 passed,
  0 failed (was 18 before; +3 doc-release conformance tests).
- `cargo test --test node_implementation_conformance` — 30
  passed, 0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0
  failed (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency
  changes).
- CI on PR #208: 13/13 green on the content head.

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent was dispatched
but the harness returned a stub (rate-limited). The implementing
agent fell back to a documented inline review with the same
comparative control gate recorded in the PR body, per the
operator-mandated reviewer protocol in
`specs/active/autonomous-e5-e6/QUEUE.md`. The inline review
inspected every file in the diff, ran every verifier command,
and walked the comparative control gate bullet-by-bullet.
Verdict: **APPROVE** with concrete evidence per bullet.

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all
  1030 baseline tests still pass.
- Stable alpha scope unchanged: the doc/release nodes are
  experimental capabilities, declared with `writableScopes: []`;
  they cannot write to the source repository or to the journal
  beyond the standard evidence retention.
- `prometheos work` behavior unchanged: this PR only adds
  `lite.node`-family capabilities consumed by the NodeRunner; it
  does not touch the workbench, intake, discovery, planning,
  implement, repair, validation, diagnostic, or review node
  surface.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry in
  `CHANGELOG.md`, which is the canonical record of recent work.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).
- The release-prep node is hard-coded to NEVER publish, tag,
  push, or merge. The `ReleasePrepResultV1` struct carries no
  authorization field; the unit test and the conformance test
  both assert the wire-format absence of every authorization key.

## Non-goals (explicit)

- No LLM-driven review. All nodes are deterministic pattern/rule-
  based.
- No new `prometheos work` command or subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop. The release-prep node only composes an
  artifact; it does not perform any release action.
- No new dependency. The implementation uses only existing
  infrastructure.
- No benchmark, conformance-fixture, or external-pilot work. E7
  issues remain on the queue and are explicitly deferred.
