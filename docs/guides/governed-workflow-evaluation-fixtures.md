# Governed-workflow evaluation fixtures (Lite-owned, experimental)

Issue #136 Slice A delivers the first repeatable, governed-workflow evaluation
fixtures for PrometheOS Lite. Slice B executes one bounded repair task end to
end on those bytes. Slice C adds a second executed task type (documentation)
and a replayable failure → replay → verified-repair evidence chain. Slice D
executes a genuine model-authored repair in an isolated pinned workspace
(synthetic, declared). This page
documents what Lite owns, what it deliberately does not own, and the exact
status of the Foundry boundary.

**No benchmark score, pass rate, comparison, certification or public claim is
produced by these fixtures. There is no Foundry integration claim.**

## Ownership boundary

| Concern | Owner |
|---|---|
| Executable product scenarios (pinned repositories, tasks, expected outcomes) | **Lite** |
| Fixture content pinning (content digests, fixture revisions) | **Lite** |
| Deterministic local fixture runner around the governed dispatch boundary | **Lite** |
| Raw evidence records (route evidence, snapshot, attempt, journal entry, accounting, graph) | **Lite** |
| Scenario descriptor schema (`lite.fixtures.scenario`) and run-envelope namespace (`lite.evidence.scenario-run`) | **Lite** (internal) |
| External benchmark runner, matrix aggregation, metrics, comparisons, certification, public-claim policy | **Foundry** |
| The Foundry external evaluation format (unpublished) | **Foundry** |

Lite produces tasks and raw evidence. Foundry owns everything downstream of
that. Nothing in this repository may present Lite-internal envelopes as a
Foundry contract, and no code path treats a Lite-internal record as a
benchmark result.

## What Slice D adds

Slice D executes a **genuine model-authored repair pilot** (#136 Slice D,
owner-ratified ADR `docs/adr/ADR-2026-11-16-real-repository-coding-proof.md`)
on a synthetic, declared OSS-style task. It is the first slice in which the
repair is authored at run time by a real model — never by a fixture script —
and it is **not** a benchmark, comparison or Foundry claim.

- **The task (synthetic, declared)** - `lite-rust-tokenbucket-bugfix-model-001`
  (`tests/fixtures/scenarios/rust-tokenbucket-bugfix/`, two descriptors:
  `descriptor.json` for deterministic offline tests, `descriptor.live.json`
  for the opt-in live pilot). A small keyed token-bucket rate limiter
  (`src/bucket.rs`, `src/clock.rs`, `src/limiter.rs`, `src/lib.rs`,
  `tests/rate_limit.rs`) in which the refill bookkeeping discards the
  sub-window remainder. `cargo test --offline` fails pre-fix
  (`refill_accumulates_sub_window_remainder`, exit 101). It is a synthetic,
  self-contained crate — deliberately NOT an external held-out repository
  bug — and that weaker provenance is stated here explicitly. No reference
  patch, answer key or hidden test is visible to the solving model; the
  editable set is `src/bucket.rs` only; the reference repair is used only
  by the offline canned path to emulate a successful response.
- **Proven pre-fix failure** - the pilot actually runs the pinned validator
  (`cargo test --offline`) in a throwaway copy of the staged source before
  any model dispatch and records exit code, output digest, output byte
  count and the captured tail (the public failure signal a CI log shows).
  `preFixProbe.exitCode == 101` on every run; a sealed envelope never exists
  for a run whose pre-fix probe is missing.
- **The model (live, opt-in, $0)** - `PROMETHEOS_REAL_MODEL_PILOT=1` plus an
  already-configured `OPENROUTER_API_KEY` runs
  `nvidia/nemotron-3-ultra-550b-a55b:free` (OpenRouter free tier; prompt and
  completion price 0, verified against `GET /api/v1/models` on 2026-11-16).
  Lite's governed local-model harness talks to a loopback operator gateway;
  the gateway alone holds the key, forwards to OpenRouter and rewrites the
  model identity to the harness-visible alias `nemotron-ultra-550b-free`.
  The harness, route resolution, capability boundary, journal, accounting,
  file policy and sealing are all the normal Lite-governed path; the
  gateway sets `reasoning.enabled=false` as operator policy because the
  reasoning-enabled variants of this class of model consumed the entire
  completion budget and returned `content: null`. A first preliminary run
  with `cohere/north-mini-code:free` (also $0) produced three no-op edits
  ("Edit 0 has identical search and replace content") that were all
  rejected by the edit boundary before any write; that run sealed nothing,
  applied nothing, and is reported here as an observation, not as a result.
