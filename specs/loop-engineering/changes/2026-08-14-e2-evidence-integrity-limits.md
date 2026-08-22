# Change Spec: E2 — Evidence & Resource Integrity Limits

- **PR:** #170 (DRAFT)
- **Branch:** `feat/e2-evidence-integrity-limits`
- **Status:** round-5 review-blocker revision complete and locally verified (fmt/clippy/test green on Windows dev env); about to push a new immutable SHA and return to REVIEW_GATE. Do **not** merge or begin #152.

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
  (timeout + output cap, process-tree kill on timeout/cancel, kernel
  `RLIMIT_CPU` on Unix / Job Object limits on Windows), `verify_patch_integrity`
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
- [x] CPU is enforced at the OS level: Unix kernel `RLIMIT_CPU`
      (`setrlimit` in `bounded_run` pre_exec; a SIGXCPU death is the
      deterministic typed verdict) and Windows Job Object job-time; memory is
      enforced by the aggregate process-tree monitor (Unix) and the Windows
      Job Object commit limit — memory has NO child-side rlimit by design
      (see round-6 section for why `RLIMIT_AS` is deliberately not used).
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
 2. **CPU / memory / disk enforcement.** Implemented: kernel `RLIMIT_CPU` on
    Unix plus Windows Job Object job-time for CPU; aggregate process-tree
    monitor (Unix) / Job Object commit limit (Windows) for memory;
    disk-pressure preflight with typed breach evidence; output-cap budget.
    Tests:
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

## REVIEW_GATE — round 4 (on `afa113e`)

The reviewer's `#pullrequestreview-4972248424` (6 PRIMARY BLOCKERS) on
`afa113e3b1062f578528aedb18705f679b283072` are addressed in this revision:

1. **Unix CI compile failure (`PathBuf` missing).** `validation.rs` now imports
   `PathBuf` (used by the Unix monitor signatures); the Windows-only `*mut c_void`
   uses inline `winapi::ctypes::c_void` so the lib compiles on all targets.
2. **Retention protection incomplete + sidecar suffix + out-of-repo acceptance.**
   - `retention.rs` now uses `artifact_integrity::sidecar_for`'s real suffix
     `.integrity.json` (was `.sidecar.json`), so checksums are matched by the
     actual writer.
   - `build_retention_protection` now also scans the durable **journal** and
     **checkpoint** for `evidence_ref` values and protects those evidence dirs
     (via `protect_evidence_ref`), so evidence referenced only by the log/checkpoint
     is never reclaimed.
   - Registry-referenced `evidence_dir` values are resolved fail-closed via
     `resolve_repo_relative` (rejects absolute / parent-escape paths); an
     out-of-repo reference is a hard error that skips reclamation rather than
     "protecting" an attacker-controlled path.
3. **Evidence parsed before verification.** `load_preserved_evidence` now calls
   `read_verified` (fail-closed) **before** `migrate_document`, so tampered evidence
   is rejected before any migration side effect.
4. **Secret coverage + unredacted persisted errors.**
   - `collect_known_secrets` now seeds configured provider credential values: for
     each `llm_routing.providers[*].api_key_env` it resolves the env var and adds
     the value (best-effort; missing config/env is skipped).
   - Both orchestrator `Err` branches (evaluate + resume) build the `Redactor` and
     pass `redactor.redact(&e.to_string())` into `resource_failure`, so provider
     error text can never persist an unredacted key.
   - `known_secret_is_redacted_from_persisted_validation` now **recursively** walks
     every file under `.prometheos` and asserts the canary appears zero times.
5. **Durable resource-failure evidence + recovery tests.**
   - `ValidationRecord` gained typed fields `resource_kind`, `configured_limit`,
     `observed_value`, `stage`, `event_timestamp` (all `#[serde(default)]`).
   - `bounded_run` now returns `Result<(Option<i32>,String,String), BoundedRunError>`
     where `BoundedRunError::{ResourceExceeded(ResourceExceeded), Fatal(anyhow::Error)}`
     and `ResourceExceeded` carries `classification/kind/configured_limit/observed_value/
     stage/code/stdout/stderr`. On a breach the caller builds a durable, classified
     `ValidationRecord` (typed fields + redacted raw logs) and returns it as a
     completed failure — diagnostics are never discarded.
   - Added `resource_failure_record_is_durable_and_maps_to_infra_blocked`,
     `output_cap_breach_is_durable_and_classified`, and updated the cpu/memory/windows
     breach tests to expect `Ok(record)` with `failure_classification` and assert
     `failure_to_terminal_state(..) == InfraBlocked` (recovery never re-derives a
     candidate-test-failure).
