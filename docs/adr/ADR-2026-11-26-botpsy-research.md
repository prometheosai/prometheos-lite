# ADR: Deferred compact local executive and bounded capability selection

Status: Proposed / research only. Date: 2026-11-26.
Tracking: https://github.com/prometheosai/prometheos-lite/issues/366

Research package BP-07. Registration 2026-11-26. Proposed documentary research only.

## Current evidence and boundary
PR #341 exclusive implementation priority; #133/#136 provenance coordination only; Disktree #364 and UMA #360. SOMA owns contracts, Foundry qualification and Lite execution.

BotPsy is an Apache-2.0 Qwen3.5-derived 0.8B tool-use model, with published BF16/Q8/Q4 artifacts. Publisher reports stronger routine agent performance but limited difficult multi-step research/general knowledge; not independently qualified, English-tagged and not for high-stakes use. Artifact size is not runtime memory; local inference is not authorization to invoke network tools.

## Proposed scope
Document deferred candidate only. After #341 is properly closed and separate owner authorization: evaluate Q8 via existing compatible loopback provider, parser/template pinned, deterministic capability filtering, typed arguments, preconditions, scope, budget, checkpoints and independent effects/receipt verification. Mnemosyne retrieval candidates feed Lite context assembly. No arbitrary shell, write, network, privileged action or automatic escalation. Model never grants permissions. No new harness or runtime by default.

## Acceptance and gates
Synthetic low-complexity tasks; invalid/ambiguous call, prompt injection, stale scope, malformed result, unauthorized proposal vs effect, cancellation, repeated call, failure recovery, no unintended side effects. No model downloads or execution now, no work on #341 branch or new dispatch; research explicitly deferred.

## Sources
https://huggingface.co/blog/qvac/botpsy
https://huggingface.co/qvac/BotPsy-0.8B
https://huggingface.co/qvac/BotPsy-0.8B-GGUFs

Reconcile overlapping issues before dispatch. Preserve zero-cost, security, exact-head independent review and owner gates. No host changes, merges, downloads, training, inference or production activation.

## Decision candidate
Evaluate bounded specialization against unchanged baselines. No mandatory model, normative change or unrestricted autonomy.
