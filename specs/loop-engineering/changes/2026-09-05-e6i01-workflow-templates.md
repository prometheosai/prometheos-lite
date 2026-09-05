# E6/I01 Slice C — Six workflow templates

Issue: #130 (`[E6/I01] Stabilize CLI commands, project configuration,
and workflow templates`).
Status: Slice C implemented; merged via PR #210 (commits
`137b0a2` + `d32dbe9` on the rustfmt fix-up) on `main` on
2026-09-05.

## Goal

This is the third bounded slice of E6/I01. The other slices
(Slice A: CLI contract integration tests; Slice B: config
version + diagnostics) are R6 and R4 respectively; R4 is already
merged. Slice A remains.

The slice delivers one E6/I01 acceptance bullet:

- "Templates cover bug fix, feature, refactor, test, documentation,
  and review workflows."

## What changed

### Six new flow templates under `flows/`

- `bug_fix.flow.yaml` — reproduce / localize / fix / verify. Inputs:
  `bug_report`. Output: `fix_patch` + `reproducer_test` + `root_cause`.
- `feature.flow.yaml` — plan / implement / document / test. Inputs:
  `feature_description`. Output: `implementation` + `tests` +
  `documentation`.
- `refactor.flow.yaml` — baseline / refactor / verify. Inputs:
  `target_path`. Output: `refactor_patch` +
  `behavior_preserved_evidence`.
- `test.flow.yaml` — read / author / verify. Inputs: `target`.
  Output: `tests` + `coverage_report`.
- `docs.flow.yaml` — inspect / draft / verify. Inputs: `target`.
  Output: `documentation` + `examples`.
- `review.flow.yaml` — security / evidence / independent review.
  Inputs: `candidate_ref`. Output: `review_verdict` +
  `review_reasons` + `evidence_refs`. **This file also fixes a
  latent broken reference in `templates/software.yaml`** whose
  lifecycle_template declared `review.flow.yaml` as a required
  flow but the file did not exist on disk.

Each file is a minimal valid `FlowFile` matching the existing
schema in `src/flow/loader/mod.rs`:

- `version: "1.0"`
- `name`, `description`
- `inputs.required: Vec<String>`, `outputs.{primary, include}: ...`
- `nodes: Vec<NodeDefinition>` with a real `start_node` and
  `id`, `node_type`, `config`
- `transitions: Vec<TransitionDefinition>` with `from`, `action`,
  `to`

No schema changes were needed: the existing `YamlLoader`
consumes the new files without any code change in `src/`.

### `tests/api_flow_execution.rs`

- `e6i01_six_workflow_templates_load_and_have_valid_shape` — a
  table-driven test that loads each of the six templates via
  `YamlLoader` and asserts: `version == "1.0"`, the expected
  `name`, the expected `start_node`, non-empty `nodes`, non-empty
  `transitions`, and that every transition's `from` / `to` resolves
  to a declared node id.
- `software_template_review_flow_reference_is_satisfied` —
  asserts `flows/review.flow.yaml` exists (the file referenced
  by `templates/software.yaml`), fixing the latent broken
  reference.

### `CHANGELOG.md`

- New `## Unreleased` entry recording Slice C: 6 new fixture
  files, 2 new integration tests, and the review-flow reference
  fix.

## Verification

- `cargo fmt --check` — clean (after a rustfmt fix-up commit
  added by the independent reviewer).
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --lib -- --test-threads=1` — **1003 passed**, 0
  failed, 1 ignored (no regression; no new lib tests in this
  slice).
- `cargo test --test api_flow_execution` — 21 passed, 0 failed
  (was 19; +2 new tests).
- `cargo test --test node_library_conformance` — 21 passed,
  0 failed (no regression).
- `cargo test --test node_implementation_conformance` — 30
  passed, 0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0 failed
  (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency
  changes).
- CI on PR #210: 13/13 green on the content head (after the
  rustfmt fix-up pushed by the independent reviewer).

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent inspected the
diff, ran every verifier command, and walked the comparative
control gate bullet-by-bullet. Initial verdict: **REPAIR**
with a single one-line fix — `cargo fmt` was not run before
pushing. After the rustfmt fix-up commit (`d32dbe9`), the
verdict becomes **APPROVE**: every acceptance bullet is
satisfied, every verifier command passes, no safety-gate hard
blockers.

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all
  1003 baseline lib tests still pass; 2 new integration tests
  added.
- Stable alpha scope unchanged: the templates are additive
  fixtures consumed by the existing `YamlLoader`. They are not
  automatically wired into any `prometheos work` command path.
- `prometheos work` behavior unchanged: no CLI surface change.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry in
  `CHANGELOG.md`.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).

## Non-goals (explicit)

- No LLM-driven work. The templates are static fixtures.
- No new `prometheos work` command or subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop.
- No new dependency. The implementation uses only existing
  infrastructure (the `YamlLoader` already in `src/flow/loader/`).
- No benchmark, conformance-fixture, or external-pilot work.

## What remains for #130

- Slice A: CLI contract integration tests (R6). Each top-level
  `Commands` variant in `src/cli/mod.rs` gets a parse-only test
  that asserts the clap parser accepts the documented invocation
  and rejects malformed ones with an actionable error. The
  redaction-style secrets and the workbench interaction are out of
  scope here; this is purely a parser-shape test.

Issue #130 stays open until Slice A lands.
