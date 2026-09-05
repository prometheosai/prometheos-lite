# E6/I01 Slice A — CLI contract integration tests

Issue: #130 (`[E6/I01] Stabilize CLI commands, project configuration,
and workflow templates`).
Status: Slice A implemented; merged via PR #211 (commit
`a6884d8`) on `main` on 2026-09-05.

## Goal

This is the third and final bounded slice of E6/I01. The other
slices (Slice B: config version; Slice C: workflow templates) are
R4 and R5 respectively, both already merged. After this slice
lands, #130 closes.

The slice delivers one E6/I01 acceptance bullet:

- "CLI contracts are documented and integration-tested."

## What changed

### `src/cli/mod.rs`

A new in-module test mod `cli_contract_tests` (compile-guarded
by `#[cfg(test)]`, so release builds are unaffected). 14
parse-only tests (no runtime needed) cover the documented
invocations and the failure cases for every top-level
`Commands` variant in the `Cli` enum:

- `cli_parses_flow_run_with_minimum_args` +
  `cli_rejects_flow_run_without_required_path`
- `cli_parses_harness_run_with_required_task` +
  `cli_rejects_harness_run_without_required_task`
- `cli_parses_serve`
- `cli_parses_bench_run_with_required_task` +
  `cli_rejects_bench_run_with_unknown_subcommand`
- `cli_parses_work_with_subcommand`
- `cli_parses_repo_workbench_alias` (covers both `repo` and
  `repo-workbench` aliases)
- `cli_parses_templates_list`
- `cli_parses_workflow_propose` (smallest plausible workflow
  invocation; the workflow surface is large and not asserted in
  detail here)
- `cli_parses_diagnostics_provider`
- `cli_rejects_unknown_subcommand` +
  `cli_rejects_unknown_top_level_command` (the diagnostic-error
  assertions verify the message names the offending arg)

Why in-module rather than as an integration test in `tests/`:

The `Cli` enum is a binary-only type (declared in `src/cli/mod.rs`)
and is not re-exported from the library crate's public API.
Putting the tests in `tests/cli_contract_tests.rs` would require
either (a) `pub mod cli;` in `lib.rs` (a public-API change) or
(b) would fail to compile. Putting the tests in-module as
`#[cfg(test)] mod cli_contract_tests` keeps the public API
unchanged while still exercising the contract under
`cargo test --bin prometheos`.

Production code (lines 1-60 of `src/cli/mod.rs` — the `Cli`
struct, the `Commands` enum, the `run()` function) is unchanged.

### `CHANGELOG.md`

- New `## Unreleased` entry recording Slice A: 14 new in-module
  `#[cfg(test)]` tests.

## Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` —
  clean.
- `cargo test --lib -- --test-threads=1` — **1003 passed**, 0
  failed, 1 ignored (no regression; no new lib tests in this
  slice; the 14 new tests live in the binary target).
- `cargo test --bin prometheos` — **33 passed**, 0 failed (was
  19; +14 new tests).
- `cargo test --test node_library_conformance` — 21 passed, 0
  failed (no regression).
- `cargo test --test node_implementation_conformance` — 30
  passed, 0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0
  failed (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency
  changes).
- CI on PR #211: 13/13 green on the content head.

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent inspected the
diff, ran every verifier command, and walked the comparative
control gate bullet-by-bullet. Verdict: **APPROVE** with
concrete evidence per bullet. The reviewer explicitly confirmed:
"production code in `src/cli/mod.rs` lines 1-60 byte-identical to
main; `Cli`, `Commands`, `run()` untouched" and "the 14 new
tests are parse-only; none call `Cli::run()` or any other runtime
path."

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all
  1003 baseline lib tests + 19 baseline binary tests still
  pass; 14 new tests added.
- Stable alpha scope unchanged: the tests are `#[cfg(test)]`
  and compile-guarded out of release builds; they cannot affect
  runtime behavior.
- `prometheos work` behavior unchanged: the binary's runtime
  path (`Cli::run()`) is unchanged.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry
  in `CHANGELOG.md`.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).

## Non-goals (explicit)

- No LLM-driven work. The tests are pure parser assertions.
- No new `prometheos work` command or subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop.
- No new dependency. The tests use only the existing
  `clap::Parser::try_parse_from` machinery.
- No benchmark, conformance-fixture, or external-pilot work.

## E6/I01 closeout

All three slices of E6/I01 (#130) are now landed:

- Slice B (R4): config version + invalid-config diagnostics — PR
  #209 (`d0144bf`).
- Slice C (R5): six workflow templates for the six required
  workflow kinds — PR #210 (`987c6de`).
- Slice A (R6): CLI contract integration tests — PR #211
  (`a6884d8`).

Issue #130 is now closed. The next autonomous-loop task is R7
(E6/I02, #131).