- **The real result (Windows, 2026-11-16)** - one attempt, 1 model-authored
  `search-replace` edit to `src/bucket.rs`
  (`self.last_refill = self.last_refill.saturating_add(ticks.saturating_mul(self.tick_millis));`),
  applied under the single-file write authority, pinned validator
  `cargo test --offline` exit 0, `taskOutcome: repaired`,
  `classification: governed-product-outcome`. Usage KNOWN: 3177 prompt +
  258 completion = 3435 tokens; cost KNOWN 0 (operator example price book,
  free upstream at $0). Committed receipt with the complete sealed envelope:
  `docs/evidence/real-model-pilot-tokenbucket-2026-11-16.json`.
  `committed_live_pilot_envelope_verifies` re-derives every binding from
  the committed bytes alone, offline, with no model and no session state.
- **Provenance, bound** - every model-authored repair carries
  `modelPatchProvenance`: sha256 of the raw model response, the canonical
  digest of the parsed proposal (identical to the applied patch digest),
  the 1-based attempt number, the routed model identity and the harness
  revision. A re-sealed envelope conflicting with any trusted pin (validator
  exit code, baseline digest) or with the provenance binding is rejected at
  verification. Model-authored repairs are NOT bound byte-for-byte to the
  task-setter's reference repair (a genuine model may author any
  behavior-correct repair); the byte-exact expected-digest pins still bind
  scripted fixtures, and the validator exit code binds both.
- **Bounded attempts, honest failures** - up to 3 attempts; the failing
  validator output tail is fed back between attempts; malformed proposals,
  out-of-scope files, identical search/replace edits, missing usage
  (usage/cost recorded Unknown, never 0) and interrupted dispatches (typed
  timeout, zero effects, journal preserved) are all typed, accounted outcomes.
- **Negative matrix** - forbidden editable file (`tests/rate_limit.rs`,
  `src/lib.rs`) rejected before any write; nonexistent search text rejected;
  stale revision refused pre-effect; interrupted dispatch keeps failure
  evidence and applies nothing; duplicate application refused (a second
  apply of the same edit does not mutate); false-success reseal rejected on
  every tamper axis; solver-visible corpus leakage guard (no reference-fix
  bytes, no pinned digests); missing usage or missing pricing is recorded
  Unknown, never fabricated.

Verification scope is unchanged: the envelope proves the pinned validator ran
and exited as measured after an applied, permitted repair — nothing more.
Not a benchmark, not model-coding-skill certification, not a Foundry
evaluation, not a public claim. One run on one synthetic task on Windows is
an existence proof, not a rate; Linux/macOS not run; the matched comparison
harness against a baseline coding agent remains Foundry-owned future work.

## What Slice C adds

Slice C adds a second executed task type and a replayable-failure evidence
chain on the Slice B bytes. It does not change Slice A/B behaviour, does not
rebuild the scenario framework, and produces no external evaluation claim.

