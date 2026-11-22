# ADR — DeepSpec-inspired speculative decoding research

**Status:** PROPOSED / RESEARCH ONLY, not accepted for production
**Date:** 2026-11-21
**Tracking:** [Issue #332](https://github.com/prometheosai/prometheos-lite/issues/332)
**Reference:** https://github.com/deepseek-ai/DeepSpec

## Context
DeepSpec explores cheap draft-token proposals with target-model verification, including EAGLE-3, DFlash and DSpark. Correct speculative sampling requires rejection correction to preserve the target distribution. Gains depend on acceptance, verification cost, batch throughput, memory and device characteristics; no reported gain transfers automatically to this project. Token-level speculation is not collective specialist intelligence or continual learning.

## Proposed decision: Feature-gated runtime feasibility
Lite may consider an optional speculative decoding adapter only after target/drafter compatibility, correct target verification, bounded cancellation/timeouts, safe fallback, and resource budgets. Preserve workspace isolation, execution journal, authorization and the existing security hold. Never merge, enable or deploy from this research ADR. Rejections and unavailable backends must not silently change output contracts.

## Boundaries
1. This is research planning, not a runtime implementation, benchmark pass, adoption or release authorization.
2. No speculative answer, confidence score, cached memory or capability declaration grants execution, financial, security or consent authority.
3. Keep current repository boundaries, security holds, protected flows and main branch intact. Do not add external paid infrastructure.
4. Any future prototype must preserve target-verifier correctness, compatibility, cancellation, privacy controls and a normal-decoding fallback.
5. Benchmark against pinned normal decode and simple fixed-length speculation before adopting adaptive confidence scheduling.

## Candidate evaluation
- Review upstream source at a pinned revision and verify actual algorithm specifics and license before code reuse.
- Pin target and drafter identities, tokenizer, sampling, datasets, seeds, concurrency, hardware and software environment.
- Test deterministic parity where applicable, proper rejection-correction behavior, sampling distribution, failure/cancellation and safe fallback.
- Record accepted tokens per target step, TTFT, p50/p95 latency, throughput, cold/warm behavior, resource footprint and provenance.
- Classify each result as PASS, FAIL, SKIP, BLOCKED or NOT RUN. Do not claim speedups without reproduced data.
- Scope any implementation to a subsequent independently reviewed PR, linked to issue #332.

## Cross-repository ownership
**Foundry:** benchmarks and independent evidence. **SOMA:** portable contract. **Lite:** optional runtime adapter. **PrometheOS / OS:** consuming orchestration and UX. **Mnemosyne:** memory/privacy provenance. **HIVE:** distinct specialist-level research. **SOVRA:** hard transaction authority boundary.

## Consequences
No code, runtime behavior, integration contract or authorization state changes in this ADR. The cost of an experiment is explicit; measurable evidence is required before any promotion.
