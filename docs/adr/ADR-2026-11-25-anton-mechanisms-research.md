# Proposed ADR — Governed behavioral observation and outcome learning

Date: 2026-11-25. Status: PROPOSED, research-only; not accepted or implemented. Owner-requested registration of prior source audit. Repository baseline: `b9553f3f21bb79cc7e73b0b5c54390e8d649398d`.

## Context and evidence
Reference: [Anton pinned source](https://github.com/mindsdb/anton/tree/6b83930e510a984aa822a2cc46a2143a4e94343f), [MindsHub pinned source](https://github.com/mindsdb/mindshub/tree/3f7d3c690295c52b64f512333c59465d20bce0b9). Relevant Anton files: `anton/core/memory/acc.py`, `cerebellum.py`, `cortex.py`, `consolidator.py`, `session.py`, `skills.py`. Deterministic behavioral detectors, selective reflection, context selection, replay and procedural retrieval are useful principles; direct memory encoding, weak text-based failure identity and advisory-only enforcement are not proof of correctness.

Local inspection: docs/research/autonomous-loop-graduation-criteria.md; docs/harness-evidence-completion.md; src/workflow; src/harness. These are selected source observations, not a complete duplication audit or executed benchmark.

## Backlog reconciliation
#156 (failure fixtures), #337 (observation/recovery), #343 (invariants), #133 (resource/routing), #155 (continuation), #136–#139 (qualification); PR #358 overlaps outcome attribution; #340/#341 security gate and all existing PRs retain precedence.

## Candidate register
A = adopt principle; I = investigate improvement to existing mechanism; R = research before adoption. IDs are traceability only, not new issues or approved scope.

| ID | Disposition | Candidate |
|---|---|---|
| LITE-01 | A | repeated equivalent failures |
| LITE-02 | A | no-material-progress loops |
| LITE-03 | A | repair churn |
| LITE-04 | A | reset churn |
| LITE-05 | A | severity escalation |
| LITE-06 | A | actionable diagnostics |
| LITE-07 | I | enforce existing budgets |
| LITE-08 | A | expected/actual outcomes |
| LITE-09 | I | semantic verification |
| LITE-10 | R | selective significant-failure extraction |
| LITE-11 | I | task context compilation |
| LITE-12 | I | mandatory-context preservation |
| LITE-13 | I | progressive disclosure |
| LITE-14 | I | context ceilings |
| LITE-15 | R | planning/coding/summary model roles |
| LITE-16 | I | incomplete tool-history recovery |
| LITE-17 | R | evidence replay |
| LITE-18 | A | pre/post execution hooks |
| LITE-19 | R | bounded scratchpad sessions |
| LITE-20 | I | adapter capability discovery |
| LITE-21 | I | explain decisions |
| LITE-22 | I | cancellation consistency |

## Proposed acceptance and rejection controls
Shadow-only detectors over revision/task/attempt/operation/input identity; distinguish changed input from equivalent retry and technical exit from semantic success. Negative controls: stale receipt, unrelated task, policy denial, exhausted budget, crash after effect. Preserve stable read-only alpha, existing authority, checkpoints, and explicit graduation gates. No automatic memory writes, arbitrary persistent code, or recovery based solely on error text.

## Decision boundary
Use existing project objectives, contracts, backlog, priority order, independent exact-head review and local verification. Reconcile each candidate against current code and issue acceptance before selecting a bounded implementation. No new project, unified cognitive architecture, mandatory integration, new service, external code reuse without license approval, paid services, production effect, host mutation, self-approval or merge. No performance or implementation claims. Reject duplicates; retain unresolved gaps as research. No runtime tests run for this documentation-only registration.