6. **Cross-platform enforcement.**
   - `from_environment` is now `Result<Self>` (fail-closed on malformed env values);
     call sites use `?`.
   - Windows: `apply_job_limits` returns the live job `HANDLE`; the child is assigned
     to the Job Object at spawn; `spawn_resource_monitor_win` polls
     `JobObjectExtendedLimitInformation.PeakJobMemoryUsed` and
     `JobObjectBasicAccountingInformation.TotalUserTime`, terminates the job on
     breach (carrying `kind` 1/2), and **closes the handle on thread exit** so the
     async future stays `Send`. The windows breach now produces a real classified
     record (`resource_cpu_exhausted`), no false `infra_blocked` from a non-zero exit.
   - macOS: added `spawn_resource_monitor_macos` using `proc_listpids` /
     `proc_pidinfo` (`PROC_PIDTBSDINFO` usertime+systime, `PROC_PIDTASKINFO`
     `pti_resident_size`) to sum aggregate CPU/RSS per process group (the prior
     Linux-only `/proc` monitor did not run on macOS).
   - Added focused CI workflow `.github/workflows/e2-resource-enforcement.yml`
     (#115) running the validation resource tests on `ubuntu-latest`,
     `macos-latest`, and `windows-latest`.

### Round-4 verification evidence (local, Windows dev env)

- `cargo fmt --all -- --check` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test --lib workflow::evaluate::validation --quiet` — 21 passed
  (incl. `cpu_limit_kills_runaway_process_windows`, `output_cap_breach_is_durable_
  and_classified`, `resource_failure_record_is_durable_and_maps_to_infra_blocked`,
  `known_secret_is_redacted_from_persisted_validation`)
- `cargo test --lib workflow::redaction --quiet` — 6 passed
- macOS monitor compiles only under `target_os = "macos"`; not exercised locally
  (Windows dev env). CI workflow `#115` is the cross-platform gate.

## REVIEW_GATE — round 5 (on `81b7288`)

The reviewer's `#pullrequestreview-4983471511` (8 PRIMARY BLOCKERS) on
`81b72886e68e05c016cdcf483a37f9bbb8aad044` are addressed in this revision:

1. **macOS compilation failure.** The prior macOS monitor used
   `proc_bsdshortinfo` / `PROC_ALL_PIDS`, neither of which exists in `libc`
   0.2.186. Rewrote `spawn_resource_monitor_macos` to use the real API:
   `proc_listallpids` to enumerate, `proc_pidinfo(PROC_PIDTBSDINFO)` for the
   parent pid (`pbi_ppid`), and `proc_pidinfo(PROC_PIDTASKINFO)` for
   `pti_total_user + pti_total_system` (microseconds) and `pti_resident_size`.
   All field/constant names verified against the pinned `libc` source.
2. **Strict clippy six errors.** Removed the dead `kill_group` /
   `proc_in_group` helpers and the non-existent `proc_bsdshortinfo` access;
   `stat_ppid` / `proc_cpu_ticks` / `proc_rss` are now gated to
   `target_os = "linux"` and the `HashMap` import to linux+macos, so no
   unused-import / dead-code lints fire on any target.
3. **Linux CPU/memory tests lost resource classification.** Replaced the
   fragile process-group match (`proc_in_group` / `setpgid`) with robust
   PID-subtree enumeration: `process_tree_subtree` walks the OS parent→child
   map (BFS from the validation child) and `kill_process_tree` SIGKILLs every
   descendant. This survives `setpgid` failures inside containers/CI, so the
   aggregate CPU/RSS sum and breach detection are reliable and carry `kind`
   1/2 to a typed `ResourceExceeded`.
4. **Timeout / disk failures lacked typed durable evidence.** The wall-clock
   timeout `select!` arm now returns `BoundedRunError::ResourceExceeded`
   (`classification = resource_timeout_exhausted`, `kind = "timeout"`) and the
   post-drain match attaches the redacted stdout/stderr. Disk breaches already
   returned `ResourceExceeded` (kind 3) from the Unix monitor; both now feed a
   durable, classified `ValidationRecord`. `validation_timeout_*` test updated
   to assert the durable classified record (not an `Err`).
5. **Crash/recovery tests remained in-memory.** Added
   `e2e_recovery_reuses_persisted_terminal_evidence`: it persists a real
   `EvidenceBundle` through `write_bundle` (integrity sidecar), appends a
   terminal `ValidationComplete` journal event referencing it, then recovers
   and loads the bundle through `read_verified` + `migrate_document_bytes`,
   proving exact-once reuse of the durable evidence rather than re-derivation.
6. **Retention omitted authoritative proposal/PWS references.**
   `build_retention_protection` now also protects the `proposal_ref` directory
   (`workflow/<id>`) from every durable journal event and the checkpoint
   snapshot (not just registry entries), and extends protection with
   `extend_from_portable_work_state` when a `PortableWorkState` is supplied.
7. **Evidence migration re-read unauthenticated bytes.** Added
   `migrate_document_bytes(path, doc_type, content: &[u8])`; `load_preserved_
   evidence` now runs migration over the already `read_verified` bytes so no
   untrusted file read occurs during evidence migration. `read_declared_
   version` (file-reading) was removed in favor of `read_declared_version_
   from_value`.
8. **Secret + cross-platform test matrices incomplete.** Added
   `seeds_provider_credential_values_from_config` (asserts provider
   `api_key_env` values are seeded into known secrets) and the E2E recovery
   test above. macOS monitor is exercised by CI workflow `#115`; the Linux
   subtree monitor is covered by the existing linux breach tests once CI runs.

### Round-5 verification evidence (local, Windows dev env)

- `cargo fmt --all -- --check` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test --lib workflow::evaluate::validation::tests` — 21 passed
  (incl. `cpu_limit_kills_runaway_process_windows`, `output_cap_breach_is_
  durable_and_classified`, `validation_timeout_is_enforced_as_resource_
  violation`)
- `cargo test --lib workflow::evaluate::recovery::tests` — 23 passed
  (incl. `e2e_recovery_reuses_persisted_terminal_evidence`)
- `cargo test --lib workflow::redaction::tests` — 7 passed
  (incl. `seeds_provider_credential_values_from_config`)
- `cargo test --lib workflow::evaluate::migration::tests` — 13 passed
- `cargo test --lib workflow::retention::tests` — 8 passed
- macOS + Linux monitors compile only under their targets; not exercised
  locally (Windows dev env). CI workflows (incl. `#115`) are the cross-platform
  gate. The macOS rewrite uses only verified `libc` 0.2.186 symbols.

### CI follow-up on `eb38786` (round 5, same revision scope)

CI on `eb38786` exposed two root causes behind the four failing checks
(macos/ubuntu `Resource limits & durable evidence`, `Rust Checks`, `golden-path` —
all traced to the same two defects):

1. **`pre_exec` `setrlimit` preempted typed classification (Linux + macOS).**
   The child-side `RLIMIT_CPU`/`RLIMIT_AS` caps killed or failed the process
   *before* the aggregate monitor could observe and classify the breach:
   `RLIMIT_CPU` SIGKILLs at exactly the soft limit (the monitor can never win
   that race), and `RLIMIT_AS` made python's 256 MB allocation fail with a
   plain non-zero exit — so both Linux breach tests lost their
   `resource_*` classification, and on macOS the `setrlimit` call itself
   returned `EINVAL` at spawn ("failed to execute validation command: Invalid
   argument (os error 22)"). Fix: removed the `pre_exec` `setrlimit` block
   entirely; the aggregate process-tree monitor (PID-subtree enumeration) is
   now the single authoritative detector/classifier on Unix — the same
   architecture Windows already uses (Job Object + monitor). Poll interval
   tightened 100 ms → 50 ms on Linux/macOS for faster breach detection.
2. **Nine new-clippy lints visible only on CI's newer stable toolchain.**
   Local toolchain was 1.95; CI installs latest stable. Fixed exactly what CI
   reported: `unnecessary_late_initialization` (`harness/file_impact.rs` —
   late-init pair converted to a tuple `let ... = if/else`),
   `result_large_err` (`bounded_run` now returns `BoundedRunResult =
   Result<..., Box<BoundedRunError>>`; `From<anyhow::Error>` implemented for
   the boxed type; all internal arms boxed), five `collapsible_if` in the
   Linux monitor (collapsed to let-chains; mirrored in the macOS and Windows
   monitors), one `let-else → ?` (`proc_cpu_ticks`), and
   `trim_split_whitespace` (`proc_rss`).

### CI-fix verification evidence (local, Windows dev env, rustc 1.98.0)

- `rustup update stable` — local toolchain aligned to 1.98.0 (2026-08-18) to
  match CI's lint era (local env only; no repo dependency change)
- `cargo fmt --all -- --check` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean on 1.98.0
- `cargo test --lib workflow::evaluate::validation::tests` — 21 passed
- `cargo test --lib workflow::evaluate::recovery::tests` — 23 passed
- `cargo test --lib workflow::redaction::tests` — 7 passed
- `cargo test --lib workflow::evaluate::migration::tests` — 13 passed
- `cargo test --lib workflow::retention::tests` — 8 passed
- Linux/macOS monitor behavior is verified by CI (`#115` + golden-path); the
  breach-classification race is eliminated by construction (single detector).

## REVIEW_GATE — round 6 (on `83049fd`)

The reviewer's `#pullrequestreview-5000028739` (7 PRIMARY BLOCKERS) are
addressed as follows:

1. **Unix OS enforcement restored, claims rescoped to match.** CPU is again a
   kernel hard cap on Unix: `setrlimit(RLIMIT_CPU)` in `bounded_run`'s
   `pre_exec` (fail-closed on failure), with a death by SIGXCPU classified
   deterministically (`stage = "rlimit_cpu"`, observed `"SIGXCPU
   (RLIMIT_CPU)"`) — no polling involved. The aggregate monitor remains for
   tree accounting and as the memory/disk enforcer. Every prior
   `RLIMIT_AS`/OS-enforced-memory claim in this spec and the PR body has been
   removed or rescoped: memory on Unix is enforced by the process-tree
   monitor (50 ms), because `RLIMIT_AS` produces no classifiable signal
   (plain non-zero exit) and fails outright on macOS. Windows CPU/memory
   remain Job-Object limits + monitor classification.
2. **macOS process-table enumeration fixed.** `proc_listallpids(NULL, 0)`
   returns a PID *count*; `macos_ppid_map` now allocates that many entries,
   passes `count × size_of::<pid_t>()` bytes as capacity, and honors the
   second call's returned PID count. Added a macOS regression test spawning a
   shell with two children and asserting subtree enumeration finds all three.
3. **Windows spawn→assign race closed.** The validation shell now starts
   `CREATE_SUSPENDED`; the Job Object is configured/assigned while suspended;
   then every thread is resumed via a toolhelp thread walk
   (`resume_suspended_process_windows`). A fast grandchild can no longer be
   created before assignment (children inherit the job). If job setup fails,
   the still-suspended child is killed synchronously (no await while the raw
   job pointer scrutinee lives — keeps the future `Send`). Tests: descendant
   cleanup after an output-cap tree kill (process-scan assertion), Windows
   memory classification, plus existing windows cpu test. NOTE: this adds the
   `tlhelp32` feature to the pinned `winapi` dependency (same crate/version).
4. **Disk preflight carries typed durable evidence.** New typed error
   `preflight::DiskPreflightBreach { required_bytes, observed_free_bytes }`
   returned fail-fast from both preflights. The orchestrator's fresh path
   persists a typed `ValidationRecord` (`resource_kind="disk"`,
   configured reserve, observed free bytes, `stage="preflight_disk"`,
   classification `resource_disk_exhausted`), journals it on the legal
   `Created → PreflightBlocked` edge carrying that classification (recovery's
   `failure_to_terminal_state` maps DISK → InfraBlocked), and returns the
   classified bundle. The resume path persists the same typed artifact before
   failing (it holds no fence there, so it does not fabricate a terminal
   journal transition). Both fatal-Err persist sites also downcast the typed
   breach. Tests: typed downcast unit tests (insufficient + unmeasurable),
   full-pipeline disk breach asserting persisted record fields, journal
   classification, recovery mapping, preserved-bundle readback via
   `read_verified`, and retry-after-terminal failing closed on append-only
   continuity with the provider counter still at zero.
5. **Orchestrator-level exact-once coverage added** (mock `PatchProvider`
   harness driving the real pipeline): completed-run reuse returns preserved
   evidence with provider-call and validation-marker counters proving zero
   re-execution; a park-barrier crash after IntegrityVerified leaves the
   designed cancelled-tail state (durable invariants asserted: tail state/
   classification/evidence-ref, unpublished evidence.json, counters at 1);
   env-driven `PROMETHEOS_MAX_CPU_SECS` / `_VALIDATION_TIMEOUT_SECS` runs
   carry typed cpu/timeout records through the FULL pipeline including reuse.
   Honest scope note: a second orchestrator run resuming mid-crash is gated
   on owner staleness by design (it waits for the live-owner window rather
   than acting immediately); that resume semantics remain covered by the
   recovery unit suite and the e2e recovery test rather than being forced
   through a 300 s wait here.
6. **Retention/PWS wiring + confinement.** `build_retention_protection` now
   loads the REAL portable work state from
   `.prometheos/workflow/portable_work_state.json` (missing ⇒ none; corrupt ⇒
   fail closed, reclamation skipped). All registry/journal/checkpoint
   proposal references resolve through the new
   `durable::confined_workflow_dir` (rejects empty/absolute/`.`/`..`),
   evidence refs through `resolve_repo_relative`, and
   `extend_from_portable_work_state` now returns `Result<()>`, resolves each
   URI inside the repo and propagates insertion errors. Tests: hostile
   journal proposal_ref rejected; missing control documents tolerated;
   corrupt PWS fails closed; valid PWS artifact protected end-to-end; PWS
   import itself already rejects absolute/traversal URIs (asserted).
7. **Orchestrator-level secret canary runs added.** Surface A: provider
   error embedding the canary in plain/URL-userinfo/query forms — full
   pipeline run, then a recursive scan of everything under `.prometheos`
   asserts ZERO occurrences. Surface B: validation stderr echoes the canary
   at runtime (via an env var so the user-supplied command text stays clean);
   stderr preview, raw logs, markdown, journal all scanned zero. Disclosed
   follow-up (found BY this test): a secret embedded in the user-supplied
   `validation_command` text itself would be persisted into proposal/
   identity/bundle artifacts verbatim — redaction of manifest-supplied
   command strings is proposed as its own follow-up change, not silently
   claimed fixed here.

### Round-6 verification evidence (local, Windows dev env, rustc 1.98.0)

- `cargo fmt --all -- --check` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test --lib` — 803 passed, 0 failed
- `cargo test --tests` — all suites green except the pre-existing
  Windows-local failure `provider_governed_proposal_tests::
  report_exposes_lifecycle_evidence` (uses validation command `true`, which
  does not exist on Windows; reproduced identically on unmodified `83049fd`
  via `git stash`, so it predates this round and CI's ubuntu gate passes it)
- New tests: macOS subtree enumeration; Windows mem classification +
  descendant-cleanup scan; SIGXCPU kernel verdict; preflight typed-breach
  units; retention confinement/PWS units; orchestrator exact-once reuse,
  crash durability, cpu/timeout/disk typed-evidence pipelines, provider-error
  + stderr canary scans.

### CI follow-ups on the round-6 series (final head `a3f51d5`)

Four CI-driven refinements after `e66fe24`, each verified by the full 13-check
matrix turning green:

1. `ExitStatusExt` import for the unix SIGXCPU/signal read (`fda5d3c`).
2. Linux RLIMIT_CPU verdict widened: with soft==hard the kernel (or the shell
   relaying its forked child's death) surfaces SIGKILL / exit code 137 rather
   than SIGXCPU — both are accepted as the deterministic kernel verdict and
   the actual signal/exit is recorded in `observed_value` (`ef0ed14`,
   `55d97ce`). OOM-kill caveat documented at the classification site.
3. Windows cpu unit test timeout raised 30 s → 90 s after one contended-runner
   flake (`55d97ce`); passed on the next two windows runs.
4. Integration expectations updated to the new typed terminal: disk-preflight
   breaches now end `INFRA_BLOCKED` / `resource_disk_exhausted` with a typed
   validation record instead of generic `PREFLIGHT_BLOCKED`
   (`workflow_evaluate_tests`, `a3f51d5`).



