# Milestone: Governed Patch Pilot

**Status:** Partially executed. Attempt 1 was infrastructure-blocked; the
governed path was verified end-to-end against a deterministic stub; Attempt 2
ran a real local model (Ollama + `qwen2.5-coder:7b`) exactly once and produced
an honest model-output compatibility failure (no tuning performed).

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

## Recorded pilot runs (Task 1)

Artifacts live under `C:\Users\Diego\AppData\Local\Temp\opencode\pilot\`.
Attempt 1 is preserved, Attempt 2 is recorded **exactly once** and must not be
re-run or tuned around.

### Attempt 1 — infrastructure-blocked (not pilot-qualified)

- Provider: `openrouter`, model: `anthropic/claude-sonnet-4`, authority: `assist`.
- Outcome: `BLOCKED / FAILED FIRST ATTEMPT`.
- Two independent blockers:
  1. `OPENROUTER_API_KEY` absent from the environment (paid credential not
     obtainable in session).
  2. `LlmClient` appends `/v1/chat/completions` to `base_url`, so a base_url of
     `https://openrouter.ai/api/v1` produced a malformed double-`/v1` URL →
     HTTP 404. (Fixed by PR #82, which normalizes OpenAI-compatible endpoints so
     a conventional base URL such as `https://openrouter.ai/api/v1` does not
     produce a duplicate `/v1`.)
- No patch generated; the governed workflow was not exercised on the repo.
- `proposal_generated: false`, `provider_cost_usd: 0.0`.
- Evidence: `task1-result.json`.

### Stub verification — governance plumbing (not pilot-qualified)

- Deterministic stub OpenAI-compatible server returning valid JSON-schema edits
  (`create_file` of a regression test).
- Repository: `BurntSushi/memchr` (the "repository you barely know" target).
- `proposal_generated: true`, `dry_run_passed: true`, `approved: true`,
  `applied: true`, `validation_passed: true`, `files_changed: 1`, `lines: 6`.
- `pilot_qualified: false` — the stub stands in for a live model, so model
  reasoning (repo understanding, coverage detection, no-op willingness) is NOT
  exercised. This is **not** Pilot Task 1.
- Also confirms PR #82 URL normalization end-to-end (`/api/v1/chat/completions`,
  not `/api/v1/v1/...`).
- Evidence: `governance-integration-verification.json`.

### Attempt 2 — pilot-qualified Ollama local-model baseline (executed once)

- Provider: `ollama`, model: `qwen2.5-coder:7b`, `base_url: http://localhost:11434/v1`.
- Config: `pilot/run/prometheos.config.json`
  (`{ "provider": "openai", "model": "qwen2.5-coder:7b", "base_url": "http://localhost:11434/v1" }`);
  `api_key` left unset (correct for Ollama). Model pulled and loaded on a local
  GTX 1070 (4.36 GiB, Q4_K).
- Authority: `assist`; allowed `src/**`, `tests/**`; forbidden `.github/**`,
  `Cargo.toml`, `Cargo.lock`; `--max-files 2`, `--max-lines 80`,
  `--validate "cargo test"`.
- Goal: add the smallest nonredundant regression test for a match at the final
  valid haystack position in `memmem`, without modifying implementation unless
  a defect is exposed.
- Outcome: `provider_produced_no_usable_candidate`.
  - The provider **responded**, but its output did not match the supported edit
    schema: a fenced ```` ```json ```` block whose edits used an unsupported
    type `"file"` (supported: `search_replace` / `whole_file` / `create_file` /
    `delete_file`) and targeted a C file rather than Rust.
  - With the markdown/edit-block fallback disabled in production config,
    `LlmPatchProvider`'s strict schema parsing recovered zero usable edits.
- `proposal_generated: false`; dry-run, approval, and apply **not reached**;
  `provider_cost_usd: 0.0`; `pilot_qualified: true`.
- **No parser, prompt, model, or configuration tuning was performed before
  recording the result.** This is a legitimate pilot data point: a 7B local
  model rarely emits the exact JSON-schema edits the governed path requires.
- Classification recorded: `task: 1, attempt: 2, pilot_qualified: true,
  provider: ollama, model: qwen2.5-coder:7b, provider_cost_usd: 0.0,
  comparison_note: "Local-model baseline; not directly equivalent in capability
  to Claude Sonnet"`.
- Evidence: `task1-attempt2-report.json`.
- **Disposition:** Close Task 1 Attempt 2 as an honest model-output
  compatibility failure. Do **not** count it as a successful task, but keep it
  in the pilot dataset. The memchr tree was restored to its pilot base
  (`bce7df7`) and the stub test file reverted; no source changes remain.
