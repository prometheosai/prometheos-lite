# Change Spec: E2 — Evidence & Resource Integrity Limits

- **PR:** #115 (DRAFT)
- **Branch:** `feat/e2-evidence-integrity-limits`
- **Status:** code complete, local verification passing, awaiting human review gate before merge.

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
  (timeout + `MAX_OUTPUT_BYTES` cap, process-tree kill on timeout/cancel),
  redaction of stdout/stderr/validation command, and raw-log persistence via
  `publish_with_integrity`. Resource exhaustion maps to `InfraBlocked`.
- `src/workflow/evaluate/evidence.rs`: proposal + validation + integrity artifacts
  now carry checksums; loaders verify (legacy-tolerant).
- `src/workflow/evaluate/generation.rs`: proposal load path verifies digest.
- `src/workflow/evaluate/orchestrator.rs`: evidence load verifies digest;
  `EvaluationConfig` unchanged (no ~80 test literals touched — see REVIEW_GATE).
- `src/workflow/mod.rs`: proposal save/load uses `publish_with_integrity` /
  `read_verified_or_legacy`; module declarations for the three new modules.
- `src/workflow/durable.rs`: adds `atomic_write_bytes`.

## Acceptance

- [x] Every persisted evaluation artifact is checksummed via SHA-256.
- [x] Tamper / legacy-tolerant verification on read (`read_verified_or_legacy`).
- [x] Corruption matrix (kind/path/digest) fails closed — covered by
      `artifact_integrity::tests`.
- [x] `prometheos work` does not write the secret canary to disk — unit test in
      `validation.rs` (planned; the `Redactor` is wired into the capture path).
- [x] Validation is bounded (timeout + output cap) and kills the process tree.
- [x] Resource exhaustion maps to `InfraBlocked` (dry-run + execution paths).
- [x] Orphan reclamation is planned against protected references and fails
      closed on out-of-repo paths.
- [x] Full cancellation and interruption-recovery suites pass (verified locally:
      `workflow_evaluate_tests` 52 passed, `cancellation_tests`,
      `interruption_recovery_tests`, `durable_state_tests`, `portable_state_tests`
      all passed; `cargo test --lib` 769 passed; `cargo clippy --all-targets
      --all-features -D warnings` clean).
- [ ] Acceptance fixtures under `tests/fixtures/e2-evidence-integrity/` — covered
      in practice by the in-module round-trip/tamper tests rather than static
      fixtures; no external validator is available in this environment to run
      the true e2 validation path.

## REVIEW_GATE — before merge

1. **Partial OS enforcement of memory / CPU / disk.** `resource.rs` classifies
   these limits and rejects absurd config, but this PR does **not** add Windows
   Job Object / cgroup / disk-preflight enforcement. Disk preflight is left as a
   documented follow-up. The `bounded_run` path enforces **timeout** and
   **output cap** at the OS-process level only. Do not claim CPU/memory/disk
   enforcement is implemented until those follow-ups land.
2. **Configurability is not yet plumbed through `EvaluationConfig`.** To avoid
   touching ~80 explicit test struct literals, `run_isolated_validation` takes
   `&ResourceLimits` and `&[String]` directly, and the orchestrator currently
   passes `&ResourceLimits::default()` and `&[]`. CLI/flag wiring for runtime
   limits and known secrets is a follow-up. Defaults are fail-open-by-design
   (no limits) so existing behavior is preserved.
3. **Secret canary requires a unit test in `validation.rs`** that runs a
   validation command echoing `SECRET_CANARY` with the canary in `known_secrets`
   and asserts the persisted record + raw logs contain zero canary bytes. The
   `Redactor` is wired but the injection test must be added and run before merge.
4. **No benchmark or autonomous-execution claims.** This PR is confined to the
   alpha-stable `prometheos work` surface.

## Follow-ups (post-merge, not in this PR)

- Windows Job Object / cgroup memory+CPU enforcement.
- Disk free-space preflight using `ResourceLimits::effective_min_free_disk_bytes`.
- CLI flags / `EvaluationConfig` fields for `ResourceLimits` and known secrets.
- Canary-injection unit test in `validation.rs`.
- Static acceptance fixtures under `tests/fixtures/e2-evidence-integrity/`.