- **Executed documentation task** - `lite-rust-docs-sum-execute-001`
  (`tests/fixtures/scenarios/rust-docs-sum-execute/descriptor.json`).
  `kind: "documentation"`. The fixture crate `fixture-docs-sum` has a CORRECT
  implementation (`sum(a, b) == a + b`) but its README claims `a - b` and
  `sum(2, 2) == 0`. The task repairs ONLY the README — the sole editable file,
  pinned by the descriptor's `validation` surface. The pinned validator is a
  doc-content check executed as `cargo test --offline --test docs`
  (`tests/docs.rs` reads the committed README): it asserts the README states
  `a + b` and `sum(2, 2) == 4` and no longer states the wrong claims. It
  validates the **documentation content**, and is not code-test evidence of
  any other fact. The repair is a fixture-scripted `search-replace` edit,
  applied with permission under the Slice B preflight order (editable set =
  `README.md`; `src/lib.rs` and `tests/docs.rs` are out of scope and refused
  before any effect). Same pins pattern: content digest, deterministic
  revision, writable `git-worktree` workspace, pinned validator command/argv,
  expected exit code, expected post-repair file digest and expected post-run
  tree digest. Classification and refusal semantics are identical to Slice B:
  `setup-error`/`invalid-task` preflight refusals with zero effects,
  `governed-product-outcome` + `repair-failed` for a validator that ran and
  failed, `infrastructure-error` when the validator cannot complete,
  `governed-product-outcome` + `repaired` only for applied repair + exit 0.
- **Replayable failure evidence** - `lite-rust-bugfix-arithmetic-replay-001`
  (`tests/fixtures/scenarios/rust-bugfix-arithmetic-replay/descriptor.json`)
  is a versioned Lite-internal **failure-replay descriptor**
  (`lite.fixtures.failure-replay`, schema `1.0.0`). It pins: the canonical
  scenario digest of the Slice B execute descriptor, the pinned baseline
  content digest, the known failed validator (`cargo test --offline`,
  observed exit 101), the failed file-after digest, the failed post-run tree
  digest, the replay expectation (`repaired`, classification
  `governed-product-outcome`, exit 0) and the exact authority BOTH runs must
  hold (`repo://fixture` writable scopes, repair origin `scripted-fixture`,
  repair write authority `scripted-fixture-workspace-file-policy`, changed
  files `["src/lib.rs"]`).
- **The failure → replay → verified-repair chain** -
  `LiteFailureReplayEvidence` (`lite.evidence.failure-replay`, schema
  `1.0.0`, module `src/workflow/failure_replay.rs`) embeds BOTH previously
  sealed task-run envelopes — the genuine failure (validator exit 101,
  `taskOutcome: repair-failed`, `classification: governed-product-outcome`)
  and the replay that followed it (validator exit 0, `repaired`) — and
  re-derives every binding at verification time. The distinct failure class
  is `governed-validation-failure`: the governed dispatch completed, the
  fixture-scripted repair was applied with permission, and the pinned
  validator exited non-zero. A zero-exit "failure", a `setup-error` failure
  or an `invalid-task` failure is not a replayable governed validation
  failure and is rejected.
- **The stable failure identity (dedup/idempotency)** - a wall-clock-free
  canonical digest of {scenario digest, validator command/args, observed
  exit code, classification, task outcome, baseline digest, failed post-run
  digest, failed file-after digest}. The same wrong fixture-scripted repair
  against the same pinned fixture reproduces the same `failureIdentity` on
  every run; a different fixture, validator, repair or exit code derives a
  different one. The identity is recorded on the evidence and re-derived at
  verification. The identity is NOT the envelope digest (which embeds
  measured wall-clock); the envelope digest binds one concrete sealed
  instance, the identity binds the failure across instances.
- **A replay may never be an unrelated success and never gains authority** -
  verification requires: both embeds re-verify on their own; the replay runs
  under the SAME embedded scenario descriptor digest (the same pinned task,
  not a substitute); the validator command/argv are identical between failure
  and replay; the baseline, writable scopes (workspace and policy), repair
  origin, repair write authority and changed-file set are identical; the
  replay's exit code is the pinned replay expectation and the failure's exit
  code is the pinned non-zero failure expectation. Any widening of writable
  scopes or changed files, any validator swap, any scenario substitution, any
  changed classification or re-presented failure outcome, any stale envelope
  digest, and any cross-scenario splice fails closed.
