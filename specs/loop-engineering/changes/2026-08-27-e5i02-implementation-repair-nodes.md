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

---

## Post-merge repair (2026-08-28)

The original PR #196 was merged before the human gate could review four
automated review findings (three P1, one P2). This section records the
focused repair that addresses them without scope creep.

### P1.1 — Path traversal through plan_id / diagnosis_id

- `validate_workspace_id(id: &str) -> anyhow::Result<()>` added to
  `src/workflow/workspace.rs`. Rejects: empty / > 200 chars, `..`,
  path separators (`/`, `\`, NUL), and any char outside
  `[A-Za-z0-9._-]`.
- Called at the start of `run_implement` and `run_repair` (before any
  workspace acquisition) AND from `GitWorktreeWorkspace::acquire` for
  defense in depth on `manifest.workspace_id`.
- Test: `repair_p1_1_implement_rejects_path_traversal_in_plan_id`,
  `repair_p1_1_repair_rejects_path_traversal_in_diagnosis_id`.

### P1.2 — Repository authority not bound to `repoRoot`

- `GitWorktreeWorkspace::acquire` now resolves the source repo's toplevel
  via `git rev-parse --show-toplevel` and requires the canonicalized
  `manifest.repo_identity` to match. Fails closed on mismatch.
- Required the existing workspace test helper `writable_manifest` to take
  a real `&Path` (the old form used a synthetic `origin/<name>` string
  that now correctly fails the authority check). Updated 7 test sites.
- Test: `repair_p1_2_authority_mismatch_is_rejected_fail_closed` builds
  a plan against `dir_a` and points the adapter at `dir_b`, asserting
  the mismatch is caught at acquire time.

### P1.3 — Emitted workspace reference cannot recover committed HEAD

- `WorkspaceRefV1` extended with optional `headRevision: Option<String>`
  (default None for forward compat; `deny_unknown_fields` preserved
  because new optional fields with `#[serde(default)]` are not
  "unknown"). Added `WorkspaceRefV1::compute_digest` so callers can
  re-seal after mutating `headRevision`.
- `WorkspaceAdapter::recover` now pins against `headRevision` when
  present and falls back to `baseRevision` for older refs.
- The implement / repair nodes now build a fresh `WorkspaceRefV1` after
  the commit: `baseRevision = newHEAD`, `headRevision = Some(newHEAD)`,
  `contentDigest` recomputed via the new helper.
- `to_reference()` (from a pre-write manifest) still emits
  `headRevision = None` so older callers and ref-only consumers see
  identical JSON shape.
- Test: `repair_p1_3_emitted_ref_carries_committed_head_and_recovers`
  drives the full implement → ref → recover cycle and asserts the
  on-disk worktree revalidates against the emitted reference.

### P2 — Generated audit timestamps were incorrect

- The hand-rolled `chrono_like_iso` in `node_implementation.rs` produced
  wrong dates (the proleptic Gregorian year/day math was off by the
  number of leap years). Removed.
- `pub fn now_iso()` in `src/workflow/mod.rs:409` was already the
  canonical RFC3339 helper (uses `chrono::DateTime::from_timestamp`).
  The node module now calls `crate::workflow::now_iso()` directly.
- Test: `repair_p2_emitted_timestamp_round_trips_through_chrono` parses
  every `appliedAt` via `chrono::DateTime::parse_from_rfc3339` and
  asserts the year is in a plausible range (2024..=2100).

### Test fragility surfaced + fixed (collateral)

- `references_round_trip_without_process_state` in `workspace.rs` had a
  pre-existing fragile PID-substring check that became reliably false
  after `headRevision` was added to the digest. Replaced with a
  positive assertion: the serialized JSON's top-level key set equals
  the exact `WorkspaceRefV1` field list. The `deny_unknown_fields`
  guarantee makes this check both stronger and stable.

### Verification (repair)

- `cargo fmt --check` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0
- `cargo test --lib` — 922 passed, 0 failed
- `cargo test --test node_library_conformance` — 7 passed
- `cargo test --test node_implementation_conformance` — **10 passed**
  (5 original + 5 new repair tests)
- No `Cargo.toml` / `Cargo.lock` edits

### Files touched (repair)

- `src/workflow/workspace.rs` — `validate_workspace_id`, repo authority
  check in `acquire`, `WorkspaceRefV1.headRevision` + `compute_digest`,
  `recover` update, test helper + test-site updates, fragile-test fix.
- `src/workflow/node_implementation.rs` — `validate_workspace_id` calls,
  post-commit `WorkspaceRefV1` construction, switch to
  `crate::workflow::now_iso`, drop broken `chrono_like_iso`.
- `src/workflow/mod.rs` — `now_iso` is now `pub` (one-character change).
- `tests/node_implementation_conformance.rs` — 5 new repair tests
  (P1.1 × 2, P1.2, P1.3, P2).
- `specs/loop-engineering/changes/2026-08-27-e5i02-implementation-repair-nodes.md`
  — this section.

Five files; no new dependencies; no Cargo edits.
