# Change: E6/I05 — repository onboarding and actionable diagnostics

**Issue:** #134 · **Parent epic:** #107 (E6)
**Depends on:** #125 ✅, #130 ✅
**Builds on:** `repo_index.rs` (read-only repository index), `soma::diagnostic_extensions`
(the repository-owned diagnostic-code registry), `governance_compiler::remediation_for`
(curated remediation hints), the `prometheos work` CLI surface (#130).

## Objective

Detect repository languages, package systems, test commands, protected paths,
dependency manifests, Git state, and a likely validation strategy — producing a
**reviewable onboarding proposal** with detection confidence and evidence, that
**enables no authority silently**: no scopes, capabilities, or tools appear
anywhere in the proposal, and nothing consumes it as a grant. Unsupported or
conflicting setups stop with typed, actionable diagnostics. User edits override
detection explicitly and are recorded; onboarding never modifies project files,
and the apply step requires explicit approval before writing anything at all.

## Deliverables

1. **Onboarding contracts** (`src/workflow/onboarding.rs`, NEW) —
   versioned `lite.onboarding.v1` contracts: `DetectionRowV1` (value +
   confidence `high`/`medium` + evidence paths), `OnboardingSurveyV1`
   (read-only survey, digest-bound over its own members), `TestCommandV1`,
   `ProposedConfigV1` (deliberately authority-free: no scopes, capabilities,
   or tools), `OverrideRowV1`, `OnboardingManifestV1` (the reviewable
   document: detection-owned survey block + human-editable proposal +
   recorded overrides + approval bit).
2. **Read-only survey** — `survey_repository(repo_root)`: composes with the
   existing `IndexedRepository` (revision, dirty, languages, file index — the
   same machinery `repo-discovery` uses) and detects package systems from
   marker manifests (cargo, npm, python, go, maven, gradle), deep-parsing
   TOML/JSON markers so a present-but-broken manifest refuses. Test commands
   and validation strategies are derived per detected system (medium
   confidence until the repository pins them; `package.json scripts.test` is
   recorded as explicit evidence). Protected paths always include `.git` and
   the tool's own `.prometheos-lite` store; `.env` files are flagged when
   present. Deterministic row order; survey digest over the canonical survey
   minus the digest member.
3. **Actionable diagnostics** — six new repository-owned diagnostic codes
   pinned in the extension registry (`SOMA-ONB-0001` no supported package
   system, `0002` not a usable git repository, `0003` unparseable package
   manifest, `0004` repository changed since proposal, `0005` survey block
   modified (detection-owned), `0006` approval required), each with a
   curated remediation action in `remediation_for` and pinned categories
   (`onboarding_unsupported`/`onboarding_conflict`/`onboarding_state`).
   The `emitted_diagnostics_conformance` mirror (`EXTENSION` + `EMITTED`) is
   extended in sync, and a resolution test pins all six codes through the
   same `category_for` path every emitted diagnostic takes.
4. **Explicit overrides + approval-gated apply** — `apply_user_manifest`:
   a user manifest whose survey no longer matches its own digest is refused
   as tampered evidence (`0005`); an honest survey that no longer matches a
   fresh survey is refused as drift (`0004` — regenerate); approval must be
   explicit in the reviewed document AND in the call (`0006`). Every
   difference between the detection proposal and the user's configuration is
   recorded as an `OverrideRowV1` (field, detected, user) — the human's value
   wins, and the provenance is durable.
5. **CLI surface** — `prometheos work onboarding [--repo] [--out] [--apply]
   [--approve] [--json]`: prints the survey with confidence + evidence and
   writes the proposal into the tool's own `.prometheos-lite` store (never
   project files); `--apply` re-surveys, applies the user's edits with the
   typed refusals above, and writes the applied manifest to
   `.prometheos-lite/onboarding.json` only after approval is recorded —
   printing every recorded override and an explicit safety line. Refusals
   print code + message + remediation action and exit nonzero without
   writing.
6. **Conformance** (`tests/onboarding_conformance.rs`, NEW, strict TDD —
   written first, then made green) — 13 tests covering every acceptance
   row: supported cargo/npm/multi-system repositories receive reviewable
   manifests with confidence + evidence; empty/non-git/broken-manifest
   setups stop with the three actionable refusal codes; user edits override
   detection explicitly (override row recorded, user value wins);
   survey-block tampering and repository drift refuse; apply requires
   explicit approval (document bit AND flag); the survey is proven
   read-only by before/after file snapshots; and the whole flow is
   byte-deterministic.

## Local gate wiring

`scripts/local_ci.py`: new check `repository onboarding` → `cargo test --test
onboarding_conformance --quiet` in BOTH `SUITE_SPEC["core"]` and `rust_core()`
(identical normalized commands per the #225 evidence contract).

## Scope decisions (stated honestly)

- The applied manifest is a recorded configuration artifact only — nothing
  consumes it as authority yet; grants continue to flow exclusively from
  reviewed workflows and permits (no silent enablement, by construction).
- The CLI writes only inside `.prometheos-lite/` (the tool's own store, the
  same store the Workbench already uses); project files are never touched,
  proven by snapshot tests.
- Test commands are suggestions at medium confidence; no command is ever
  executed by onboarding.
- No dependency change (`toml` is already a dependency); vendored SOMA,
  frontend, and the API server untouched.

## Verification

- `cargo fmt --check`, clippy `--all-targets --all-features -- -D warnings`,
  full `cargo test --all-targets --all-features -- --test-threads 4`, focused:
  `onboarding_conformance` (13), `emitted_diagnostics_conformance` (14),
  plus the standing suites.
- Repository-owned gates at the exact candidate commit:
  `python scripts/local_ci.py run --suite core|platform|smoke`, then
  `verify --commit <head> --require-platform windows` with all three evidence
  files; evidence attached to the PR.
- Open PR for fresh independent review; no merge authorized by the dispatch.
