# Governed-workflow evaluation fixtures (Lite-owned, experimental)

Issue #136 Slice A delivers the first repeatable, governed-workflow evaluation
fixtures for PrometheOS Lite. Slice B executes one bounded repair task end to
end on those bytes. This page documents what Lite owns, what it
deliberately does not own, and the exact status of the Foundry boundary.

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
  (6) the governed dispatch. A preflight refusal is `setup-error` (or
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
