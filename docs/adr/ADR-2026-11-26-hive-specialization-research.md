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
