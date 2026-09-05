# Autonomous Loop: E5 Closeout → E6 Product Surface Queue

## Mode

Epic Completion Mode under Loop Engineering Protocol, delegated to
the implementing agent with independent-reviewer-per-PR gating.

## Operator intent (recorded 2026-09-02)

> Operate autonomously and continuously until the entire project roadmap,
> requirements, TODOs, issues, and in-scope development work is fully
> completed, unless I explicitly instruct you to stop or a genuine
> hard-stop condition requires human authority.
>
> For every implementation or meaningful change, create or delegate to a
> separate independent reviewer agent with a clean context and no
> reliance on the implementing agent's conclusions. …
>
> Development is complete only when all defined in-scope work is
> implemented end-to-end with real production functionality, required
> tests and quality gates pass, documentation and persistent project
> context are current, no known actionable implementation gaps remain,
> and the independent reviewer approves the final project state.

This queue defines the bounded work the agent will execute under that
mandate. It is approved and active; the agent does not require further
operator approval to proceed through the listed items.

## Sources of truth

- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `docs/guides/product-surface-inventory.md`
- `docs/research/model-layer-positioning.md`
- `docs/research/autonomous-loop-graduation-criteria.md`
- Issue #147 — `[CANONICAL ROADMAP] PrometheOS Lite-first product and ecosystem execution plan`
- Issue #106 — `[EPIC E5] Build the specialized governed node library`

## Branch and comparator baseline

- Branch pattern: `autonomous/<scope>` from `origin/main`
- Comparator baseline (must remain ≥ for every PR in this queue):
  - `cargo fmt --check` — clean
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean
  - `cargo test --lib -- --test-threads=4` — **955 passed**, 0 failed, 1 ignored
  - `cargo test --test node_library_conformance` — 13 passed
  - `cargo test --test node_implementation_conformance` — 30 passed
  - `cargo test --test node_conformance_kit` — 2 passed
  - PR is built off a clean main with no unrelated local changes

## In-scope queue (ordered by dependency)

The agent picks the highest-priority unblocked item on each iteration.
Items marked **HARD-STOP** require human authority; the agent will
record the stop in the PR body and queue status, not fabricate
workarounds.

### Phase 1 — Close E5 (governed node library)

1. **#128 — E5/I04: security review, evidence audit, independent
   correctness review nodes** (PR1)
   - Acceptance: review covers paths/commands/deps/secrets/authority;
     evidence audit fails closed on missing/inconsistent artifacts;
     review emits `approve | changes-required | reject` with reasons;
     review cannot authorize apply or merge by itself; nodes pass the
     conformance suite.
   - Comparator: new in-module tests ≥ 5, new conformance tests ≥ 2,
     lib + clippy green, no `Cargo.toml`/`Cargo.lock` changes.
2. **#129 — E5/I05: documentation + release-preparation nodes** (PR2,
   depends on #128)
   - Acceptance: changes stay within approved scope; release output
     links implementation/validation/review/decision evidence; no node
     performs merge/publish/deploy without explicit authority; nodes
     pass the conformance suite.
3. Update handoff and `specs/loop-engineering/changes/` once both
   items merge; close epic #106.
