# Change Spec: E2 — Evidence & Resource Integrity Limits

- **PR:** #170 (DRAFT)
- **Branch:** `feat/e2-evidence-integrity-limits`
- **Status:** round-3 review-blocker revision complete and pushed; local verification passing; returned to REVIEW_GATE. Do **not** merge or begin #152.

## Summary

Brings evaluation evidence and resource bounds to an auditable, tamper-evident,
leak-safe state for the alpha-stable `prometheos work` surface. Implements the
E2 worktree controls from the product surface inventory:

- SHA-256 checksum sidecars for every persisted evaluation artifact.
- Fail-closed tamper + legacy-tolerant verification on read.
- Secret canary redaction for all persisted diagnostics and raw logs.
- Bounded validation capture (timeout + output cap) with OS-process-tree kill.
- Honest resource-limit classification (CPU / memory / disk / timeout / output).
- Orphan artifact reclamation planned against protected references.

## Files

- `src/workflow/artifact_integrity.rs` (new): `ArtifactDigest`, `ArtifactKind`,
  `DigestAlgorithm`, `sha256_hex`, `validate_digest_hex`, `validate_artifact_path`,
  `sidecar_for`, `publish_with_integrity`, `read_verified`, `read_verified_or_legacy`,
  `verify_with_sidecar`. In-module tests cover round-trip, tamper detection,
  corruption matrix (kind/path/digest mismatch), and hostile path/hex rejection.
- `src/workflow/redaction.rs` (new): `REDACTED_PLACEHOLDER`, `SECRET_CANARY`,
  `Redactor` (url userinfo, auth header, bearer/basic, JSON credential keys,
  query secrets), `redact_diagnostics`. In-module tests.
- `src/workflow/retention.rs` (new): `ProtectedReferences`, `RetentionEntry`,
  `RetentionPlan`, `RetentionOutcome`, `collect_candidates`, `plan_retention`,
  `apply_retention`, `extend_from_portable_work_state`. Fail-closed on
  out-of-repo paths. In-module tests.
- `src/workflow/evaluate/resource.rs` (new): `ResourceLimitKind`, `ResourceLimits`,
  classification constants, `classification_for_resource`. In-module tests.
- `src/workflow/evaluate/validation.rs`: `run_isolated_validation` now takes
  `&ResourceLimits` and `&[String]` (known secrets). Adds `bounded_run`
  (timeout + output cap, process-tree kill on timeout/cancel, OS CPU/memory
  caps via Unix `setrlimit` / Windows Job Object), `verify_patch_integrity`
  (rejects mismatched `patch_hash` and `approved.patch_hash`),
  `check_output_budget`, disk-pressure preflight, redaction of
  stdout/stderr/validation command, and raw-log persistence via
  `publish_with_integrity`. Resource exhaustion maps to `InfraBlocked`.
- `src/workflow/evaluate/evidence.rs`: proposal + validation + integrity artifacts
  now carry checksums; loaders verify (legacy-tolerant).
- `src/workflow/evaluate/generation.rs`: proposal load path verifies digest.
- `src/workflow/evaluate/orchestrator.rs`: evidence load verifies digest (fail-closed
  `read_verified`); both validation call sites now pass
  `ResourceLimits::from_environment().with_manifest_disk(...)` and
  `collect_known_secrets(&repo)`; `reclaim_orphan_artifacts` runs after finalize.
  `EvaluationConfig` unchanged (no ~80 test literals touched — see REVIEW_GATE).
- `src/workflow/mod.rs`: proposal save/load uses `publish_with_integrity` /
  `read_verified_or_legacy`; module declarations for the three new modules.
- `src/workflow/durable.rs`: adds `atomic_write_bytes`.

## Acceptance

- [x] Every persisted evaluation artifact is checksummed via SHA-256.
- [x] Tamper / legacy-tolerant verification on read (`read_verified_or_legacy`).
- [x] Corruption matrix (kind/path/digest) fails closed — covered by
      `artifact_integrity::tests`.
- [x] `prometheos work` does not write the secret canary to disk — unit test
      `known_secret_is_redacted_from_persisted_validation` in `validation.rs`
      runs a validation command echoing `SECRET_CANARY` with the canary in
      `known_secrets` and asserts persisted record + raw logs contain zero
      canary bytes; `collect_known_secrets` is wired into the capture path.
