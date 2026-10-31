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
  requirement digest, the declared/effective authority digests, the
  evaluation instant, the canonical decision digest, the decision id/status
  and the recorded substitutions.

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
  digest, capability-set digest, requirement digest, authority digest,
  evaluation instant or decision digest does not verify.
- `evaluatedAt` is parsed as RFC 3339 and must be a canonical UTC (`Z`) instant
  that is not a leap second. Separator-shape checks alone are not enough:
  `2026:10:22-12-00-00Z` and `2026-99-99T99:99:99Z` are refused.
- Integrity is not semantic validity. `HarnessSelectionV1::verify_against`
  re-derives every negotiation-input digest from *trusted* values and re-runs
  `resolve()` with the recorded instant, requiring the reproduced decision to
  match the recorded status and canonical digest (`HSEL-0008`). A re-sealed
  substitution therefore still fails closed.
- Only `DecisionStatus::Compatible` is selected. `CompatibleWithDegradation` is
  refused like `Incompatible`, and any other status is a hard refusal rather
  than an implicit accept.
- Evidence paths are panic-free: sealing and digesting selection evidence are
  fallible, and every untrusted-input verification path (declaration, outcome,
  checkpoint, selection) uses the fallible digest. The remaining
  `compute_digest()` panic wrappers are reachable only from Lite-constructed
  values.
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
| `HSEL-0008` | evidence does not verify against trusted inputs / reproduced decision |

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

### Review repair rounds

1. *Evidence binding* — selection evidence recorded the decision id and status
   but discarded the negotiation basis. Added `evaluatedAt`,
   `authorityDeclaredDigest`, `authorityEffectiveDigest` and `decisionDigest`,
   with a regression proving two selections over the same harness but different
   authority/instant produce distinct evidence.
2. *Timestamp and semantics* — replaced positional character checks with RFC
   3339 parsing (UTC `Z` only, leap second refused), restricted selection to
   `DecisionStatus::Compatible`, and added `verify_against` so evidence is
   checked against trusted inputs and a resolver replay rather than only
   against itself. Negative tests cover separator confusion, invalid calendar
   dates, leap years and out-of-range clock values; a re-sealed authority
   substitution and a forged decision digest both fail `HSEL-0008`.
3. *Panic audit* — `HarnessSelectionV1` sealing is fallible
   (`try_sealed`/`try_compute_digest`); declaration, outcome and checkpoint
   verification paths use fallible digests, so untrusted input cannot reach a
   panic wrapper.

Round 3 evidence: selection suite 9/9, conformance suite 5/5, formatting exit
0, Clippy `-D warnings` exit 0, `scripts/local_ci.py run --suite core` exit 0,
MG2 regressions 6/6.

## Budget note

Exceeds the default 200-net-line preference (implementation + six-case
integration suite + this record). The escalation is explicit: the binding is
meaningless without the refusals it adds, and each refusal needs a case that
proves it happens before the adapter is touched.