4. **R3 — E5 closeout bookkeeping (PR3)** — the E5 epic closeout
   PR: a one-commit PR that records the E5 closeout comment on
   #106, updates the queue doc, and closes any loose bookkeeping.
   It exists so the E5→E6 transition is a clean handoff, not a
   silent cut. (Already completed in this session: comment on
   #106 + queue-doc update.)

### Phase 2 — E6 (developer-facing product, experimental surfaces)

The E6 issues are each large enough to span multiple PRs. The
queue below documents the **first bounded slice** of each issue
that fits the 5-file/200-LOC default PR budget and ships real
value. Additional slices are added to the queue after the first
slice of each issue merges, in dependency order.

5. **R4 — #130 (E6/I01) Slice B: config version + invalid-config
   diagnostics** (PR4)
   - Add a `configVersion: String` field to `AppConfig`; the
     loader rejects unknown versions with a message naming the
     `configVersion` field and the supported range.
   - Add an integration test that loads a fixture config and
     asserts the error message names the field and the
     supported versions.
   - Comparator: ≥ 2 new integration tests; `cargo test --test
     config_loader_tests` (or equivalent) green; no
     `Cargo.toml`/`Cargo.lock` changes.
6. **R5 — #130 Slice C: workflow templates for bug-fix / feature
   / refactor / test / docs / review** (PR5)
   - Add 6 new flow YAMLs under `flows/`, registered in
     `TemplateLoader`; each is parseable and a unit test asserts
     the loader enumerates it.
7. **R6 — #130 Slice A: CLI contract integration tests** (PR6)
   - Add CLI-parse-only tests (no runtime needed) for each
     top-level `Commands` variant, asserting the clap parser
     accepts the documented invocation and rejects malformed
     ones with an actionable error.
8. **R7 — #131 (E6/I02) Run inspector, evidence viewer, human-decision
   interface** (PR7+)
   - First slice: a read-only `prometheos work inspect <work_id>`
     subcommand that prints graph state, node attempts, evidence
     chain, and approval history to stdout. The interface cannot
     mutate state. Companion conformance tests drive it through
     the CLI parser + a synthetic work fixture.
9. **R8 — #132 (E6/I03) Local API + durable execution event stream**
   (PR8+)
   - The existing `src/api/` surface already implements the
     headless runtime control boundary. The first slice is a
     regression that locks the API's read-model rebuild property
     (cursors, idempotent mutations) with a dedicated test
     suite, not new endpoints.
10. **R9 — #133 (E6/I04) Provider routing, policy profiles, cost
    accounting** (PR9+)
    - First slice: an explicit `CompatibilityDecision` record
    wired into the existing `node_runner` path; the
    `prometheosai/soma#83` semantics are documented in code
    (an enum + the `WorkRequirements ∩ RuntimeCapabilitySet
    ∩ effective ExecutionProfile = CompatibilityDecision`
    formula) but a full SOMA-published contract consumer is out
    of scope here.
11. **R10 — #134 (E6/I05) Repository onboarding + actionable
    diagnostics** (PR10+)
    - First slice: a deterministic, rule-based detector for
    the most common languages + package systems + test
    commands (Cargo, npm, Go, Python, Make). Confidence +
    evidence are emitted in the output. The detector NEVER
    modifies project files; the operator must approve.
12. #135 — E6/I06 (local web dashboard) is **experimental**; the
    canonical roadmap and `product-surface-inventory.md` list it as
    such. Out of scope for the alpha promise; deferred to a later
    issue once #130-#134 ship.

### Phase 3 — P1 follow-ups from the canonical roadmap

10. #136 — Foundry benchmark fixtures (in-scope only if a local
    benchmark is reproducible without external model access; otherwise
    hard-stop).
11. #137 — interruption/concurrency/resource-pressure campaigns
    (**HARD-STOP**: requires orchestrated multi-hour runs on real
    repositories; the agent may author the campaign script but cannot
    execute it as autonomous CI).
12. #138 — independent security/governance assessment (**HARD-STOP**:
    human authority; agent records the request, does not self-assign).
13. #139 — human-review usability study (**HARD-STOP**: requires real
    human subjects).
14. #140 — paid design-partner pilot (**HARD-STOP**: external commercial
    relationship).

### Status checkpoint (2026-09-05)

The autonomous loop has driven the following work in this session,
all under the operator-mandated independent-reviewer protocol with a
comparative control gate per PR:

- Phase 1 (E5 closeout) complete. PRs #207 (E5/I04 review nodes) and
  #208 (E5/I05 doc/release nodes) merged. Issues #128, #129 closed.
