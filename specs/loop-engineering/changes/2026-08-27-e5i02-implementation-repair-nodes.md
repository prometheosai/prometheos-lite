# E5/I02 — Governed implementation and repair nodes

Issue: #126 (`[E5/I02] Implement governed implementation and repair nodes`).
Status: implemented; awaiting PR review and merge.
Date: 2026-08-27.

## Goal

Complete the E5 executor half of the governed node library: the writing nodes
that turn a `ScopedPlanV1` (or a `DiagnosisV1`) into a recorded, reviewable
change set inside an isolated, governed workspace. Unlike the read-only
intake/discovery/planning nodes (#125), these nodes MUST NOT mutate the source
checkout; all writes happen in a `GitWorktreeWorkspace` pinned to the cited
base revision.

## What changed

### New module — `src/workflow/node_implementation.rs`

- Declared two Lite-owned capabilities:
  - `implement` — takes a `ScopedPlanV1` (plus `repoRoot` + `workspaceParent`),
    acquires a writable worktree pinned to `plan.discovery_revision`, records
    one change artifact per plan step under
    `prometheos/changes/<planId>/<step>.change.json`, commits, and emits a
    typed `ImplementationResultV1` linked back to the plan.
  - `repair` — takes a `DiagnosisV1` (with `baseRevision`, `failingTarget`,
    `message`, `diagnosisId`), acquires a worktree pinned to
    `diagnosis.baseRevision`, records a corrective change artifact under
    `prometheos/repairs/<id>.repair.json`, commits, and emits a typed
    `RepairResultV1` linked to the diagnosis.
- Typed contracts added:
  - `DiagnosisV1` — `diagnosisId`, `failingTarget`, `message`, `baseRevision`
    (version `1.0.0`).
  - `ImplementationChangeV1` — per-step evidence record
    (`step`, `title`, `targets`, `evidenceRef`, `appliedAt`).
  - `ImplementationResultV1` — committed result (`planId`,
    `discoveryEvidenceId`, `revision`, `workspaceRef`, `changedFiles`,
    `changes`).
  - `RepairResultV1` — committed result (`repairId`, `diagnosisRef`,
    `failingTarget`, `revision`, `workspaceRef`, `changedFiles`,
    `correctiveSummary`).
- `implementation_repair_registry()` — registers both capabilities on a
  `CapabilityRegistry`; the generic nine-gate `NodeRunner` enforces all the
  contracts (lite.node.v1, lite.policy.v1, journal durability).
- `node_manifest(node_id, capability)` — builds a `NodeManifestV1` carrying
  `writableScopes: ["repo://fixture"]` and matching `readableScopes`, so the
  policy gate can fail closed when the local policy does not grant the scope.
- Three internal lib unit tests:
  - `implement_rejects_missing_plan`
  - `repair_rejects_missing_diagnosis`
  - `manifest_declares_writable_scope`
- Self-contained timestamp helper `now_iso()` (no `chrono` dependency in the
  node surface) and minimal `git` + `head_revision` helpers in the node
  module — the workspace module's own helpers are private and intentionally
  scoped to the adapter.

### Module wiring — `src/workflow/mod.rs`

- Added `pub mod node_implementation;` (sibling of `node_library`).

### Conformance — `tests/node_implementation_conformance.rs` (new, 5 tests)

Drives both nodes through the generic `NodeRunner` (same machinery as
#125's conformance + the E3 conformance kit). Tests prove:

- `implement_acquires_worktree_commits_and_links_plan` — implement completes
  under a granted writable scope; result is typed, links `planId` and
  `discoveryEvidenceId`, produces a NEW commit (revision differs from base),
  emits `prometheos/changes/...` artifacts, parses through
  `WorkspaceRefV1::parse_json`, and emits a `node-output` evidence ref.
- `implement_does_not_mutate_source_checkout` — after the implement node
  runs, the source repository's HEAD is unchanged, `prometheos/` does NOT
  exist in the source, and `git status --porcelain` is clean. Proof of
  workspace isolation.
- `repair_records_corrective_change_linked_to_diagnosis` — repair completes
  with granted write scope; result is typed and cites `diagnosisRef` +
  `failingTarget`; produces a commit; `workspaceRef` round-trips through
  `WorkspaceRefV1`; evidence journal records a `node-output` entry.
- `policy_denies_write_when_local_writable_scope_is_empty` — with local
  `writableScopes: []` and a manifest declaring `writableScopes:
  ["repo://fixture"]`, the runner DENIES the call before any effect
  (lite.policy.v1 gate 3 fail-closed behavior). Mirrors the #125 policy
  test.
- `implement_rejects_unparseable_plan` — malformed plan input is rejected
  by the handler before any workspace acquisition.

## Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0.
- `cargo test --lib` — **922 passed**, 0 failed, 1 ignored (was 919 before;
  +3 from this module's unit tests).
- `cargo test --test node_library_conformance` — 7 passed, 0 failed.
- `cargo test --test node_implementation_conformance` — 5 passed, 0 failed.
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency changes).

## Non-goals (explicit)

- No LLM-driven synthesis. The node's governed responsibility is the
  write pipeline (acquire isolation under authority, record a durable,
  reviewable change artifact, commit, emit evidence). Concrete source
  edits in production are supplied by a provider; the executor wires the
  governed path so a provider can plug in without acquiring new write
  authority.
- No orchestration / multi-plan execution; one implement call → one
  commit set per plan.
- No new external dependencies. The worktree adapter and the runner's
  policy/journal gates are reused unchanged.

## Linkage

- Builds on: #125 (intake / discovery / planning), #124 (governed
  workspace adapter + workspace module), #123 (nine-gate node runner).
- Feeds: later E5 executor-side issues (parallel execution, commit
  aggregation) and the E6 cloud path.
- Issue #126 will be closed on PR merge.