- **Negative matrix** - invalid revisions (flip one hex digit; unresolvable
  `000…0`), out-of-scope edits (the correct source `src/lib.rs` in the docs
  task; `Cargo.toml` in the bugfix), path traversal, inapplicable repairs,
  failed dispatch (HTTP 500 → typed failure journal and accounting preserved,
  no repair), post-dispatch refusal (step-capability refusal → zero effects),
  incompatible workspace (read-only manifest against the writable-only git
  worktree), tampered expected digests (seal refuses, infrastructure),
  re-sealed outcomes, drifted baseline/post-run digests, flipped result
  provenance, zero-byte capture without the empty digest, swapped validators,
  widened authority, cross-scenario replay and changed failure
  classification/outcome all fail closed with zero unauthorized
  filesystem effects.
- **Accounting, preserved exactly** - loopback provider usage (7/4/11 tokens)
  is measured and KNOWN under the example price book with measured latency;
  the HTTP 500 failed dispatch produces `Failed { kind: "execution-failed" }`
  accounting with NO attempt digest; unreported measurements remain Unknown
  (never zero); graph aggregates re-derive from the embedded records.

Verification scope, stated on every record: internal consistency of
operator-observed measurements only — NOT cryptographic proof of execution
truth, NOT evidence of model coding ability, NOT a benchmark, NOT a Foundry
evaluation or certification. The failure-replay descriptor is a bridge for
issue #156.

## What Slice B adds

Slice B proves the executed-task path end to end on ONE bounded fixture; it
does not replace or extend Slice A and it does not rebuild the scenario
framework.

- **Execute scenario** — `lite-rust-bugfix-arithmetic-execute-001`
  (`tests/fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json`).
  Same content-pinned Rust bugfix bytes as Slice A (`rust-bugfix-arithmetic`),
  new writable + isolated workspace pins: `git-worktree` adapter (revision
  `lite.workspace.adapter.v1`), `mode: writable`, scope `repo://fixture`, and
  a revision pinned to the deterministic commit of the disposable source
  repository (fixed author identity `lite-fixture` and fixed commit timestamp,
  so the commit — and the pin — reproduce on every platform; the blob bytes
  are the pinned fixture bytes, never modified). Slice A's read-only scenarios
  are untouched.
- **Disposable, deterministic staging** — the runner copies the pinned fixture
  bytes into a fresh temporary git repository, commits with fixed identity and
  timestamps, and checks the produced commit against
  `repository.revision` **before** any workspace exists. The committed source
  fixture in this repository is never mutated in place.
- **Writable isolated worktree** — acquired through the existing #171
  `GitWorktreeWorkspace` adapter under a sealed, writable
  `WorkspaceManifestV1`: the adapter's writable-only gate runs first, the
  acquired revision is re-read and compared to the pin, the workspace is
  validated, and the worktree source tree is digested and compared to the
  fixture pin before any effect. `ensure_writable()` must pass before the
  repair runs.
