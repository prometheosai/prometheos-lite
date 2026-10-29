# Change: E6/I07 Slice B — harness selection through SPEC 007 negotiation

**Issue:** #154
**Depends on:** Slice A (`b710e9da66`, ancestor of `31fd2840f9`), SPEC 007 /
SOMA `#83` negotiation core (`src/workflow/soma/capability.rs`), #117/#118
governed gates (unchanged), #171 `ExecutionWorkspace` (unchanged).

## Why this slice

Slice A shipped the seam, the declaration, the catalogue and the conformance
kit, and explicitly left harness selection as a follow-up. That left four #154
acceptance lines unmet in a way Slice A could not reach:

- unsupported capability/version/workspace combinations failed at *dispatch*
  time, not at *selection* time;
- harness selection and revision were not recorded in durable evidence;
- the SOMA `#83` negotiation existed but the harness seam was not wired to it;
- the substitution prohibition (`SOMA-CAP-0004`) was enforced inside
  `resolve()` and therefore unreachable from the harness surface.

## Scope

One binding, no new adapter and no new transport:

- **`HarnessSelectionRequest`** — the work's `WorkRequirements`, the
  declared/effective authority pair, an injected evaluation instant, the
  proposed substitutions, and an optional pin.
- **`HarnessRegistry::select`** — runs every candidate's declared
  `RuntimeCapabilitySet` through `soma::capability::resolve` and returns
  exactly one `HarnessSelectionV1`.
- **`HarnessSelectionV1`** — digest-sealed durable evidence binding
  `harnessId`, the published identity `name`/`revision`, the closed
  `HarnessFamily`, the declaration digest, the capability-set digest, the
  requirement digest, the decision id/status and the recorded substitutions.

## Invariants enforced (fail closed)

- An adapter cannot be selected for work whose required capabilities its
  declaration does not cover — refused before execution (`HSEL-0003`, with the
  negotiation diagnostics carried in the refusal).
- Selection is explicit. Two compatible candidates are an ambiguity
  (`HSEL-0004`); the caller pins one. Registration order is never authority.
- A pin is a requirement, not a hint: an unregistered pin is refused
  (`HSEL-0002`).
- A substitution the work forbids is refused by `SOMA-CAP-0004` and emits no
  selection evidence.
- The declaration stays metadata: the decision comes from the published
  resolver, never from the adapter's claim.
- Selection never invokes the adapter. The test adapters refuse to run, so any
  invocation would fail the suite.
- Evidence is self-verifying (`HSEL-0005`): a rewritten revision, declaration
  digest, capability-set digest or requirement digest does not verify.
- The seam reads no clock; `evaluatedAt` is injected.

## Refusal codes

| Code | Meaning |
| --- | --- |
| `HSEL-0001` | no harness registered |
| `HSEL-0002` | pinned harness not registered |
| `HSEL-0003` | no registered harness satisfies the requirements |
| `HSEL-0004` | ambiguous: more than one compatible candidate, unpinned |
| `HSEL-0005` | selection evidence does not verify |
| `HSEL-0006` | empty selection id |
| `HSEL-0007` | canonical digest of an input could not be computed |

## Non-goals honored

No concrete external adapter, no transport, no scheduler, no
harness-selection promotion (#133), no autonomous-loop graduation, no
dependency change, no `prometheos work` change. The nine gates, the permit
enforcement and `HarnessDispatch` are untouched: Slice B decides *who* may run
*before* any of them is entered, it does not replace any of them.

## Out of scope (follow-ups)

- Wiring `select()` into the graph executor and the `prometheos work` path.
- Real adapter families (`subprocess`, `local-model`, `remote-service`,
  `protocol-bridged`) and their declared limits.
- Capability data imported from Foundry evaluation artifacts.
- Async/streaming effect paths and cancellation propagation.

## Verification evidence

- Red first: `cargo test --test execution_harness_selection` → exit 101,
  8 errors, all "`HarnessSelectionRequest`/`HarnessSelectionV1`/`select` do
  not exist" (`C:\154-verify-logs\sliceB-red.log`).
- Green: same command → 6 passed, 0 failed, exit 0
  (`C:\154-verify-logs\sliceB-green1.log`).
- `cargo fmt --check` → exit 0.
- `cargo check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test`, and `scripts/local_ci.py run --suite core|platform|smoke`
  recorded on the candidate commit before review.

## Budget note

Exceeds the default 200-net-line preference (implementation + six-case
integration suite + this record). The escalation is explicit: the binding is
meaningless without the refusals it adds, and each refusal needs a case that
proves it happens before the adapter is touched.
