# ADR: TUIOS-inspired Durable task activity, rehydration and security-safe attention projection

Date: 2026-11-21
Status: Proposed (research-only)

## Problem and scope
Investigate read-only, versioned agent activity events and attention projection from Lite's authoritative execution. Distinguish running-process survival, resumable conversation, and persisted task record. Use event cursor/snapshot contracts and source provenance. OSC 7501 and harness hooks are inputs only, never authority. No direct adoption of TUIOS control socket or permissive shell execution. Historical suspect objects from archived P0 #297 remain untrusted: tainted refs and suspect executable content must not be executed, adopted or merged. #297 is archived (closed as not planned) and is not a blanket prohibition on merging independently reviewed PRs; every merge still requires its own verification, authority and acceptance gates.

## Project-specific acceptance criteria
Draft a contract and test plan for idempotent replay, sequence gaps, stale event rejection, daemon/reboot recovery, untrusted status strings, source rank, per-principal visibility and no side effects from read-only queries. Do not publish runtime code or touch tainted refs.

## Evidence and provenance
Upstream: https://github.com/Gaurav-Gosain/tuios
- Architecture: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/ARCHITECTURE.md
- Agent state and Inbox: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/AGENT_STATE.md
- Program status: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/PROGRAM_STATUS.md
- Rehydration: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/REHYDRATION.md
- Sessions: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/SESSIONS.md
- Protocol: https://github.com/Gaurav-Gosain/tuios/blob/main/docs/protocol.md

Observed upstream: long-lived daemon owns PTYs and session state; clients attach/detach; multi-agent Inbox; OSC 7501 program state; source-ranked agent states; agent messaging; grants; optional read-only MCP; isolated worktree fan-out; snapshots paired with output sequence; different recovery guarantees for detach vs daemon restart. These are source-derived design facts, not evidence that this repository implements them.

## Decision
**Proposed / research-only. Not accepted for production.** Adapt mechanisms and invariants rather than copy TUIOS wholesale. No code, merge, deployment, paid service, new privileged integration or modification to authoritative main is authorized by this ADR.

## Cross-project ownership
- PrometheOS / OS: omnipresent UX and OS-level attention projection.
- PrometheOS Lite: governed execution and task activity.
- SOMA: identity, authorization and capability contracts.
- SOVRA: protected decision and risk policy.
- Mnemosyne: durable provenance and replay.
- HIVE: coordination and bounded agent fleet.
- Foundry: independent evaluation and benchmarks.

## Non-goals and caveats
- TUIOS is not an OS kernel; do not rename or duplicate existing products.
- Explicit status reports may be wrong. Reported completion is not independently verified success.
- A local socket protected by filesystem permissions is not a substitute for remote authenticated principal identity.
- TUIOS normal daemon restart restores layouts and fresh shells, **not** original processes; live upgrade is experimental and platform-limited.
- Its 64KB PTY output ring can lose history after long disconnections; replay gaps must be detectable and visible.
- Command-pattern risk detection does not replace policy enforcement or a sandbox.
- No blanket Git write/merge authority; avoid tainted refs and paid CI.
- A local socket or filesystem permission is not authenticated client identity; any privileged observation/activity transport remains bound to #340's trusted administrator-controlled SID→owner provisioning prerequisite, which is currently unmet, and must not be built on a self-writable mapping.
- Keep source-derived facts separate from proposed project architecture.

## Acceptance gates
1. Owner/Principal approves precise scope and authoritative owner before implementation.
2. Explicit threat model, resource budget and failure/recovery matrix.
3. Tests prove replay correctness, idempotence, source attribution and least privilege where applicable.
4. Independent review of any future runtime changes; no self-merge.
5. Security and integration holds must be cleared independently.