- **The controlled repair action** — a fixture-scripted `search-replace` edit
  (`a - b` → `a + b`) applied through the filesystem permission boundary
  (`FilePolicy` + `FileSet` + `apply_patch_with_rollback`), with the editable
  set restricted to `src/lib.rs`. The governed model dispatch holds **no**
  write authority; the repair write is gated by the workspace boundary and
  the file permission policy, not by the model. The repair is a **scripted
  fixture** — it demonstrates the executed-task path, never model coding
  ability, and no field in the evidence claims otherwise
  (`repair.origin: scripted-fixture`,
  `repair.writeAuthority: scripted-fixture-workspace-file-policy`; the model
  node's manifest pins `writableScopes: []`).
- **The real validator, pinned** — the descriptor pins a versioned
  `validation` section (`schemaVersion: "1.0.0"`): the exact executable
  (`cargo`), the exact argv (`test --offline`), the expected exit code
  (`0`), the human-readable expected assertion (`sum(2, 2) == 4`), the
  expected post-repair file digest and the expected post-run tree digest.
  The fixture runner executes exactly the pinned argv, captures stdout and
  stderr (bounded at 1 MB, digested into a receipt), records the exit code,
  wall-clock duration and provenance (`observed-subprocess` /
  `not-executed`), and binds all of it into the envelope. A different
  command, argument vector, expected exit code or assertion — or an
  unaltered command with a doctored exit code or claim — cannot seal.
- **Verification limitation, stated** — a re-sealed envelope whose records
  are all mutually recomputed remains undetectable-by-bytes; `verify()`
  proves only internal consistency of operator-observed measurements. The
  envelope pins a `verificationScope` statement to that effect and rejects
  envelopes carrying anything else. It is not cryptographic proof that an
  external process produced the recorded output; the pinned command/argv and
  expected-outcome digests narrow the forgery surface but do not remove it.
- **Task-run evidence** — `LiteTaskRunEvidence` (namespace
  `lite.evidence.task-run`, `schemaVersion: "1.1.0"`) **embeds** the full
  Slice A envelope and re-verifies it, then adds: the `task_run_tree_digest`
  before and after the run (a worktree-pointer-safe digest that excludes the
  top-level `.git` file, unlike the fixture pin; the baseline is bound to
  the embedded descriptor's pinned `repository.contentDigest`), the repair
  record, the validator receipt and a `taskOutcome` re-derived from the
  measurements — `repaired` / `repair-failed` / `repair-refused` — with
  `externalEvaluationFormat` and provenance pinned `unsupported`/`fixture_only`.
- **Preflight order (load-bearing)** — the runner executes: (1) fixture
  content verification, (2) descriptor completeness (a `validation` section
  must be pinned — without it the task is `invalid-task` and is never
  dispatched), (3) deterministic staging with the staged commit checked
  against the pin, (4) isolated writable workspace acquisition and
  verification (writable-only gate, `ensure_writable`, adapter validation,
  revision, baseline tree digest), (5) a repair-target authorization
  preflight (repo-relative paths, editable-set gate, existence check,
  transaction dry-run against the live bytes — zero writes), and only then
  (6) the governed dispatch. A result gate immediately after the dispatch
  refuses to repair after ANY `Err` (typed `Refused` or `ExecutionFailed`):
  no repair, no validator, no success envelope, typed failure/journal/
  accounting evidence preserved exactly as measured, zero filesystem effects —
  a refused or failed dispatch never authorizes the repair. A preflight
  refusal is `setup-error` (or
  `invalid-task`), never runs a model, and carries capability-execution
  counters reading zero, with no accounting record and no sealed evidence.
  On every preflight refusal the loopback server is never spawned and the
  governed boundary is never entered; the positive path dispatches exactly
  once.
- **Classification** — `setup-error` for any pre-effect refusal (fixture
  digest drift, staged/pinned revision mismatch, read-only workspace, no
  worktree, out-of-scope or traversal edits, inapplicable repair);
  `governed-product-outcome` with `taskOutcome: repair-failed` for a repair
  that applies but fails the validator; `infrastructure-error` when the
  validator cannot complete; `governed-product-outcome` + `repaired` only for
  applied repair + validator exit 0. No path produces a false success.

## What Slice A contains

- **Scenario descriptor** — versioned (`schemaVersion: "1.0.0"`, namespace
  `lite.fixtures.scenario`), validated in
  `src/workflow/scenario.rs` with `deny_unknown_fields`, failing closed on
  malformed documents, unsupported versions/languages, path leakage and
  missing or ambiguous pins.
- **Pinned fixtures** (`tests/fixtures/scenarios/<id>/`):
  - `rust-bugfix-arithmetic` — a deliberately broken Rust crate (content-digest
    pinned; `-text` in `.gitattributes` so the digest is byte-stable);
  - `python-bugfix-strings` — a deliberately broken Python module (same
    pinning rules).
  - Each directory pins: inputs (task statement), repository content digest
    (sha256 over the sorted `path hash` listing — not machine paths, not git
    history), expected observable outcome, evidence artifacts, policy profile,
    authority, workspace configuration (`existing-read-only`, adapter revision
    `lite.workspace.adapter.v1`, read-only), harness identity + revision
    (`lite.harness.local_model` / `v1`), route identity (route set, candidate
    pin, provider, model, injected evaluation/dispatch/trust instants), and the
    controlled/variable dimension split (Slice A fixtures are fully
    controlled).
- **Deterministic local runner** — `tests/scenario_fixtures.rs` executes each
  scenario through the real governed boundary (`resolve_route` →
  `dispatch_resolved_route` → `account_routed_outcome`) with a loopback model
  fixture. No network beyond `127.0.0.1`, no paid provider, no fixture
  toolchain execution, no unattended authority.
- **Lite-internal run envelope** — `LiteScenarioRunEvidence` (namespace
  `lite.evidence.scenario-run`, `schemaVersion: "1.0.0"`) **embeds every record
  it references** — the validated scenario descriptor, the sealed route
  evidence, the sealed effective execution snapshot, the node manifest, the
  sealed routed attempt, the execution workspace manifest, the raw sealed
  accounting record and the graph aggregate — and an explicitly typed
  `externalEvaluationFormat: { status: "unsupported" }` stub. `verify()`
  re-derives the envelope's own digest, then re-verifies each embedded record
  against its own sealed digest, then re-checks EVERY cross-object binding
  from the embedded bytes alone: descriptor ↔ route evidence ↔ snapshot ↔
  manifest ↔ attempt ↔ accounting ↔ workspace manifest (identity, digest and
  field level), and the graph aggregate by RE-DERIVING it from the embedded
  accounting records and requiring equality. A forged envelope that splices
  plausible 64-hex digests together, or re-seals a record whose identity
  disagrees with the rest of the chain, fails verification. What the envelope
  does NOT prove is enumerated on the type itself (fixture bytes, measured
  latency provenance, loopback-provider usage truthfulness). Nothing is
  labelled verified that is not re-derived; no cross-object relation is
  checked by hex64 shape alone.

## Failure classification

Setups and tasks are classified, never silently counted as product/model
failure:

| Class | Meaning |
|---|---|
| `setup-error` | Deterministic setup defect: malformed descriptor, unsupported version/language, fixture digest drift, path leakage, missing/ambiguous pins, pre-effect route/workspace/authority refusal. |
| `invalid-task` | The task itself is broken or self-contradictory under the pinned authority/workspace (empty/too-short statement; read-only workspace pinned with writable scopes; route/policy profile disagreement; contradictory authority). |
| `infrastructure-error` | The local harness/runtime pipeline failed after a governed dispatch (e.g., accounting failed to seal). |
| `governed-product-outcome` | The governed boundary ran. A typed, sealed failure inside the effect is **still a product outcome** and is accounted as such; it is not a setup error and says nothing about model capability beyond this scenario. |

## Repeatability

- All evaluation instants are injected; the trusted dispatch clock is fixed
  per fixture (`FixedTrustedTimeSource`).
- The resolution side of the boundary pins every instant, so route evidence,
  snapshot and decision digests reproduce byte-for-byte across reruns.
- The effect side is sealed after the governed effect settles and embeds the
  executing process's wall-clock instants (`startedAt`/`completedAt`,
  measured dispatch latency). Byte equality is therefore asserted only on
  wall-clock-free canonical parts; effect-side records are re-verified on
  every rerun. Any claim of fully byte-identical effect chains would be
  false and is not made.
- Fixture content drift fails closed (`fixture_content_digest`); fixtures are
  `-text` in `.gitattributes` so CRLF conversion cannot change the pinned
  digest.

## Foundry contract status

The Foundry external evaluation format **does not exist yet** (no published
schema, no consumed-by contract). Slice A therefore ships a
`LiteScenarioRunEvidence` envelope that is Lite-internal, carries no Foundry
namespace as a claim of consumption, and types its external-format stub as
`unsupported` — fail-closed at re-verification
(`external_evaluation_format_is_typed_unsupported`).

When Foundry publishes the format, the "Foundry import" work belongs in a
separate, explicitly reviewed change that pins Foundry's schema revision.
Until then:

- Lite does **not** claim Foundry consumes these envelopes;
- Lite does **not** emit benchmark metrics, scores or comparisons;
- Foundry can reproduce latency/token/cost/digest measurements from the raw
  evidence only after Foundry defines its own import contract — that
  definition is out of scope here.
