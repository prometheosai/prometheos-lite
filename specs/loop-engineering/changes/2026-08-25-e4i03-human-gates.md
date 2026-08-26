# Change: E4/I03 — human gates, retry edges, classified failure routing

**Issue:** #122 · **Depends on:** #121 ✅ (#191), #118 ✅ (#180). Extends #120 state law additively.

## Scope

`src/workflow/graph_gates.rs`:

1. **`HumanDecisionRecordV1` (`lite.hdecision.v1`)** — durable, authoritative human gate decisions: approved | rejected | changes_requested, gate node, human identity, review channel (interactive CLI or external review system id), basis evidence digest, reason; canonical digest seal; fail-closed parse. Construction REFUSES machine actors (agent/model/provider/harness/tool identities) — approval can never be inferred from model output, and absence of a decision blocks gating (no inference from inactivity).

2. **Graph state addition (additive, backward-compatible)**: `gate_decisions: BTreeMap<node_id, HumanDecisionRecordV1>` on `GraphRunStateV1` + `record_gate_decision(manifest, decision)` which verifies the node IS a declared gate (capability prefix `gate.`).

3. **Routing law extension**: when the source node is a gate, eligibility ignores outcomes entirely and keys off the recorded decision — approved ⇒ edges labeled `approved`; rejected ⇒ ONLY edges labeled `rejected`; changes_requested ⇒ `changes-requested`. No recorded decision ⇒ typed `HumanGatePending` (never inferred). A rejected decision may only route to a TERMINAL exit — continuing after rejection ⇒ `RejectionTerminal` typed error ("new authorized route required" = fresh run/remap, never silent continuation).

4. **Classified failure routing**: canonical labels for failure classes — code ⇒ `failed-code`, infrastructure ⇒ `failed-infra`, policy ⇒ `failed-policy`, evidence ⇒ `failed-evidence`; conditional edges key on these so classes route separately.

5. **Retry edges**: existing `retry`-labeled conditionals + #121 caps (per-node cycle cap, global budget) — fixture proves caps bind retries.

## Acceptance mapping
durable+authoritative decisions ✔ · no inference ✔ · four-way failure separation ✔ · caps on retry ✔ · rejection forces terminal ✔

Out of scope: UI surfaces (#131), external review system integrations (#141), parallel scheduling (#124).

## Verification
cargo test --lib workflow::graph_gates; fmt/clippy/full lib green. No dependency changes.
