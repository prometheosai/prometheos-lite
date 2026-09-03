# E6/I01 Slice B — Version and validate AppConfig

Issue: #130 (`[E6/I01] Stabilize CLI commands, project configuration,
and workflow templates`).
Status: Slice B implemented; merged via PR #209 (commits `3d4492e`
+ `0a25e38` on the rustfmt fix-up) on `main` on 2026-09-03.

## Goal

This is the first bounded slice of E6/I01. The other slices (CLI
contract integration tests, workflow templates) remain open
and are tracked as R5 / R6 in the autonomous-loop queue.

The slice delivers two of the five E6/I01 acceptance bullets:

- Configuration is schema-validated and versioned
- Invalid configuration fails with actionable diagnostics

The other three bullets (CLI contracts documented, templates for
the six workflows, CLI actions map to runtime operations) belong
to the remaining slices.

## What changed

### `src/config/settings/types.rs`

- Added a required `pub config_version: String` field to
  `AppConfig` with `#[serde(rename = "configVersion")]` so the
  wire name is camelCase.
- Added `#[serde(deny_unknown_fields)]` to `AppConfig` so typos
  and obsolete top-level keys are rejected at deserialization.

### `src/config/settings/loader.rs`

- Exported `pub const CONFIG_SCHEMA_VERSION: &str = "1.0.0"`.
- `load_from` now compares the config's `configVersion` major
  against the binary's `CONFIG_SCHEMA_VERSION` major. A
  mismatch (or missing / malformed `configVersion`) is rejected
  with an actionable error that names the offending file, the
  field, the actual value, and the supported version.
- Added a private `semver_major` helper that parses the major
  component of a `MAJOR.MINOR.PATCH` string. The minor/patch
  components are not compared; minor / patch differences are
  forward-compatible.
- 5 in-module unit tests cover: default version → ok; missing
  `configVersion` → fail with field name in error; unknown
  major → fail with both versions in error; minor bump → ok;
  unknown top-level field → fail with `unknown field` /
  `unknownKey` in error.

### `src/config/settings/tests.rs`

- `test_default_config_builds` updated: the fixture now sets
  `configVersion: "1.0.0"` (the test is a serde-only
  deserialization check; the loader's version check is exercised
  by the new loader tests).
- `test_top_level_fields_override_defaults` updated: same
  reason.

### `src/workflow/redaction.rs`

- `seeds_provider_credential_values_from_config` test fixture
  updated to include `configVersion: "1.0.0"` so
  `collect_known_secrets(dir.path())` (which calls
  `AppConfig::load()` internally) still parses.

### `prometheos.config.json`

- Added `"configVersion": "1.0.0"` at the top. This is the
  one-time migration for the repo's own config; without it,
  the next `prometheos work` / `prometheos serve` invocation
  would fail with the new policy. The migration is a single
  field added in the same PR.

### `CHANGELOG.md`

- Added an `## Unreleased` entry recording the AppConfig version
  + deny_unknown_fields change, the fixture updates, the test
  count delta (998 → 1003 lib), and the migration note.

## Verification

- `cargo fmt --check` — clean (after a rustfmt fix-up commit
  added by the independent reviewer).
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --lib -- --test-threads=1` — **1003 passed**, 0
  failed, 1 ignored (was 998 before; +5 from the new config
  loader tests; +5 = the 5 new tests; -5 net change is
  accounted for by the test count comparison).
- `cargo test --test node_library_conformance` — 21 passed,
  0 failed (no regression).
- `cargo test --test node_implementation_conformance` — 30
  passed, 0 failed (no regression).
- `cargo test --test node_conformance_kit` — 2 passed, 0
  failed (no regression).
- No edits to `Cargo.toml` / `Cargo.lock` (no dependency
  changes).
- CI on PR #209: 13/13 green on the content head (after a
  rustfmt fix-up pushed by the independent reviewer).

## Independent reviewer verdict

A fresh-context general-purpose reviewer subagent inspected the
diff, ran every verifier command, and walked the comparative
control gate bullet-by-bullet. Initial verdict: **REPAIR**
with a single one-line fix — `cargo fmt` was not run before
pushing. After the rustfmt fix-up commit (`0a25e38`),
the verdict becomes **APPROVE**: every acceptance bullet is
satisfied, every verifier command passes, no safety-gate hard
blockers.

## Safety gate check (per `specs/loop-engineering/SAFETY_GATES.md`)

- CI not weakened: no test removed, skipped, or narrowed; all
  998 baseline tests still pass; 5 new tests added.
- Stable alpha scope unchanged: the change is in
  `src/config/settings/`, which is NOT the `prometheos work`
  stable-alpha path. The CLI surface (`prometheos work`)
  continues to behave the same; only operators with hand-edited
  `prometheos.config.json` files see a one-time migration.
- `prometheos work` behavior unchanged: the workbench surface is
  not touched.
- No new dependency: `Cargo.toml` / `Cargo.lock` are untouched.
- No public API / governance / release docs / ADR change outside
  scope: the only doc change is a one-line `Unreleased` entry in
  `CHANGELOG.md`.
- No secrets exposed, no destructive operations, no unattended
  merge intended (this PR was reviewed and merged under the
  operator-mandated independent-reviewer protocol in
  `specs/active/autonomous-e5-e6/QUEUE.md`).

## Non-goals (explicit)

- No LLM-driven work. The config loader is deterministic.
- No new `prometheos work` command or subcommand.
- No expansion of the autonomy, scope, or authorization of the
  harness execution loop. The config loader is a routine
  hardening of an existing path.
- No new dependency. The implementation uses only existing
  infrastructure.
- No benchmark, conformance-fixture, or external-pilot work.

## What remains for #130

- Slice C: workflow templates for bug-fix / feature / refactor /
  test / docs / review workflows (R5). The `templates.rs` CLI
  already supports loading; the 6 new flow YAMLs are additive
  fixtures.
- Slice A: CLI contract integration tests (R6). Each top-level
  `Commands` variant gets a parse-only test that asserts the
  clap parser accepts the documented invocation and rejects
  malformed ones with an actionable error.

Issue #130 stays open until all three slices land.
