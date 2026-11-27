# ADR: Deferred governed tool lifecycle for heterogeneous specialists

Status: Proposed / research only. Date: 2026-11-26.
Tracking: https://github.com/prometheosai/prometheos-lite/issues/370

Research package HSCC-08. Owner requested registration 2026-11-26. Proposed documentary research only.

## Current boundaries and related work
PR #341 exclusive implementation priority; #133/#136 bounded provenance coordination; BotPsy #366/#367, decision models #368/#369 deferred.

## Hypothesis and proposal
Documentation only. After #341 properly closed and separate owner authorization, reconcile existing typed action contracts and evaluate deterministic eligibility -> bounded selection -> argument extraction/generation -> schema/semantic precondition verification -> authorized execution -> independent receipt and state update. HIVE may propose specialist results or task DAG but cannot authorize, schedule privileged host work or execute effects. Distinguish Laya/Kev selection-only vs BotPsy complete proposal, direct deterministic path when possible, strict scoped evidence and budget, cancellation, no unsafe fallback. Keep Lite execution and context assembly; no new harness/daemon, no duplicated Foundry certification or SOMA semantics.

## Acceptance and gates
Deferred synthetic fixtures: invalid tool/arg, unauthorized proposal, stale observation/lease, dependency failure, timeout/replay, cancellation, budget and deterministic verification. No implementation dispatch or work on #341 branch, no model run/download or host mutation.

## Evidence and sources
https://github.com/NandhaKishorM/laya/blob/main/docs/finetune_browser_agent.md
https://huggingface.co/cklxx/laya-browser
https://huggingface.co/qvac/BotPsy-0.8B
https://github.com/jaredpalmer/kev
https://arxiv.org/abs/2303.17580
https://arxiv.org/abs/2101.03961
https://arxiv.org/abs/2305.05176
https://arxiv.org/abs/2406.18665
https://github.com/PaddlePaddle/PaddleOCR

Cross-project references: HIVE #296, Foundry #246, SOMA #234, Axiom #38. Research precedents motivate but do not prove HIVE superiority; selection is not complete tool execution. Prior source inspection and publisher-reported data only; no independent run. Verify exact source revision, licensing and dependencies before reuse; no code copied. Reconcile existing BotPsy, decision-model and UMA/Disktree work before dispatch. Preserve zero-cost, security, storage, exact-head independent review and owner gates. No merges, paid services, downloads, inference, training, host changes or production activation.

## Decision candidate
Test the hypothesis under controlled workloads; retain deterministic baselines and negative results. No mandatory model, parallel execution or authority expansion.


## Laya/Vega ArchiLens addendum (2026-11-26)
Proposed research only. Laya 0.5.5 commit 8f445c623453f7514f30fe958a73878296aea9af; actual tree 68dd3bccee15c84c17edda368aab46dbf0356906. VegaML source 0.20.1 commit cca98c8ac0ab398cf35ec8d7c89050f5f12836ec; tree c82157a684ea3c92e39347b51bc58572558e6b9f. Since 0.5.0 Agent() defaults to Vega if available; explicit classic engine/checkpoint required. Laya/VegaML core and classic checkpoints Apache-2.0; vegaml-learn PolyForm Noncommercial 1.0.0, no commercial adoption or restricted code reuse. Verify resolved dependency/model/runtime/licensing and artifact digests before use. https://github.com/NandhaKishorM/laya/tree/8f445c623453f7514f30fe958a73878296aea9af ; https://github.com/NandhaKishorM/vegaml/tree/cca98c8ac0ab398cf35ec8d7c89050f5f12836ec ; https://huggingface.co/convaiinnovations/laya ; https://doi.org/10.20944/preprints202611.0781.v1 (not peer reviewed). No independent run or qualification. HIVE #296, Foundry #246, SOMA #234 coordination/evaluation/semantics. Preserve all existing owner/security/storage/zero-cost/independent review gates; no downloads, training, inference, host/Cargo changes, merges, paid operations or activation.

DEFERRED: PR #341 exclusive priority, no unrelated implementation until properly closed and separately approved. LayaToolSelector.select is reference only; call() directly invokes caller-supplied args and is not authorization/argument generation. Reconcile existing contracts: deterministic eligible capability IDs, bounded choice, validated externally extracted/generated args, permission/precondition/budget/cancel and receipt; no direct call helper adoption. Reject stale/missing artifact provenance, truncated/colliding options, unknown finalists, conditional probabilities as full set, unqualified calibration and unreachable band (no weaker auto fallback). Laya classic Rust ONNX CPU reference with offline parity only after authorization; Vega optional explicit engine, no implicit fallback/download. Guardrails not security authority, document injection misses. No code/activation, #133/#136 scope unchanged.