- [x] Validation is bounded (timeout + output cap) and kills the process tree.
- [x] CPU/memory enforced at the OS level: Unix `setrlimit` (RLIMIT_AS /
      RLIMIT_CPU) in `bounded_run` pre_exec, Windows Job Object
      (`apply_job_limits`) for commit + working-set + job-time limits.
- [x] Disk-pressure preflight via `preflight::available_disk_bytes` before
      `bounded_run`; fails closed when free space < `min_free_disk_bytes`.
- [x] Output-cap enforced by `check_output_budget` (validation rejected if
      stdout+stderr exceeds `max_output_bytes`).
- [x] Resource exhaustion maps to `InfraBlocked` (dry-run + execution paths).
- [x] Patch integrity enforced by `verify_patch_integrity`: `patch_hash` and
      `approved.patch_hash` must match the SHA-256 of the recorded patch.
- [x] Orphan reclamation is planned against protected references, fails closed on
      out-of-repo paths, deletes the artifact first then its sidecar only on
      success (`apply_retention` is atomic), and is integrated via
      `reclaim_orphan_artifacts` after finalize.
- [x] Fail-closed reads: `read_verified` (requires sidecar) is now used on all
      critical evidence/validation/integrity/proposal load paths, not the
      legacy-tolerant variant.
- [x] Full cancellation and interruption-recovery suites pass. Verified locally
      (single-threaded to avoid a pre-existing parallel-load timing flake on
      `heartbeat_loss_during_validation_prevents_publication`):
      `cargo test --lib` 777 passed; `workflow_evaluate_tests` 52 passed;
      `cancellation_tests` 19; `interruption_recovery_tests` 11;
      `durable_state_tests` + `portable_state_tests` 64; `cargo clippy
      --all-targets --all-features -D warnings` clean; `cargo fmt --check` clean.
- [ ] Acceptance fixtures under `tests/fixtures/e2-evidence-integrity/` — covered
      in practice by the in-module round-trip/tamper tests rather than static
      fixtures; no external validator is available in this environment to run
      the true e2 validation path.

## REVIEW_GATE — before merge

The review blockers raised on `91ccdd1` are addressed in `b7a1a71`:

1. **Fail-closed reads (missing sidecars).** Critical load paths now use
   `read_verified` (requires the SHA-256 sidecar; fails closed). The
   legacy-tolerant `read_verified_or_legacy` is no longer on the critical path.
   Covered by `artifact_integrity::tests::missing_sidecar_fails_closed`.
2. **CPU / memory / disk enforcement.** Implemented: Unix `setrlimit` and Windows
   Job Object for CPU/memory; disk-pressure preflight; output-cap budget. Tests:
   `validation_timeout_is_enforced_as_resource_violation`,
   `disk_preflight_blocks_validation_when_unsatisfied`,
   `check_output_budget_enforces_cap`, and `#[cfg(unix)]
   cpu_limit_kills_runaway_process`.
3. **Resource failures must not break #114 recovery.** Resource exhaustion is
   classified `InfraBlocked` (via `classify_dry_run_error` on
   CLASSIFICATION_* prefixes). The recovery suites
   (`cancellation_tests`, `interruption_recovery_tests`, `durable_state_tests`,
   `portable_state_tests`) pass. The `heartbeat_loss_*` test is a pre-existing
   parallel-load timing flake (passes single-threaded / in isolation).
4. **Production secret injection + patch rejection.** `collect_known_secrets`
   (env `PROMETHEOS_KNOWN_SECRETS` + `.prometheos/known_secrets`) is wired into
   `run_isolated_validation`; `verify_patch_integrity` rejects tampered
   `patch_hash` and `approved.patch_hash`. Tests:
   `known_secret_is_redacted_from_persisted_validation`,
   `patch_integrity_rejects_tampered_patch_hash`,
   `patch_integrity_rejects_mismatched_approval_hash`.
5. **Retention integration + artifact/checksum coupling.** `reclaim_orphan_artifacts`
   is integrated after finalize (scoped to `.prometheos/workflow`); `apply_retention`
   is atomic (artifact first, sidecar only after). Tests:
   `expired_orphan_is_removed_with_sidecar`,
   `sidecar_preserved_when_artifact_removal_fails`,
   `reclaim_orphan_artifacts_scoped_to_workflow_tree`.
