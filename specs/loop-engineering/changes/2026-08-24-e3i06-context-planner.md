# Change Spec: E3/I06 - Model-aware ContextPlanner + final ContextBundle assembly (#153)

- **Issue:** #153 (parent #104; deps #116 #118 #152 #167 - ALL MERGED)
- **Branch:** feat/context-planner
- **Status:** single bounded PR

## Objective

The canonical execution-time ContextPlanner/Assembler in Lite: consume
declared inputs (task, work state, repo index port, decisions/failures/
validations evidence, capability profile, budgets), apply deterministic
ranking + freshness + dedupe + progressive disclosure, and produce the final
ordered provenance-rich `ContextBundle` (lite.memory.v1) under enforced
budgets. Lite owns final assembly; backends only supply candidates.

## Deliverables

`src/workflow/context_planner.rs`:

- `PlannerInputsV1`: schema_version, planner_id, task text, work_state_ref,
  revision (current), candidate sources via `MemoryRetrievalPort` refs,
  model profile (id, context_window_tokens, reserved_output_tokens),
  token_budget (effective, from lite.policy snapshot or explicit),
  include_decisions/include_failures/include_validations flags,
  privacy: denied_providers (informational for policy binding).
- `plan(inputs, ports: &[&dyn MemoryRetrievalPort]) -> Result<PlanOutcome>`
  where PlanOutcome { bundle: ContextBundle, audit: PlanningAudit }:
  1. retrieve from every port (typed unavailable => omitted entry w/ reason,
     never aborts the whole plan when >=1 port succeeds);
  2. merge RawCandidates through assemble_retrieval (dedupe/conflict/stale);
  3. progressive disclosure: rank blocks by relevance desc then memory_id;
     cap at budget = min(model window - reserved output, explicit budget);
  4. deterministic ContextBundle via assemble_context_bundle.
- `PlanningAuditV1`: per-port status (ok/unavailable/stale), counts
  retrieved/selected/omitted by reason class, digest echo. Fail-closed parse.
- Determinism: same inputs+port results+policy => same ordered bundle +
  same digest (tested).
- Model-profile awareness: two profiles over identical candidates yield
  different projections (different budgets) — tested.

## Acceptance mapping (#153)

- local-only end-to-end ✓ (local RepoEvidencePort from #178)
- local + mnemosyne-shaped stub pass same fixtures ✓ (pattern from #178)
- determinism test ✓
- provenance/revision/selection-reason on every included item ✓ (blocks carry memory_id; omitted carry reasons)
- missing symbols typed-absent ✓ (#167 NotFound + empty evidence)
- stale rejected/classified ✓ (assemble_retrieval omission)
- budgets enforced before provider invocation ✓ (assembly-time capping)
- different model profiles => different projections ✓ (test)
- private/unauthorized excluded with reasons ✓ (scope enforcement upstream + omitted reasons)
- prior failures/decisions without chat dupes: decision/failure evidence enter as candidates with kinds Decision/Fact and are ranked, not concatenated ✓
- Foundry fixtures: out of scope here (#136 owns them); noted non-goal

## Non-goals

Mnemosyne real adapter (#142/#82); SOMA compact projection research;
provider invocation itself.

## Verification plan

fmt/clippy/-D warnings; cargo test --lib workflow::context_planner;
tests/context_planner_scenarios.rs (determinism, two-profile projection,
multi-port partial failure, budget capping).
