# ADR: Outcome attribution and commitment-safe execution refinements

Status: PROPOSED / RESEARCH ONLY — additive acceptance criteria, not ratification or implementation
Date: 2026-11-24
Baseline: `8240e95eaca7aa8a9b7014c0cb2590fe562b8eb5` (tree `88372cf3944435f4dc5c60eac19a35fc788b8591`)

## Existing work / reconciliation
`src/harness/failure.rs` has a typed failure and recovery taxonomy; #133 already specifies immutable effective ExecutionProfile and CompatibilityDecision; #155 already owns checkpoint portability; #156 owns replayable failure fixtures; #334 owns governed evaluations. `src/flow/idempotency.rs::compute_key` concatenates three strings with colons and hashes with DefaultHasher. Different triples can form identical preimages (e.g. `a:b,c,d` and `a,b:c,d`); DefaultHasher does not specify cross-version persistence compatibility. This is a source-observed encoding risk, not proof a production duplicate occurred. Existing journal/authority owners remain canonical.

## Decision and alternatives
Extend only the specific uncovered acceptance cases below through existing owners. Reject new competing runtime, evaluation or contract implementation. Preserve existing architecture and all owner/security gates. PRs #341, #347, #357; #340 security remains independent. Historical #297 archived; never execute suspect refs.

## Incremental acceptance criteria
- [#343](https://github.com/prometheosai/prometheos-lite/issues/343): Reproduce ambiguous tuple encoding in `IdempotencyKey::compute_key`, check all actual callers and legacy persisted keys. Use versioned canonical structured encoding and a stable digest if durable identity is required; preserve old reads or explicitly refuse incompatible keys. Add RED/GREEN distinct-tuple, JSON object order, malformed/empty input, duplicate claim, crash after side effect before acknowledgement, and uncertain replay tests at the actual owned effect boundary. Hash alone is not atomic admission. Do not alter unrelated journal/authority semantics.
- [#156](https://github.com/prometheosai/prometheos-lite/issues/156): Extend existing FailureKind-to-Fixture mapping: distinguish bad specification, missing context, model capability, flaky verifier, provider/resource, authority and unknown as attributed vs inferred causes. Include exact model/harness/tools/environment/profile and independent verifier identity; stale positive and negative outcomes do not transfer automatically. Test identical error text with different trusted cause, unknown attribution, and no automatic escalation on non-model failure.
- [#334](https://github.com/prometheosai/prometheos-lite/issues/334): In existing governed evaluation receipts, compare first-pass independently accepted work and total bounded resource cost per accepted task, including failed attempts and verification. For zero accepted tasks report undefined/no accepted result, not zero cost. Add deterministic task-class/difficulty/risk fixtures with explicit provenance; labels are metadata, not capability or permission. Compare fixed baseline with an offline hypothetical escalation ladder only, no live policy activation.

## Research provenance
- [Auto Model Router](https://github.com/drewappling/auto-model-router), inspected tree 6e639aa83a838ffc64ae23e33bda4f0fa92f55a2.
- [FreeLLMAPI](https://github.com/tashfeenahmed/freellmapi), inspected tree a54da2121a2e14d1ea52941018bdbb44a5364b8f.
- [Advanced Delegation](https://github.com/NousResearch/hermes-official-plugins/tree/610da681bc9d1ce86b7ce600ace33d3e614d09c9/advanced-delegation), pinned commit 610da681bc9d1ce86b7ce600ace33d3e614d09c9.
FreeLLMAPI server/src/services/key-budget.ts, server/src/lib/fallback-loop.ts, server/src/services/idempotency.ts (completed replay does not imply atomic execution); Auto Model Router src/server/turn.ts and tools/replay.ts (commitment/replay); Advanced Delegation skills/model-routing/scripts/route_ledger.py (failure cause and bounded adaptive hypotheses).

Static prior source inspection; no upstream tests or new benchmarks. OmniRoute is documentation-only and not a source for these criteria. No source copied. Reconcile exact current source and older PRs before coding; if existing tests already prove a criterion, cite exact evidence instead of rebuilding.

## Delivery
OpenCode only for coding work. Existing issue dependency/authority and security gates remain binding. Oldest PR first; then oldest executable issue. Use bounded isolated local fixtures, actual negative tests and exact-head evidence; no paid inference, live credentials, production changes, sibling-repo writes, self-merge or release. Separate reviewed implementation PRs only when a concrete gap is proven. Record blockers once and continue independent work. No tests executed by this documentation registration.