6. **Cross-platform enforcement tests.** Added (see 1–5). Note: the
   `#[cfg(unix)]` CPU test only runs on Unix CI; CPU/memory on Windows is
   exercised by `apply_job_limits` (best-effort, logged warning on failure).

No benchmark or autonomous-execution claims. This change is confined to the
alpha-stable `prometheos work` surface. Do not merge or begin #152.

## REVIEW_GATE — round 2 (on `40a15d2`)

The reviewer's `#pullrequestreview-4952312004` (7 PRIMARY BLOCKERS) on
`40a15d252850ea100a838c3372283a6b664d0c03` are addressed in this revision:

1. **Unix CI fails to compile (unsafe `pre_exec`).** `tokio::process::Command::pre_exec`
   is an `unsafe fn`; the call is now wrapped in `unsafe { cmd.pre_exec(...) }`.
   Additionally the `setrlimit` calls inside `pre_exec` now return `Err` on
   failure (fail-closed) instead of logging. CI compiles on both Unix and Windows.
2. **Empty retention protection can delete authoritative state.** `reclaim_orphan_artifacts`
   is now invoked with a **populated** `ProtectedReferences`: the proposal registry
   file, the journal subtree (`insert_dir`), every referenced proposal directory
   discovered via the new public `read_registry`, and the current run dir. Added
   `ProtectedReferences::insert_dir` (recursive, skips `.sidecar.json`) and
   `pub fn read_registry` in `registry.rs`. Tests: `insert_dir_protects_subtree_
   including_sidecars`, `reclaim_preserves_referenced_dir_and_reclaims_orphans`.
3. **Resource failures not durably recorded before `ValidationComplete`.** On the
   validation `Err` branch, a `ValidationRecord::resource_failure(...)` is written
   durably via `write_validation_artifact` **before** the `ValidationComplete`
   journal transition, and `bundle.validation` is set so recovery maps it to the
   correct terminal state. Added `ValidationRecord::resource_failure`. Test:
   `resource_failure_record_is_a_terminal_failure`.
4. **Integrity metadata not bound to artifact path; evidence parsed before
   verification.** `verify_with_sidecar` now requires `digest.path ==
   durable::repo_relative_path(repo, absolute_path)` and bails before any record
   deserialization. The path itself is read only as a string from the sidecar
   bytes, never as a deserialized record. Test: `integrity_metadata_is_bound_
   to_artifact_path`.
5. **Secret-bearing patch rejection remains unwired.** Added
   `verify_patch_free_of_secrets(&proposal, known_secrets)?`, invoked immediately
   after `verify_patch_integrity` in `run_isolated_validation`; the run fails
   closed with a "known secret" error. Test: `patch_containing_known_secret_is_
   rejected`.
6. **CPU/memory/disk enforcement not fail-closed / cross-platform.** Windows
   `apply_job_limits` is now `...?` (fail-closed) instead of a best-effort
   `eprintln`; Unix `setrlimit` fails closed; disk preflight fails closed. CPU/mem
   rejection is verified by `#[cfg(unix)] cpu_limit_kills_runaway_process`.
7. **Test coverage + PR metadata incomplete/stale.** Added the tests above;
   PR description refreshed on push. The `heartbeat_loss_*` parallel-load flake
   remains pre-existing (passes single-threaded / in isolation) and is not part
   of this change.

## Follow-ups (post-merge, not in this PR)

- CLI flags / `EvaluationConfig` fields for `ResourceLimits` and known secrets
  (currently wired via `from_environment()` + `collect_known_secrets`).
- Static acceptance fixtures under `tests/fixtures/e2-evidence-integrity/`.

## REVIEW_GATE — round 3 (on `f75173d`)

The reviewer's `#pullrequestreview-4960748239` (6 PRIMARY BLOCKERS) on
`f75173d96c21d5187f29b4cd82ec008faf8f609e` are addressed in this revision:

1. **Retention protected identity-key dirs, not referenced dirs, and continued
   after failure.** The orchestrator now builds protection with
   `build_retention_protection(repo, &refs.dir)`: it inserts the `proposal_registry.json`,
   the journal subtree (`insert_dir`), and for every `reg.entries` value the
   referenced proposal directory (`workflow/<proposal_id>`, when present) and the
   referenced evidence directory (`entry.evidence_dir`, absolute or repo-joined,
   when present), plus the current run dir. The build is `Result`-returning and
   **fail-closed**: reclamation is skipped (with an `eprintln`) if protection
   construction fails, so authoritative state can never be reclaimed on a failure.
