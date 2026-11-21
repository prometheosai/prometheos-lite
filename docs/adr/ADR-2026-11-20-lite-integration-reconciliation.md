# ADR-2026-11-20 — Reconcile Lite integration, persistence ownership, and evidence boundaries

- **Status:** Proposed; recording integration decisions, not granting merge/release authority
- **Tracking:** #328
- **Related PRs:** #318, #321, #322, #323, #324, #325, #326
- **Other repositories:** [SOMA #188](https://github.com/prometheosai/soma/issues/188), [Foundry #194](https://github.com/prometheosai/foundry/issues/194), [PrometheOS #3](https://github.com/prometheosai/prometheos/issues/3)

## Context

On 2026-11-20 the repository owner closed security incident #297 as not planned and stopped the active investigation. The archival receipt records owner testimony about a ChatGPT connection; authenticated attribution, execution/exposure and credential scope remain unverified. **Administrative closure is not security remediation or license to execute tainted history.** Existing forensic records and refs are not rewritten by this ADR.

The live PR comparisons against `main` at `0b0063cfb1aeac5bfa0f605f40a759515793283a` show:

| PR | Observed relation | Integration decision |
| --- | --- | --- |
| #318 | 13 ahead / 4 behind; includes Windows monitor and validation changes beyond initial path-guard work | Reconcile exact commit range and target branch; prove clean ancestry and run Windows regression, resource and path tests before merge |
| #321 | 6 ahead / 0 behind | Adopt as migration lifecycle candidate only after SQLite upgrade/rollback/foreign-key and compatibility checks |
| #322 | independent of #321 (3 ahead, 6 behind when comparing #321 head to #322 head) | Documentation is logically dependent on the implemented #321 contract; align exact claims with merged code before integration |
| #323 | independent single-commit journal seam | Integrate on own merits with journal/digest and recovery regression evidence |
| #324 | 2 ahead / 0 behind | Independently check requirements hashes and isolated Python CI preflight on supported platform, without implicit installs |
| #325 | diverged from #323; overlapping `src/db/repository/work_run_metrics.rs` and `src/work/mod.rs` | After #323, reconcile metrics persistence and verify strict corrupted-row refusal |
| #326 | direct descendant of #325 (+5 commits, no commits behind) | Preserve stack semantics: integrate #325 then validate only remaining #326 changes against new main; avoid duplicate squash/replay |

These relationships are snapshots, not proof that a branch remains unchanged at merge time.

## Decision

1. **Integration is SHA-pinned, oldest-first where dependencies allow.** Re-fetch HEAD/main and reassess conflict/ancestry at every merge. No bulk merge on metadata alone. Reject unexpected file changes or contaminated history. Do not close an issue solely because code exists.
2. **Database schema ownership belongs to `db`.** #321's migration ledger/transaction lifecycle is the candidate canonical mechanism; #322 must reflect the exact accepted implementation, including foreign-key suspension/restore, rollback, forward-version refusal and read-only reopening where applicable. Test historical and corrupt fixtures before accepting.
3. **Domain-owned data contracts precede further inversion.** #323 defines the journal sealing seam. #325 defines metrics persistence and strict row decode. #326 extends approval/autonomy and work-context parsing. Preserve durable records, canonical digests, isolation and fail-closed semantics. Never silently fabricate values from corrupt rows.
4. **Stack discipline.** #326 contains #325 history. #323 is a sibling branch and needs explicit reconciliation. Resolve on approved clean branches, not by blindly merging every branch independently.
5. **Security and validation remain independent.** #318's path-guard work cannot be conflated with its Windows monitor changes or external `patch(1)` residual TOCTOU issue #327. Security tests and residual-risk statements must be verified independently. #324's wheel hashes are installation inputs, not evidence of runtime correctness.
6. **Authority never originates in persistence, UI or evidence.** SOMA defines portable ExecutionProfile, compatibility negotiation and governed-run contracts. Lite implements governed work and storage; the runtime must fail closed on unrecognized capability/version/authorization, record effective choices and avoid widening permissions on fallback.
7. **Foundry evaluates, Lite emits evidence.** Foundry owns reproducible benchmarks, aggregation, certification and claims. Record exact integrated SHA, fixtures, environment, skipped gates and externally blocked acceptance before asserting conformance.
8. **PrometheOS consumes, does not fork.** The product/control-plane surface consumes versioned Lite/SOMA contracts; it does not duplicate the governed execution kernel, SQLite migrations, evidence journal or permissions.

## Integration evidence required

- Exact PR HEAD, merge base, changed-file list and unexpected changes assessment.
- No suspect #297 ancestry in integrated branch; preserve forensic evidence.
- Rust format, clippy, locked workspace tests and focused DB, journal, corruption, path and Windows tests relevant to changes.
- Migration up/down compatibility is forward-only and fail-closed; verify upgrade/rollback/retry and corrupt-history refusal.
- Explicit platform/CI limitations rather than implied cross-platform success.
- A post-integration full-main verification receipt and issues/PR updates.

## Consequences

This ADR is an **integration record**, not a declaration that the seven PRs are reviewed, merged, secure or accepted. Any release or authority-changing operation remains subject to its separate gate. Update this ADR through a reviewed PR when actual merge SHAs and evidence become available.
