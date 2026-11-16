# ADR — Real repository coding as PrometheOS Lite's primary proving case

- **Status:** Accepted owner direction (2026-11-16); execution and benchmark acceptance remain outstanding
- **Owner decision:** Real software-engineering work in a real repository, performed by a real coding model, is the primary proof of Lite + SOMA + Foundry. The illustrative store/inventory/payment scenario is not the product proof.
- **Tracking:** Lite #136, #108, #155; SOMA #133/#146; Foundry #69/#150.
- **Evidence baseline:** merged Lite #136 Slice A/B, including PR #307. Its isolated worktree, governed dispatch and actual `cargo test --offline` validation establish **execution infrastructure**, but its `scripted-fixture` patch is **not** autonomous model coding skill.

## Scope and test design

1. Select an actual reproducible issue in an existing nontrivial repository; freeze base revision, dependencies, toolchain, allowed paths, setup and initial failing behavior. Use disposable workspaces; never modify source checkout in place.
2. Keep reference patches, answer keys, hidden tests and evaluation labels inaccessible to the coding agent. The agent must investigate and author its own actual code changes. No scripted replacement, golden patch injection, or precomputed solution.
3. Exercise logical stages: inspect/root cause → reproduce failing test → model-authored patch → build/test/iterate from observed failures → handoff diff and evidence. These are **logical phases, not a new five-node normative executor**. A genuine run can have variable attempts and feedback loops.
4. Preserve existing Lite authority resolution, file/path policy, workspace isolation, harness identity/revision, policy/compatibility checks, evidence journal, checkpoints and strict failure taxonomy. Capture all failed attempts, refusals, retries, interruptions, recovery and resource use.
5. Create a **matched comparison**: baseline coding agent versus the same model/harness with Lite + SOMA governance; hold repository, issue, test/tool environment, permissions, available context and budgets constant wherever feasible. Declare every controlled/variable dimension. Include a control for extra governance overhead, rather than assuming improvement.
6. Emit actual patch/diff, pre/post tests, validation command/output receipts, changed file/tree digests, adapter/workspace/runtime revisions, authority/effective profile, work events, timestamps and cost/token/latency observations. Recompute the claims; don't use self-attested success alone. Keep signed/verified human decisions separate from agent output.
7. Lite owns execution and reproducible raw export, not metrics verdicts. SOMA owns canonical governed semantics and envelope/evidence checks. Foundry owns held-out tasks, independent grading, matched control comparisons and statistical conclusions.
8. Begin with one real bug-fix pilot, then add a feature and a security/regression repair, and eventually repeated runs across repositories/harnesses. A passing unit test does not constitute a benchmark result.

## Acceptance for the next Lite #136 slice

- [ ] One **genuine model-authored** repair from an unseen issue, in an isolated pinned real repository, with a reproduced pre-fix failure.
- [ ] Actual model-written source diff, recorded intermediate attempts and real post-fix tests.
- [ ] Safe failure and recovery behavior under interruption, duplicate requests and denied writes.
- [ ] Sealed output includes exact evidence identity and can be independently rechecked without hidden model session state.
- [ ] Handoff contract agreed with Foundry #150; Foundry #69 measures a matched baseline run before any advantage claim.

## Explicit non-decisions

This ADR does not grant live publication, production human-disposition authority, a new normative SOMA executor, release authorization, benchmark scores, or a production domain adapter. Existing illustrative store fixtures remain valid *structural research tests* but do not demonstrate product coding capability. Foundry #80 language representation research is independent and not a prerequisite for the real coding pilot.

## Decision provenance

Owner instruction in SOMA conversation on 2026-11-16: "We should use real coding skills in a real-world repo problem, and prove that it does what it was built for." Recorded on Lite #136 and cross-linked to SOMA #133/#146 and Foundry #69/#150.
