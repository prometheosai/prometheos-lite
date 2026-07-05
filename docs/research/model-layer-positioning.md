# Model-Layer Positioning

PrometheOS Lite is not a model.

PrometheOS Lite is the local-first workbench around models.

## Decision

PrometheOS Lite should not compete with model labs on raw model capability.

It should compete on:

- local-first workflow
- trust
- transparency
- persistence
- provenance
- verification
- approval gates
- model-agnostic orchestration
- continuity across sessions

## Why this matters

Open coding models are improving quickly.

Models such as Ornith show that agentic coding models may increasingly learn parts of their own scaffold, tool-use patterns, and rollout strategies.

That means PrometheOS Lite should avoid betting its identity on a fixed hand-designed scaffold being permanently superior.

Instead, PrometheOS Lite should become the place where those models are safely used, compared, verified, and remembered.

## The OpenClaw lesson

Do not frame this as "we need to build a better model."

The lesson from OpenClaw-like products is that the interface, persistence layer, distribution surface, and trusted workflow can be more important than owning the underlying model.

PrometheOS Lite should apply that lesson to software work:

- persistent local workbench
- repo-aware context
- model-agnostic provider layer
- human approval
- reviewable artifacts
- safety-first continuation

Do not include unsourced star counts or acquisition/hiring claims unless cited in a separate references section.

## The Ornith lesson

Ornith-like models are not just another provider.

They represent a direction where coding models may internalize more agentic behavior.

PrometheOS Lite should treat Ornith as:

- a model to integrate
- a model to evaluate
- a benchmark pressure test
- not a competitor to imitate at the model level

## Strategic position

```
PrometheOS Lite is the trusted workspace for model-agnostic autonomous software workflows.

Models generate candidate reasoning and code.

PrometheOS Lite frames the work, preserves context, records provenance, gates risk, verifies outputs, and keeps the developer in control.
```

## What PrometheOS Lite should not claim

- PrometheOS Lite makes small models match frontier models.
- PrometheOS Lite beats Ornith.
- PrometheOS Lite has independently validated Ornith benchmarks.
- The harness is superior to learned scaffolds.
- Ornith integration is automatic unless it is actually tested.
- Any benchmark result that has not been reproduced locally.

## Validation plan

Three phases for testing whether PrometheOS Lite adds value on top of Ornith.

### Phase 1: Manual local endpoint validation

Use the existing manual local endpoint path. See the [Ornith manual validation guide](../guides/ornith-manual-validation.md) for step-by-step instructions.

Target examples:

- Ollama `ornith:9b`
- Ollama `ornith:35b`
- vLLM/SGLang OpenAI-compatible endpoint for `deepreinforce-ai/Ornith-1.0-9B`

This phase verifies:

- endpoint compatibility
- request/response parsing
- artifact provenance
- no API key requirement for local endpoint
- deterministic fallback still works

### Phase 2: Harness value comparison

Compare:

1. Ornith used directly through its local endpoint.
2. Ornith used through PrometheOS Lite provider routing + WorkContext + artifacts + provenance.
3. Future: Ornith used through harness validation loops.

Measure:

- task completion
- compile/test pass rate
- number of unsafe/source-modifying attempts
- artifact clarity
- reproducibility
- time/cost if available
- developer review burden

### Phase 3: Small benchmark slice

Use a tiny internal benchmark slice before touching public benchmark claims.

Example tasks:

- risky Rust fixture
- small Python bugfix fixture
- JS/TS lint or type issue fixture
- README/doc patch task
- failing unit test repair fixture

Do not claim public benchmark performance.

The autonomous execution loop should not be evaluated for promotion until this validation plan has produced real evidence. See [Autonomous Loop Graduation Criteria](autonomous-loop-graduation-criteria.md).

## Near-term implementation path

Recommended future PRs:

1. Add an Ornith manual validation guide.
2. Add a repeatable local model comparison harness.
3. Add fixture-based evaluation tasks.
4. Add artifact-level model comparison output.
5. Add docs showing when PrometheOS adds value over direct model use.

## Current recommended product line

```
PrometheOS Lite is not the model. It is the workbench that makes model output inspectable, repeatable, governable, and safe to continue.
```
