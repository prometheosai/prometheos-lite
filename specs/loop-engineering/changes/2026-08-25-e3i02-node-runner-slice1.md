# Change: E3/I02 — generic governed NodeRunner (slice 1)

**Issue:** #117
**Depends on:** #116 ✅ (merged #179); builds on #118 ✅, #152 ✅, #159/#161 ✅.

## Slice 1 scope

The generic node execution interface with the mandatory nine-gate pipeline, implemented over existing Lite infrastructure:

| Gate | Enforcement |
|---|---|
| 1. resolve declared capability | `CapabilityRegistry` — unknown names fail closed (`SOMA-AUTH-0005`) |
| 2. validate typed args | declared required-arg keys (`SOMA-CMP-0003`) |
| 3. authorize | `lite.policy.v1`: `resolve_effective` + `enforce_before_effects` before any effect |
| 4. constraints | bounded-by-declaration check (retry budget present); deep sandboxing remains the orchestrator resource layer |
| 5. execute | the ONLY delegation point (handlers, later: nested nodes/tool bridges/adapters) |
| 6. verify result/state | non-empty verified output; typed `OutcomeKind` |
| 7. redact protected material | `Redactor` with known secrets |
| 8. retain evidence | digest-bound artifact BEFORE journaling |
| 9. journal durable order | digest-chained `JournalEntryV1`, terminal results reference the journaled entry |

Plus:
- **Gate transition law** (`validate_gate_transition`): only the immediate next stage is legal; skips/backwards rejected.
- **Idempotency**: same identity key ⇒ cached outcome, handler runs exactly once.
- **Terminal ⇒ durable evidence**: result's `evidence_refs[0].event_digest == journal entry digest`; result sealed with `result_digest`.

## Out of scope (follow-ups)

- Nested-node/code-mode/tool-bridge/external-adapter bypass-proof fixtures (#154 ExecutionHarness adapters; slice 2).
- Wiring Fast Governed Loop V1 orchestrator to route through NodeRunner (slice 2/3 — acceptance row stays open until then).
- SOMA#80 portable governed-run mappings (upstream unpublished).

## Verification plan

`cargo test --lib workflow::node_runner` (7 tests), fmt/clippy -D warnings/full lib suite. No dependency changes.
