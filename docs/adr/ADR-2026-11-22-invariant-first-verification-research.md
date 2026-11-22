# ADR: Governed execution and crash/retry invariant verification

Status: PROPOSED / RESEARCH ONLY
Date: 2026-11-22

## Project-specific scope
Model one logical consequential operation across timeout, cancellation, retry, crash, replay, and provider/tool uncertainty. A missing reply does not imply failure. Enforce idempotency at the authoritative side-effect boundary; do not rely on a prompt.

## Proposed acceptance checks
Propose local deterministic model checking and fault injection for duplicate tool effects, orphan approvals, stale credentials, and journal replay. Preserve P0 security hold #297; no tainted-ref operations, merges, external calls, or runtime code in this PR.

## Video-derived basis (not independently verified claims)
Source: user-provided video `export_1795372632721.mp4` (11m54s), visually inspected frames. It presents a payment retry/double-charge example; idempotency as an invariant; exploration of execution orders and minimized counterexamples; TLA+-style model checking, Rust/Kani bounded verification, property tests, mutation/regression testing; and a reusable verification Skill. The video cites scale figures and examples that were not independently verified. The cross-project architecture below is a proposed application, not a claim that the video proves these systems correct.

## Decision (PROPOSED / RESEARCH ONLY)
Adopt **invariant-first, independently verified** development as a proposed discipline. Define properties before implementation. Agent proposes code; independent local verifier checks selected properties and returns reproducible evidence. A pass is bounded by assumptions, state space and tool configuration, never universal proof. Do not duplicate existing Foundry/SOMA/Lite contracts. Owner/Principal must accept the ADR and independently review future implementation.

## Verification levels
1. Concrete regression tests for observed counterexamples.
2. Property-based tests over generated inputs, deterministic seeds and shrinking.
3. State exploration/model checking for concurrency, retries, revocation, ordering and crash recovery.
4. Bounded symbolic verification (e.g. Kani for suitable Rust functions).
5. Mutation testing to confirm tests detect intentionally introduced faults, with clean-tree restoration.
6. Independent reviewer checks source SHA, test logs, exclusions, residual risk and evidence provenance.

## Agent-agnostic Skill contract (proposal, Foundry-owned)
Inputs: repo path, scope, risk class, existing tests/specs, explicit permission boundaries, offline budget. Steps: inventory -> extract laws -> select smallest verifier -> generate witness/counterexample -> run local tests -> optional bounded repair -> independent evidence report. Outputs: invariant IDs, source revision, assumptions, tools/versions, seeds, state coverage, test exits, minimized trace, unresolved gaps and approval gates. Limit attempts; do not loop indefinitely. No external paid infrastructure, unapproved Git writes, deploys, privileged changes, self-merge or live financial effects.

## Cross-project responsibility
Foundry owns reusable verification harness/Skill; SOMA owns portable identity and authority; Lite owns governed runtime side-effect boundary; SOVRA owns protected transactions; HIVE owns specialist coordination; Mnemosyne owns provenance-aware evidence retention; OS and PrometheOS own device and user-facing projections; BRAIN owns its Rust model invariants.

## Exit criteria
- Inventory and link existing canonical tests and issues before coding.
- Produce falsifiable laws and negative-control tests that fail when a guard is removed.
- Prove no new privileged side effects from verification.
- Run local offline fixtures with measured resource ceilings.
- Independently review any implementation PR; no self-merge.
- Respect security holds, especially Lite P0 #297.
