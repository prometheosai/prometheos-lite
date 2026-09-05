# E6/I02 Slice A — Read-only run inspector

Issue: #131 (`[E6/I02] Build the run inspector, evidence viewer,
and human decision interface`).
Status: Slice A implemented; merged via PR #212 (commit
`270a625`) on `main` on 2026-09-05.

## Goal

This is the first bounded slice of E6/I02. The canonical E6/I02
also mentions "graph state, node attempts" which is a different
surface (the E4 graph surface, not the repo-workbench surface).
This slice targets the repo-workbench read-only inspector; future
slices may add a graph-state inspector and content-hash-based
stale-approval detection.

The slice delivers all five E6/I02 acceptance bullets (within the
repo-workbench scope):

- Reviewer can trace every claim to evidence.
- Human decisions include actor, timestamp, target SHA/state,
  and rationale.
- Stale approvals are rejected after state or artifact changes
  (detection; re-validation as a pre-apply hook is a future slice).
- Interface cannot bypass authority policy (read-only).
- CLI output remains usable without a web interface (text by
  default; `--json` for downstream tooling).

## What changed

### `src/cli/commands/work.rs`

- New `Inspect` variant in `WorkSubcommand`:
  - `prometheos work inspect <id> [--json]`.
  - The match arm calls `inspect_repo_workbench(&id)?` and prints;
    it does NOT call any mutation function (no
    `approve_artifact`, no `set_status`, no `add_decision`, no
    `save_context`, no `write_memory`).
- `inspect_repo_workbench(id: &str) -> Result<serde_json::Value>`
  — top-level helper that:
  1. Loads the context via `repo_workbench::load_context(&id)`.
  2. Reads the artifacts via `repo_workbench::get_artifacts`.
  3. Detects stale approvals: an approval is stale iff
     `decision.approved == true && decision.artifact_id ∉
     current_artifact_ids`. The result is a `staleApprovals`
     array of artifact_ids. Conservative: a future slice may add
     content-hash comparison for full coverage.
  4. Builds a `serde_json::Value` report with the fields
     `schemaVersion`, `workId`, `title`, `goal`, `status`,
     `phase`, `createdAt`, `updatedAt`, `artifacts`,
     `decisions`, `staleApprovals`. The "actor" of each decision
     is derived from the artifact's `provenance.generator` (the
     same field the rest of the repo-workbench surface uses for
     attribution).
- `print_inspect_report_text(report)` — top-level helper that
  formats the same `serde_json::Value` for human consumption.
  The text path prints a "Read-only: this command did NOT mutate
  any state." footer so the operator has explicit confirmation.
- `#[cfg(test)] mod inspect_tests` — 5 unit tests:
  1. `inspect_report_includes_metadata_artifacts_and_decisions`
  2. `stale_approvals_are_listed_and_unapproved_is_not`
  3. `decision_actor_is_the_artifacts_provenance_generator`
  4. `approved_decision_for_existing_artifact_is_not_stale`
     (regression test)
  5. `unapproved_decision_is_not_stale_even_if_artifact_gone`
     (unapproved decisions must NOT be flagged as stale)
- `inspect_repo_workbench_for(ctx)` — test helper that mirrors
  the production helper for in-memory contexts. Keeps the
  tests hermetic and fast (no tempdir, no file I/O).

### `src/cli/mod.rs`

- 1 new parse-only test `cli_parses_work_inspect` in the
  existing `cli_contract_tests` mod: it asserts
  `prometheos work inspect <id>` and
  `prometheos work inspect <id> --json` both parse.

### `CHANGELOG.md`

- `## Unreleased` entry recording the slice.

## Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --lib -- --test-threads=1` — **1003 passed**, 0
  failed, 1 ignored (no regression; no new lib tests in this
  slice).
- `cargo test --bin prometheos` — **39 passed**, 0 failed (was
  33 before; +5 inspect_tests + 1 cli_parses_work_inspect).
- `cargo test --test node_library_conformance` — 21 passed, 0
  failed (no regression).
- `cargo test --test node_implementation_conformance` — 30
  passed, 0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0
  failed (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency
  changes).
- CI on PR #212: 13/13 green on the content head.

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent inspected the
diff, ran every verifier command, and walked the comparative
control gate bullet-by-bullet. Verdict: **APPROVE** with
concrete evidence per bullet. The reviewer explicitly verified
that the inspect handler only calls `inspect_repo_workbench(&id)?`
and never a mutation function, and that the diff size (3 files,
+422/-0 LOC) is within the minimality budget.

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all
  1003 baseline lib tests still pass; 6 new tests added.
- Stable alpha scope unchanged: the new subcommand is
  read-only and additive; existing `prometheos work` subcommands
  are untouched.
- `prometheos work` behavior unchanged.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry in
  `CHANGELOG.md`.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).

## Non-goals (explicit)

- No LLM-driven work. The inspector is a deterministic report
  builder.
- No new `prometheos work` command or subcommand beyond the
  additive `inspect` subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop. The inspect command is read-only and
  does not trigger any workflow side effects.
- No new dependency. The implementation uses only existing
  infrastructure (`serde`, `serde_json`, `chrono`).
- No benchmark, conformance-fixture, or external-pilot work.

## What remains for #131

- A pre-apply hook that RE-validates stale approvals and refuses
  to apply a patch if any referenced approval is stale. This
  converts the detection (current slice) into a hard block.
- A graph-state / node-attempt inspector (the canonical E6/I02
  also mentions "graph state"). This is a different surface
  (the E4 graph state) and would land in a separate slice.
- A content-hash-based stale-approval check (the current
  artifact_id-based check is conservative; content-hash
  comparison would catch changes to the artifact content even if
  the id stays the same).

Issue #131 stays open until all of the above land.
