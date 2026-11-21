# ADR — Agent evaluation, prompt experiments and verified memory

**Status:** PROPOSED / RESEARCH ONLY
**Date:** 2026-11-21
**Issue:** https://github.com/prometheosai/prometheos-lite/issues/334

## Repository-specific proposed decision: Governed evaluation execution and replay
Lite may execute bounded evaluation runs with existing workspace isolation, authority, journal, cancellation, provenance and tool receipts. It must not grant approval because an evaluation score is high or bypass current security constraints.

## Video evidence and limits
The attached 51.6-second video visibly demonstrates a LinkedIn-post agent, prompt versions V3/V4, five example tasks (product launch, hiring post, feature update, event recap, customer story), checks for confident/clear tone, under 120 words, no hashtags/emojis, and a conceptual agent diagram with prompts/evals, tools, MCP, state, memory and guardrails. Audio was not transcribed. These examples do **not** prove a production framework, a measured accuracy gain, or a complete implementation.

## Proposed shared engineering rules
1. Pin prompt, model, tools, policy, guardrails, rubric, fixtures and test harness revisions independently. Record baseline and candidate under comparable conditions.
2. Distinguish deterministic assertions (word count, hashtags, emoji, schema, tool permissions) from subjective quality checks. Calibrate model judges with trusted labels; a judge score alone is not proof.
3. Include representative and adversarial cases, false positives/negatives, context leakage, cancellation, tool failure, cost, latency and rollback.
4. Preserve PASS, FAIL, SKIP, BLOCKED and NOT RUN. Never convert absent tests or self-reported success into a verified result.
5. Verified memory requires provenance, consent, minimization and expiry; a prompt improvement does not imply neural-weight learning.
6. An eval score never grants runtime, financial, security or consent authority. Protected promotion requires the existing approval path.
7. No new paid infrastructure, provider dependency, deployment, merge authorization or security-hold bypass.

## Evidence required before implementation
- Inspect existing contracts and open work to avoid parallel evaluation platforms.
- Define a bounded fixture suite and baseline, versioned run manifest and evidence format.
- Demonstrate correctness, regression, safety and resource comparisons on actual repository behavior.
- Track implementation in a separately reviewed PR; this ADR is not a passing test or production approval.

## Cross-project responsibility
Foundry owns independent evaluation and reproducible evidence; Lite owns governed execution; SOMA owns portable semantics; HIVE owns specialist research; Mnemosyne owns verified memory provenance; SOVRA owns transaction authority; PrometheOS and OS consume read-only status.
