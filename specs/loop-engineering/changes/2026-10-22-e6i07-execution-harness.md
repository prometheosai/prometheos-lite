# Change: E6/I07 Slice A — governed execution-harness seam + conformance foundation

**Issue:** #154
**Depends on:** #117/#118 (governed NodeRunner gates) ✅, #171 (`ExecutionWorkspace`) ✅, #159 (SOMA++ contract families) ✅.

## Scope

Versioned Rust-native `ExecutionHarness` seam so a harness is a replaceable
worker, never the durable authority:

- **`HarnessDeclarationV1` (`lite.harness.v1`)** — implementation-owned,
  sealed, fail-closed. Identity (name + revision) maps to the published
  SOMA++ `HarnessAdapter.identity`; `declaredCapabilities` /
  `capabilities` map to the same document fields (claim ⊆ declaration, as in
  SOMA-ADAPT-0001); the SPEC 007 `RuntimeCapabilitySet` is embedded and its
  identity must equal the declaration's. Closed `HarnessFamily` set
  (`native`, `subprocess`, `local-model`, `remote-service`,
  `protocol-bridged`) — a new family is a schema change.
- **`ExecutionHarness` trait** — `declaration`, `run`, plus `checkpoint` and
  `resume` whose DEFAULTS refuse. An adapter that does not declare
  `checkpointExport` / resume cannot claim continuity by silence.
- **`HarnessDispatch`** — the only sanctioned effect path. The caller fixes
  the governed context (manifest, local restrictions, known secrets) and the
  runner; the adapter chooses only which DECLARED capability to invoke.
  `call` delegates to `NodeRunner::execute`, so the nine gates cannot be
  bypassed by an adapter that "just needs to run a process".
- **`HarnessCheckpointV1` / `HarnessContinuation`** — bind
  harness id + revision + a 64-hex PORTABLE state digest. No process, PTY,
  chat or conversation state is carried or required.
- **`HarnessRegistry` → `HarnessCatalog`** — the catalogue is generated from
  the declarations the implementations return (ordered by `harnessId`,
  digest-sealed), so there is no separate catalogue file to drift.
  Duplicate ids and unsealed/mis-digested declarations are refused at
  registration.
- **Conformance kit** (`conformance_report`, `HCONF-0001…0024`) — executes
  the adapter through a caller-supplied dispatch rather than only inspecting
  its declaration, and proves declaration/behaviour agreement.

## Invariants enforced (fail closed)

- An adapter cannot invoke a capability outside `declaredCapabilities`; the
  refusal happens before the runner is touched and nothing is journaled.
- A declaration is metadata, never authority: negotiation, permit
  enforcement and the nine gates are unchanged and still decide.
- `claimedCapabilities ⊄ declaredCapabilities` is refused at parse and at
  conformance.
- `resume_support` without `checkpoint_import` is an incoherent declaration
  (HCONF-0007); a declared export that cannot export is HCONF-0019.
- A checkpoint from another harness or revision is refused; a checkpoint
  bound to a different portable state digest is refused.
- The catalogue is order-independent and byte-identical for the same
  declaration set.

## Non-goals honored

No concrete external adapter (Claude Code, Codex, Goose, OpenCode, Aider),
no transport, no scheduler, no harness-selection promotion (#133), no
autonomous-loop graduation. No dependency changes. `prometheos work` is
untouched.

## Out of scope (follow-ups)

- Real adapter families consuming this seam (`subprocess`, `local-model`,
  `remote-service`, `protocol-bridged`) plus their declared limits.
- Harness selection from declared capabilities (#133) and wiring into the
  graph executor.
- A generated on-disk catalogue/schema artifact if an external consumer
  needs one; the declaration-driven catalogue makes an in-repo copy
  redundant today, and adding one later must add a drift check.
- Async/streaming effect paths (`NodeRunner::execute_async`) and
  cancellation propagation; this slice fixes the lifecycle shape, not the
  transport.

## Verification plan

`cargo test --test execution_harness_conformance` — four tests:

1. two distinct implementations (in-process native, protocol-bridged
   scripted) pass the conformance kit;
2. an incoherent continuity declaration (resume without import/export) is
   rejected with HCONF-0007/HCONF-0023;
3. an undeclared capability is refused before the runner or journal is
   touched;
4. duplicate/unsealed registration is refused and the catalogue is
   order-independent.

Plus `cargo fmt --check`, `cargo check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test`, and the repository evidence
runner (`core`, `platform`, `smoke`).

## Budget note

This slice exceeds the default 200-net-line preference. The escalation is
explicit: the seam, its fail-closed declaration, the generated registry and
the conformance kit are one bounded deliverable, and a smaller version would
ship an interface without the evidence that it can be implemented twice.
