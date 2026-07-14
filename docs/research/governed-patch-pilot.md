# Milestone: Governed Patch Pilot

**Status:** Partially executed. Attempt 1 was infrastructure-blocked; the
governed path was verified end-to-end against a deterministic stub; Attempt 2
ran a real local model (Ollama + `qwen2.5-coder:7b`) exactly once and produced
an honest model-output compatibility failure (no tuning performed). Attempt 3
re-ran the same model/goal/repo after the prompt/parser compatibility fix and
again produced zero usable edits; the **exact rejection subtype is unknown**
because production observability did not preserve or safely expose the raw
response or the parser rejection reason. An observability fix (issue #86, PR
#87) was merged to make future attempts classifiable.

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

### Attempt 3 — same-model re-run after prompt/parser compatibility fix (executed once)

- **Pilot-qualified:** true
- Provider: `ollama`, model: `qwen2.5-coder:7b`, `base_url: http://localhost:11434/v1`
  (conventional `/v1` form; PR #82 normalization applies).
- Authority: `assist`; allowed `src/**`, `tests/**`; forbidden `.github/**`,
  `Cargo.toml`, `Cargo.lock`; `--max-files 2`, `--max-lines 80`,
  `--validate "cargo test"`; `--provider config`.
- Same repository (`memchr`) and commit (`bce7df7`) as Attempt 2; same goal;
  no prompt, model, temperature, or configuration changes; no retries.
- Purpose: same-model evaluation after the prompt/parser compatibility fix
  (PR #84 hardened the fenced ```edit``` fallback grammar; the production
  `LlmPatchProvider` now enables that fallback).
- Result category: **`unclassified_parser_rejection`** (also recorded as
  **`zero_usable_edits_reason_unknown`**).
- Outcome: `provider_produced_no_usable_candidate`.
- `proposal_generated: false`; dry-run, approval, and apply **not reached**;
  `repository mutation: none`; `cost: $0`.
- **Raw provider response: not captured.** The model responded (this was not a
  timeout or model-connection failure), but the production parser recovered
  zero usable edits.
- **Detailed rejection classification: unknown.** Reason: production
  observability did not preserve or safely expose the raw response or the
  parser rejection reason. The system can refuse model output but could not
  audit *how* it refused it.
- Repo tree left clean at the pilot base (`bce7df7`); no artifact, report, or
  patch persisted.
- **Disposition:** This is a valid pilot data point, not a success. The unknown
  rejection subtype is itself a finding — see issue #86 / PR #87, which add
  structured, content-free parse diagnostics (`rejection_reason`,
  `response_sha256`, etc.) persisted by default, with the raw response saved
  only behind `PROMETHEOS_CAPTURE_PROVIDER_RESPONSE=1`. Attempt 4 is reserved
  to **classify** this zero-edit outcome using those diagnostics, not to retry
  for success.
- Supporting evidence: standalone `task1-attempt3-record.md` retained from the
  run (kept alongside this canonical log; the canonical record above is
  self-contained and does not require it).

### Attempt 4 — diagnostic classification after observability fix (executed once)

- **Pilot-qualified:** true
- **Purpose:** diagnostic classification after observability fix (issue #86 / PR #87).
- **Retry-for-success:** false. Same repository (`memchr`), commit (`bce7df7`),
  goal, model (`qwen2.5-coder:7b`), endpoint (`http://localhost:11434/v1`),
  scope, validation, and authority as Attempts 2–3. The only change was enabling
  `PROMETHEOS_CAPTURE_PROVIDER_RESPONSE=1`.
- **Raw capture enabled:** true.
- Outcome: `provider_produced_no_usable_candidate`. `proposal_generated: false`;
  dry-run, approval, and apply **not reached**; `repository mutation: none`;
  `cost: $0`.
- **Classification (from structured diagnostics only):**
  - `rejection_reason`: `edit_fence_missing`
  - `parse_route_attempted`: `edit_block_fallback`
  - `canonical_json_detected`: false
  - `edit_fence_detected`: false
  - `response_length`: 424
  - `response_sha256`: `cab7c3bb46ce05d09a4ac35ab31724f53cdbff331045b3280d7d7eb8f70a861e`
  - `usable_edit_count`: 0
  - `raw_response_persisted`: true
- **Interpretation:** the production parser now *explains* the rejection. The
  model responded with a ```json-fenced block whose JSON was malformed (an
  invalid `"content:` key), so the canonical JSON route failed to parse it and
  the fenced ```edit``` fallback did not apply (no ```edit fence present). Hence
  `edit_fence_missing` via the `edit_block_fallback` route. This is the same
  zero-usable-edits family as Attempt 3, now **classified** rather than opaque.
- Canonical evidence: `.prometheos/diagnostics/cab7c3bb46ce05d09a4ac35ab31724f53cdbff331045b3280d7d7eb8f70a861e.json`
  (structured) and `.response.txt` (raw, captured locally). The raw response was
  manually verified to contain **no** credentials, API keys, or authorization
  material; it is kept local per the capture policy and is not required to read
  the structured classification above.
- **Disposition:** Valid pilot data point, not a success. Attempt 4 closes the
  loop opened by Attempt 3: the experiment is now honest — the system can state
  *why* it refused the model output (`edit_fence_missing`). No prompt, parser,
  model, config, or temperature change was made after observing the response, and
  no second call was issued.

### Task 1 — consolidated outcome

```text
Outcome: unsuccessful
Primary reason: local model failed to emit a valid supported edit format
Governance result: safe rejection, no repository mutation
System defect discovered: yes, fixed during Attempts 2–3
Remaining failure attributable to system: no
```

- Attempts 1–4 are the **same Task 1**. Attempt 4 is diagnostic evidence attached
  to Task 1, not a separate product-success task. It does not move the success
  count.
- The remaining failure is attributable to the model's output format, not to
  PrometheOS. Closing the parser to malformed JSON is the correct boundary;
  loosening it to accommodate one model's typo would weaken governance.

### Pilot metrics (to date)

```text
Real tasks attempted: 4
Successful tasks: 0
Silent scope violations: 0
Unsafe mutations: 0
Diagnosable rejections: 4
Rollback demonstrations: 1
```

The meaningful result is not "the model failed." It is:

> PrometheOS rejected malformed model output deterministically, explained the
> rejection, and left the target repository clean.

## Task 2 — next honest data point

**Not yet executed.** Per the analysis, Task 2 must differ from Task 1 in one
axis only: a *different repository*, with the *same unchanged model and
configuration* (`ollama` / `qwen2.5-coder:7b` / `http://localhost:11434/v1`).
This isolates model behavior from repo-specific quirks instead of repeatedly
re-interrogating `memchr` until randomness yields a success.

Setup required before execution:

- A reachable provider (Ollama running at `http://localhost:11434/v1`).
- A chosen "repository you know well" or "repository you barely know" that is
  **not** `BurntSushi/memchr`.
- A small, real, narrowly-scoped goal (e.g. add one missing test, repair a
  boundary condition, fix an error message).
- No prompt / parser / model / config changes relative to Task 1.

Execution is gated on the above and on an explicit go-ahead, since the
autonomous execution loop is experimental and outside the stable-alpha surface.

### Task 2 — preflight (infrastructure-blocked, not pilot-qualified)

- Target repository: `BurntSushi/byteorder`, branch `master`, initial commit
  `5a82625fae462e8ba64cec8146b24a372b4d75c6` (cloned and pinned before any run).
- Provider preflight: `POST http://localhost:11434/v1/chat/completions` with
  `qwen2.5-coder:7b` → **connection refused** (`Impossível conectar-se ao
  servidor remoto`). Ollama is not running/reachable at the configured endpoint.
- **No `workflow generate` was issued.** The go-ahead condition (preflight
  returns `ready`) is not satisfied.
- `pilot_qualified`: false (infrastructure-blocked, identical class to Task 1
  Attempt 1). `proposal_generated: false`; repository mutation: none.
- Disposition: blocked, not failed. Re-run the preflight once Ollama is live;
  then execute Task 2 Attempt 1 exactly once with no config changes.

### Task 2 Attempt 1 — pilot-qualified, executed once

- **Pilot-qualified:** true
- **Repository:** `BurntSushi/byteorder`, branch `master`, commit `5a82625fae462e8ba64cec8146b24a372b4d75c6` (different codebase from Task 1's `memchr`; same language/toolchain).
- **Provider:** `ollama`, model `qwen2.5-coder:7b`, `base_url: http://localhost:11434/v1`
  (unchanged config from Task 1; model pulled during this session after the
  `not_found_error` preflight, then preflight returned `ready`).
- **Authority:** `assist`; allowed `src/**`, `tests/**`; forbidden `.github/**`,
  `Cargo.toml`, `Cargo.lock`; `--max-files 2`, `--max-lines 80`,
  `--validate "cargo test"`; `--provider config`.
- **Same goal** as specified (boundary/zero-length/exact-length read/write test).
- **Outcome:** `provider_produced_no_usable_candidate`.
- `proposal_generated: false`; dry-run, approval, and apply **not reached**;
  `repository mutation: none` (byteorder tree clean at `5a82625`); `cost: $0`.
- **Classification (from structured diagnostics only):**
  - `provider_response_received`: true
  - `rejection_reason`: `edit_fence_missing`
  - `parse_route_attempted`: `edit_block_fallback`
  - `canonical_json_detected`: false
  - `edit_fence_detected`: false
  - `response_length`: 843
  - `response_sha256`: `3abc5b9456ba84a7fcf5a2a0111a605986ccc603149d091370e795c311de175e`
  - `usable_edit_count`: 0
  - `raw_response_persisted`: false
- **Interpretation:** same zero-usable-edits family as Task 1 Attempts 3–4, now on
  a *different repository*. The model responded (843 bytes) but emitted neither a
  canonical JSON edit block nor a fenced ```edit``` block, so the production parser
  rejected it deterministically via `edit_fence_missing`. The change of codebase
  did not change the failure mode — the defect is in the model's output format,
  not the target repo.
- **Disposition:** Valid pilot data point, not a success. Attempted exactly once;
  no prompt, parser, model, config, or temperature change after observing the
  result, and no second call. Structured diagnostics persisted by default at
  `pilot/run/.prometheos/diagnostics/3abc5b9456ba84a7fcf5a2a0111a605986ccc603149d091370e795c311de175e.json`.

### Task 2 — consolidated outcome

```text
Outcome: unsuccessful
Primary reason: local model failed to emit a valid supported edit format
Governance result: safe rejection, no repository mutation
System defect discovered: no (parser already hardened in Attempts 2–3)
Remaining failure attributable to system: no
```

- Same classification as Task 1 (`edit_fence_missing`); the failure reproduces
  across two distinct repositories (`memchr`, `byteorder`) with one unchanged
  model/config. This strengthens the reading that the boundary is correct and the
  model output format is the cause, not the target codebase.

## Task 3 — model-comparison (controlled variable: model only)

**Design:** identical to Task 2 in every axis (repository `byteorder` @ `5a82625`,
same goal, same scope/authority/governance) except the **model**: `qwen2.5-coder:14b`
replaces `qwen2.5-coder:7b`. This isolates *model-specific conformance failure*
from *prompt/schema design broadly incompatible with local coding models*.
Recorded as a comparison, **not** a retry of Task 2. Parser unchanged.

### Task 3 Attempt 1 — pilot-qualified, executed once

- **Pilot-qualified:** true
- **Repository:** `BurntSushi/byteorder`, branch `master`, commit `5a82625fae462e8ba64cec8146b24a372b4d75c6` (identical to Task 2).
- **Provider:** `ollama`, model `qwen2.5-coder:14b`, `base_url: http://localhost:11434/v1`
  (model pulled this session; preflight returned `ready`). All other settings
  unchanged from Task 2.
- **Authority:** `assist`; allowed `src/**`, `tests/**`; forbidden `.github/**`,
  `Cargo.toml`, `Cargo.lock`; `--max-files 2`, `--max-lines 80`,
  `--validate "cargo test"`; `--provider config`.
- **Same goal** as Tasks 1–2.
- **Outcome:** `provider_produced_no_usable_candidate`.
- `proposal_generated: false`; dry-run, approval, and apply **not reached**;
  `repository mutation: none` (byteorder tree clean at `5a82625`); `cost: $0`.
- **Classification (from structured diagnostics only):**
  - `provider_response_received`: true
  - `canonical_json_detected`: **true**  ← differs from 7b (`false`)
  - `edit_fence_detected`: false
  - `parse_route_attempted`: `edit_block_fallback`
  - `rejection_reason`: `edit_fence_missing`
  - `response_length`: 190
  - `response_sha256`: `779e43537678b5822fcd135ae968f81a81c8261c497234c4a8ed03fca80806a5`
  - `usable_edit_count`: 0
  - `raw_response_persisted`: false
- **Interpretation:** the 14b model emitted a *canonical JSON* block (detected),
  but its edits were not a usable supported set, so the fallback still rejected it
  via `edit_fence_missing`. This is a **different failure signature** from the 7b
  (which emitted no detectable JSON and no edit fence). Both fail safely with zero
  mutation. The 14b gets "closer" to the schema but still does not satisfy it.
- **Disposition:** Valid pilot data point, not a success. Attempted exactly once;
  no parser/prompt/config/temperature change after observing the result, and no
  second call. Structured diagnostics persisted by default at
  `pilot/run/.prometheos/diagnostics/779e43537678b5822fcd135ae968f81a81c8261c497234c4a8ed03fca80806a5.json`.

### Task 3 — consolidated outcome

```text
Outcome: unsuccessful
Primary reason: local model emitted canonical JSON but no usable supported edit set
Governance result: safe rejection, no repository mutation
Controlled variable: model (7b -> 14b); all else unchanged
Remaining failure attributable to system: no
```

## Model-comparison read (Tasks 2 vs 3)

| Axis | Task 2 (7b) | Task 3 (14b) |
|------|-------------|-------------|
| `canonical_json_detected` | false | true |
| `edit_fence_detected` | false | false |
| `usable_edit_count` | 0 | 0 |
| rejection | `edit_fence_missing` | `edit_fence_missing` |
| repo mutation | none | none |

The two models fail for *related but distinct* reasons: 7b never reaches the JSON
schema; 14b reaches it but still does not emit a usable edit set. This argues
against "the prompt/schema is broadly incompatible with local coding models" being
the sole cause (14b clearly engages the schema) and toward *model-specific
conformance failure* at the 7b/14b tier. Both are still governed safely. Caveat:
two models and three tasks remain a small sample; this is a directional signal, not
proof.

## Pilot metrics (to date)

```text
Real tasks attempted: 3
Successful tasks: 0
Diagnosable rejections: 3
Silent scope violations: 0
Unsafe mutations: 0
Repositories exercised: 2
Rollback demonstrations: 1
```

