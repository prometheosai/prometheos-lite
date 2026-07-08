# Safety Gates

Hard blockers and soft warnings for the PrometheOS Lite Loop Engineering Protocol.

## Hard blockers

Stop immediately and hand off if any of these appear:

- **CI weakened** — CI is weakened, tests are removed, skipped, or narrowed only to pass.
- **Stable alpha scope change** — Stable alpha scope changes without explicit approval.
- **`prometheos work` behavior change** — `prometheos work` behavior changes outside approved scope.
- **Source modification in stable alpha** — Source-modifying behavior is added to the stable alpha path.
- **Autonomous execution promoted** — Autonomous execution loop is promoted or implied stable.
- **Benchmark claims without validation** — Benchmark claims are added without completed validation evidence.
- **Ornith/local model false claim** — Ornith/local model support is claimed automatic without validation.
- **Frontend promoted** — Frontend is promoted to stable alpha.
- **API server promoted** — API server is promoted to stable alpha.
- **New dependency** — New dependency is added without explicit review.
- **Public API, governance, release docs, or ADRs changed outside scope** — Public API, governance docs, release docs, or ADRs change outside scope.
- **Secrets exposed** — Secrets, credentials, API keys, or private tokens are exposed.
- **Large unrelated refactor** — Large unrelated refactor appears outside approved scope.
- **Cannot separate changes** — The agent cannot separate task changes from unrelated local changes.
- **Unattended merge** — Merge to main would happen without human approval.

## Soft warnings

Soft warnings must be visible in the PR body or handoff report. They do not stop execution but must be documented:

- **Docs drift** — docs drift from current code.
- **Partial verification** — verification is partial (not all checks run).
- **Local service unavailable** — local service (API server, frontend) unavailable for manual demo.
- **Manual demo not run** — manual demo was not executed.
- **Lint not enforced** — lint is not enforced (known follow-up for frontend).
- **E2E not available** — E2E coverage is not available.
- **Frontend/API compatibility not fully covered** — frontend/API compatibility is not fully covered by tests.

## Product boundary reference

### Stable alpha

- `prometheos work` — Repo Workbench CRUD, artifacts, memory, continue, approve
- deterministic static analysis (tree-sitter, no model required)
- WorkContext creation, run, continuation
- risk report and suggested patch plan artifacts
- approval recording
- artifact provenance
- provider routing
- LLM client (OpenAI-compatible)
- provider configuration
- mock provider integration tests
- Linux install smoke CI
- golden path CI

### Experimental

- `prometheos serve` API server
- frontend
- harness execution loop
- model-backed provider paths
- local model / Ornith validation
- autonomous execution loop

### Future (not alpha)

- Brain
- Mnemosyne integration
- cloud/team control plane
- plugin marketplace
- benchmark claims
- autonomous coding claims