- Phase 2 opened. E6/I01 (#130) closed via three PRs (#209 Slice B
  config version + diagnostics; #210 Slice C workflow templates;
  #211 Slice A CLI contract tests).
- E6/I02 (#131) Slice A complete. PR #212 (read-only run inspector)
  merged. Issue #131 still open; the inspector slice delivered all
  five acceptance bullets within the repo-workbench scope. The
  remaining work (pre-apply hook, graph-state inspector, content-
  hash stale-approval check) is documented in the R7 change
  record.

The next E6 tasks (R8: #132 local API + event stream, R9: #133
provider routing, R10: #134 repo onboarding) are downstream of
this checkpoint. A session-level status report is recorded in
`handoff.md` so the operator can see exactly which PRs / commits /
issues the autonomous loop has driven, which safety-gate hard-
blockers were honoured, and which deferred work remains.

### Hard-stops the agent must NOT work around

- Promoting the frontend or API server to stable alpha.
- Claiming autonomous execution, Mnemosyne, Brain, cloud/team,
  marketplace, or benchmark results that are not actually demonstrated.
- Adding new dependencies without explicit operator approval.
- CI weakening, test removal/skip, or narrowing acceptance criteria to
  make a build pass.
- Hidden failures, mocks/stubs/placeholders in place of production
  code, or treating the reviewer as a rubber stamp.
- Re-architecting the runtime/control boundary in ways not already
  approved in the canonical roadmap.
- Cross-repo work in `prometheosai/soma`, `prometheosai/foundry`,
  `prometheosai/mnemosyne`, or `prometheosai/prometheos` — those
  repos are out of scope; the agent only produces handoff notes for
  them when relevant.

## Independent-reviewer protocol

Per operator instruction, every PR in this queue is reviewed by a
**fresh-context** reviewer agent. The reviewer is given:

- the issue body and acceptance criteria;
- the exact diff range (`git diff origin/main..HEAD`) and commit SHAs;
- the comparator baseline (numbers above);
- the safety gates and product boundary docs;
- the explicit comparative control gate: "expected outcome vs.
  observed outcome" per acceptance bullet.

The reviewer returns one of:

- `APPROVE` — implementation matches acceptance; merge.
- `REPAIR` — precise, actionable fix list; the implementing agent
  re-runs the loop with a new exact revision and submits a fresh
  review cycle.
- `HARD_STOP` — used only for the conditions listed in the operator
  instruction (missing credentials, irreversible destructive
  decisions, contradictory requirements, explicit policy/security
  gates). A failed test, lint error, CI flake, or normal engineering
  doubt is NOT a HARD_STOP — it's a REPAIR.

The reviewer MUST run the actual build commands. The implementing
agent MUST NOT self-approve. If the subagent reviewer is unavailable
(rate-limit, payment, harness fault), the agent falls back to a
documented inline review with the same comparative control gate
recorded in the PR body.

## Loop invariants

- One concern per PR; no unrelated refactors.
- 5 files / 200 LOC budget per PR is a default; the agent escalates
  before exceeding it.
- Each PR merges only after reviewer `APPROVE` AND CI green.
- The agent updates this queue doc and the `handoff.md` after every
  merged PR; the `specs/loop-engineering/changes/` record follows the
  Loop Engineering Protocol template.
- The agent does not begin a new task until the prior task is
  merged, REPAIR'd and re-merged, or recorded as HARD-STOP.

## Status

- 2026-09-02: queue opened; baseline captured (Phase 0).
- 2026-09-02: ready to begin Phase 1.
- 2026-09-03: **Phase 1 (E5 closeout) complete.** PR #207 (E5/I04
  review-nodes, merge 6e11a72) and PR #208 (E5/I05 doc/release-nodes,
  merge fa305e0) merged. Issues #128 and #129 closed. E5 epic #106
  delivery checklist 5/5; closeout comment posted. 998 lib + 21
  lib-conformance + 30 impl-conformance + 2 kit tests passing.
- 2026-09-03: **Phase 2 opened.** E6 work is now picked up per
  the per-slice plan in this doc. First task: R4 (#130 Slice B —
  config version + diagnostics).