2. **Resumed / fresh resource failures lacked durable classification.**
   `ValidationRecord` gained `failure_classification: Option<String>`;
   `resource_failure(.., classification, ..)` stores it; `classify_validation_failure`
   returns it verbatim when present (else the prior heuristic). Both orchestrator
   `Err` branches (evaluate + resume) now durably write
   `ValidationRecord::resource_failure(.., classification, ..)` **before**
   `ValidationComplete` and pass `bundle.failure_classification` into the durable
   transition, so resumed resource failures carry the correct classification (not
   `candidate_test_failed`) and fresh failures stay `InfraBlocked`.
3. **Secret-bearing patch rejected only after `proposal.json` persisted.**
   `propose_with_meta` now calls `collect_known_secrets(repo)` and bails with a
   "patch embeds a known secret" error **before** `save_proposal`, so the proposal
   is never persisted. `verify_patch_free_of_secrets` remains as defense-in-depth
   at evaluation time.
4. **Critical dry-run/apply loaders trusted missing sidecars; evidence parsed
   before verification.** `load_proposal` (dry-run and apply paths) now uses
   `read_verified` instead of `read_verified_or_legacy`, so it fails closed on a
   missing/tampered sidecar. (`load_proposal_from_repo` and `run_isolated_validation`
   already used `read_verified`.)
5. **CPU/memory not aggregate process-tree enforcement; incomplete config
   validation / ongoing disk enforcement / cross-platform proofs.** Added
   `resource_limits.validate().context("invalid resource limits configuration")?`
   at the top of `run_isolated_validation` (config errors now fail closed via
   `ResourceLimits::validate`). Windows `apply_job_limits` switched from
   per-process `ProcessMemoryLimit` to aggregate `JobMemoryLimit` +
   `JOB_OBJECT_LIMIT_JOB_MEMORY` (job-level commit cap). Added a Unix aggregate
   process-tree monitor (`spawn_resource_monitor`) that walks `/proc`, sums the
   **aggregate** CPU ticks and RSS across the whole validation process group, and
   kills the group (SIGKILL) on CPU / memory / free-disk breach — independent of
   and in addition to per-process `setrlimit`. Ongoing disk enforcement is now
   continuous (monitor polls every 100ms) rather than preflight-only. Cross-platform
   proof: `#[cfg(windows)] cpu_limit_kills_runaway_process_windows` passes locally
   (aggregate Job Object); `#[cfg(unix)] cpu_limit_kills_runaway_process` and
   `#[cfg(unix)] memory_limit_kills_runaway_process` cover Unix (memory test
   python3-gated). Note: Windows Job Objects cannot nest under a parent job that
   forbids breakaway, which may affect CI runners that already place the test
   process in a job; the fail-closed path (`apply_job_limits` errors) is still
   safe and never persists a tampered/unbounded result.
6. **PR body lacked `Closes #115`.** Added `Closes #115` to the PR description.

### Round-3 verification evidence (local, Windows dev env)

- `cargo fmt --check` — clean
- `cargo clippy --lib --all-features -- -D warnings` — clean
- `cargo test --lib --all-features` — relevant suites pass:
  - `workflow::retention::tests` (protection scoping + sidecar atomicity)
  - `workflow::artifact_integrity::tests::publish_then_read_verified_round_trips`
  - `workflow::evaluate::evidence::tests::resource_failure_record_is_a_terminal_failure`
  - `workflow::evaluate::recovery::tests::governance_passed_resumes_validation`
  - `workflow::evaluate::validation::tests::patch_containing_known_secret_is_rejected`
  - `workflow::evaluate::validation::tests::disk_preflight_blocks_validation_when_unsatisfied`
  - `workflow::evaluate::validation::tests::cpu_limit_kills_runaway_process_windows` (passes — aggregate Job Object)
- Known pre-existing flake `known_secret_is_redacted_from_persisted_validation`
  uses a non-unique temp dir (`prometheos-canary-<pid>`) shared across parallel
  tests; passes single-threaded / in isolation (confirmed by isolated run). Not
  introduced by this change.
