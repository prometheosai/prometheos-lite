# Milestone: Governed Patch Pilot

**Status:** planned — blocked on a live configured provider in this environment.

**Purpose:** Stop building platform foundations and prove PrometheOS Lite on
real repositories. The product already has the full governed path:

```
goal
  -> provider generates edits
  -> governed proposal (hostile-input + scope checks)
  -> isolated dry-run
  -> human approval (bound to exact patch hash)
  -> checkpoint
  -> application
  -> validation + rollback
  -> evidence report
```

The claim to validate is **not** "PrometheOS writes better code than every
model." It is: *PrometheOS makes model-generated software changes safer,
reviewable, reproducible, and recoverable.*

## Tasks

Run **10 real tasks across at least 5 repositories**, in three categories:

1. A repository you know well.
2. A repository you barely know.
3. A public repository maintained by someone else.

Tasks are small but real (no synthetic "create generated_patch.rs" demos):

- fix a failing unit test
- add input validation
- repair an error message
- update a deprecated API
- add one missing test
- fix a boundary condition
- correct a configuration bug
- remove a small duplication
- add a narrowly scoped CLI option
- repair a documentation/code mismatch

## Metrics (one structured result file per run)

```json
{
  "repository": "owner/repo",
  "task": "Fix boundary condition in parser",
  "provider": "openrouter",
  "model": "model-name",
  "authority": "assist",
  "proposal_generated": true,
  "dry_run_passed": true,
  "approved": true,
  "applied": true,
  "validation_passed": true,
  "rollback_used": false,
  "files_changed": 2,
  "lines_changed": 31,
  "scope_violations": 0,
  "human_edits_required": 1,
  "time_to_first_proposal_seconds": 48,
  "total_time_seconds": 310,
  "provider_cost_usd": 0.07,
  "useful": true,
  "notes": "Changed one extra test fixture before scope was tightened"
}
```

Key measurements: task correctness, scope adherence, human correction
required, regression rate, approval rejection rate, dry-run failure rate, time
saved, provider cost, rollback success, and whether the evidence report
changed the human decision.

## Baselines

For each task, compare three configurations:

- **Baseline A** — model / coding agent modifies the repository directly.
- **Baseline B** — model produces a patch, human reviews manually.
- **PrometheOS** — provider produces a governed proposal through the full path.

Measure: correct result, unrelated files changed, regressions introduced, time
to review, time to repair, number of attempts, cost, evidence completeness,
recovery quality.

## Success gate (before any new large feature)

- 10 real tasks attempted
- at least 7 completed correctly
- zero silent scope violations
- rollback proven in at least two controlled failures
- at least three external users
- at least one user returning for a second repository
- evidence that review time is lower than the direct-agent baseline

## Packaging note

Internally the command remains `workflow` (generate / dry-run / approve /
apply / report). The user-facing surface should be renamed around outcomes:

```
prometheos plan "Fix the parser boundary bug"
prometheos propose
prometheos verify
prometheos approve
prometheos apply
prometheos report
```

## Next: GitHub-native workflow

Once 5–10 real tasks work, build the distribution surface:

```
GitHub issue
  -> /prometheos plan
  -> proposal attached to issue or PR
  -> dry-run and validation report
  -> human approval
  -> branch and draft PR
```

First GitHub commands: `/prometheos review`, `/prometheos plan`,
`/prometheos propose`. The app reads an issue, analyzes the repo, creates a
governed proposal, posts scope / patch summary / risk / validation /
provenance, and creates a draft PR only after explicit approval. It never
merges and never runs arbitrary shell from comments.

## Blocker (this environment)

Executing the pilot requires a **reachable model provider** (e.g. OpenRouter,
LM Studio, Ollama). In the verification environment used to merge #79 there was
no live endpoint: LM Studio (127.0.0.1:1234) was down and no
`PROMETHEOS_*`/API key was configured. The real `LlmPatchProvider` path was
exercised end-to-end against a stub OpenAI-compatible server returning
JSON-schema edits, confirming the governed workflow routes real provider
output correctly. To run the 10 real tasks, configure a provider (or point
`PROMETHEOS_BASE_URL` at a local model server) and supply the first
"repository you barely know."
