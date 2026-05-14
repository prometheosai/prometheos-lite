# Handoff

## Objective
Move PrometheOS Lite from hardcoded identity replies toward a runtime that is OpenRouter-first, exposes its actual runtime stack in the UI, propagates memory-retrieval failures instead of hiding them, and presents PrometheOS identity/capabilities through the personality system with live tool awareness. This matters because the product goal is a local-first harness assistant that can explain what it is actually capable of doing, route work correctly, and surface the true model/tool stack instead of stale frontend defaults or raw model self-identification.

## Current State
### Completed
- `prometheos.config.json` is switched to an OpenRouter-first runtime stack with primary/fallback chat models and OpenRouter embeddings.
- Backend runtime wiring supports `embedding_model` and OpenRouter embeddings via `src/cli/runtime_builder.rs`, `src/config/settings/defaults.rs`, and `src/config/settings/types.rs`.
- `src/context/builder.rs` now propagates memory retrieval failures instead of silently degrading.
- `src/flow/memory/embedding.rs` now sends the configured embedding model to local/OpenAI-compatible embedding endpoints and parses both legacy and OpenAI-style responses.
- `src/api/health.rs` and `src/api/router.rs` now expose `/runtime/stack`; verified response earlier in session showed:
  - provider: `openrouter`
  - primary model: `owl-alpha`
  - fallbacks: `qwen/qwen3-8b:free`, `mistralai/mistral-7b-instruct:free`
  - embedding model: `openai/text-embedding-3-small`
- Frontend runtime stack wiring exists in `frontend/src/lib/api.ts`, `frontend/src/components/layout/app-layout.tsx`, and `frontend/src/components/layout/right-sidebar.tsx`.
- `src/flow/factory/builtin_nodes.rs` now adds:
  - PrometheOS Lite identity contract
  - deterministic sanitizer against OWL/ZOO self-identification
  - live tool inventory rendering from the runtime tool registry
  - personality-shaped identity/capability/tool framing
- `src/personality/prompt.rs` now defines identity/capability/tool-enumeration style helpers per personality mode.
- `src/api/flow_runs/handler.rs` now auto-selects a personality mode from the incoming user message and passes it into flow execution.
- `src/flow/intelligence/tool.rs` now exposes tool metadata listing for live prompt/tool enumeration.
- API/DB verification from this session confirmed:
  - identity replies are stored as PrometheOS Lite, not OWL/ZOO
  - tool-enumeration replies list the actual registered tools with examples
  - a direct/honest prompt produced a more direct identity answer, showing personality-mode influence is active
- All Copilot-started terminals were killed, and a final process check found no remaining `prometheos-lite`, `cargo`, or `rustc` processes.

### Partially completed
- Frontend/browser validation is incomplete after the latest personality/tool-aware backend changes because the backend was intentionally shut down at the end of the session. The browser page can also show stale historical conversation messages.
- `flows/chat.flow.yaml` and `flows/memory-augmented.json` still contain explicit identity prompt text in addition to the new dynamic backend contract. This duplicates responsibility and may drift.
- `flows/memory-augmented.json` currently contains the phrase `your Omini System`; this looks like either a typo or user-specific wording and was not normalized in this session.
- `src/cli/commands/serve.rs` still constructs an `AppState` embedding provider using a hardcoded local URL (`http://127.0.0.1:1234`) even though runtime/memory embedding selection was generalized elsewhere. This may be intentional for API state construction, but it is inconsistent with the rest of the runtime builder changes.
- `frontend/src/app/conversations/[id]/page.tsx` is modified and uses live flow/WebSocket events, but no browser-side regression pass was performed after the latest backend changes.

### Not started
- No end-to-end frontend verification of the latest changes with a freshly restarted backend and a clean conversation.
- No user-selectable personality-mode control in the UI; current mode selection is heuristic and server-side.
- No dedicated API endpoint for listing live tools; tool enumeration currently only happens through prompt injection.
- No documentation update was made for the new personality-shaped identity/tool behavior.
- No commit was created.

### Blocked
- There is no hard technical blocker in the codebase from inspected evidence.
- Immediate live verification is blocked until another developer/agent restarts the backend, because all terminals/processes were intentionally terminated at the end of the session.
- Uncertainty: `Cargo.toml` still shows as modified in `git status`, but `git diff -- Cargo.toml` produced only an LF/CRLF warning and no content diff. Treat it as likely line-ending noise unless proven otherwise.

## Active Files / Files in Flight
### Cargo.toml
- Status: modified in `git status`; no content diff shown by `git diff -- Cargo.toml`
- Purpose: Rust workspace/package manifest
- What changed: no verified content change was observed; only a line-ending warning was emitted
- What still needs work: confirm whether this is pure CRLF noise before including it in any commit
- Risk level: low

### flows/chat.flow.yaml
- Status: modified
- Purpose: direct chat flow definition for conversation/question intents
- What changed: added a `prompt_template` that explicitly identifies the assistant as PrometheOS Lite and describes flow/memory/tool/runtime capabilities
- What still needs work: decide whether identity belongs here, in the backend prompt contract, or both; avoid duplicated prompt authority
- Risk level: medium

### flows/memory-augmented.json
- Status: modified
- Purpose: memory-augmented flow definition for context retrieval and write-back
- What changed: updated `llm_processor` prompt template to identify as PrometheOS Lite and mention flow routing/memory/tool orchestration
- What still needs work: review the phrase `your Omini System`; also decide whether this file should keep static identity wording now that backend identity is dynamic
- Risk level: medium

### frontend/src/app/conversations/[id]/page.tsx
- Status: modified
- Purpose: standalone conversation page with message list, flow execution, and WebSocket updates
- What changed: page uses `runFlow`, WebSocket events, temporary local message insertion, and execution timeline/thinking states
- What still needs work: verify no duplicate-assistant-message or stale-history issues after backend restarts; confirm this page still matches the main layout behavior
- Risk level: medium

### frontend/src/components/layout/app-layout.tsx
- Status: modified
- Purpose: main app shell and model/provider display source of truth
- What changed: fetches `/runtime/stack`, stores runtime model stack, and feeds active runtime provider/model information into the sidebar while keeping static fallbacks
- What still needs work: verify runtime stack loads reliably when backend is unavailable/restarted and that fallback UI is acceptable
- Risk level: medium

### frontend/src/components/layout/right-sidebar.tsx
- Status: modified
- Purpose: right sidebar showing flow state, models, memory, policy, and debug panels
- What changed: now accepts active runtime stack/provider props instead of relying solely on hardcoded model labels
- What still needs work: browser verification of the Models tab against a live backend response
- Risk level: medium

### frontend/src/lib/api.ts
- Status: modified
- Purpose: frontend API client definitions
- What changed: added `RuntimeModelStack` typing and `getRuntimeModelStack()` for `/runtime/stack`
- What still needs work: none obvious beyond runtime UI verification
- Risk level: low

### prometheos.config.json
- Status: modified
- Purpose: local runtime configuration for provider/model/embedding routing
- What changed: set provider to `openrouter`, primary model to `owl-alpha`, configured OpenRouter embedding endpoint/model, and defined fallback chains in `llm_routing`
- What still needs work: verify chosen models/embedding endpoint are correct for the intended environment and available credits/keys
- Risk level: high

### request.json
- Status: untracked
- Purpose: ad hoc request payload for embedding or API testing
- What changed: contains `{ "model":"text-embedding-nomic-embed-text-v1.5", "input":["hello"] }`
- What still needs work: decide whether to keep as a reproducible test artifact or remove before commit
- Risk level: low

### src/api/flow_runs/handler.rs
- Status: modified
- Purpose: API entrypoint for conversation flow runs
- What changed: preserves memory context loading, now auto-selects a personality mode via `ModeSelector`, and passes it into `ExecutionOptions`
- What still needs work: decide whether heuristics are sufficient or whether explicit UI/API personality selection is required
- Risk level: medium

### src/api/health.rs
- Status: modified
- Purpose: health and runtime metadata endpoints
- What changed: added `RuntimeStackResponse` and `/runtime/stack` support with provider label, primary model, fallbacks, and embedding metadata
- What still needs work: none obvious beyond consumer/UI verification
- Risk level: low

### src/api/router.rs
- Status: modified
- Purpose: API route registration
- What changed: registered `GET /runtime/stack`
- What still needs work: none obvious
- Risk level: low

### src/cli/commands/serve.rs
- Status: modified
- Purpose: API server startup path
- What changed: updated local embedding provider construction to accept optional embedding model; comments note runtime already includes OpenRouter embedding support
- What still needs work: reconcile the remaining hardcoded local embedding URL used for `AppState` construction with the generalized runtime builder
- Risk level: high

### src/cli/runtime_builder.rs
- Status: modified
- Purpose: runtime construction for API/CLI
- What changed: generalized embedding provider selection, supports configured `embedding_model`, and wires OpenRouter/Jina/local embedding providers; still builds full runtime with tool runtime, router, memory service, and trace storage
- What still needs work: review consistency with `serve.rs` and confirm embedding-provider behavior in all provider combinations
- Risk level: high

### src/config/settings/defaults.rs
- Status: modified
- Purpose: default config values
- What changed: includes defaults for `embedding_model` and multi-provider `llm_routing` chains
- What still needs work: ensure defaults still match the intended product direction and available models
- Risk level: medium

### src/config/settings/types.rs
- Status: modified
- Purpose: config schema
- What changed: includes `embedding_model` and `llm_routing` support in `AppConfig`
- What still needs work: none obvious beyond config migration/documentation
- Risk level: low

### src/context/builder.rs
- Status: modified
- Purpose: prompt/context construction and memory retrieval integration
- What changed: `build_with_memory_retrieval` now propagates semantic search failures instead of silently returning empty memory
- What still needs work: ensure callers handle surfaced failures in a user-friendly way; more integration testing would help
- Risk level: medium

### src/flow/factory/builtin_nodes.rs
- Status: modified
- Purpose: implementations of built-in flow nodes, especially the LLM node
- What changed: LLM node now receives tool runtime, builds a personality-shaped identity contract, injects live tool inventory with examples, sanitizes leaked OWL/ZOO self-identification, and stores sanitized output
- What still needs work: evaluate whether duplicated static flow prompts should be removed now that this file is the main identity authority; add focused tests around sanitizer and tool inventory if desired
- Risk level: high

### src/flow/factory/node_factory.rs
- Status: modified
- Purpose: creates built-in nodes from flow definitions
- What changed: now passes `tool_runtime` into `LlmNode`
- What still needs work: none obvious
- Risk level: low

### src/flow/intelligence/tool.rs
- Status: modified
- Purpose: tool registry/runtime and tool metadata
- What changed: `ToolRegistry` now exposes `list_tool_metadata()` for live prompt-time tool enumeration
- What still needs work: if tool inventory should be API-visible outside prompts, add a dedicated endpoint or structured response path
- Risk level: medium

### src/flow/memory/embedding.rs
- Status: modified
- Purpose: embedding provider implementations
- What changed: local embedding provider now sends optional configured model and parses both legacy and OpenAI-style embedding responses; OpenRouter embedding provider accepts a preferred model
- What still needs work: broader validation of Jina/OpenRouter/local providers; current Jina parsing logic should be reviewed because it appears unusual and was not reworked in this session
- Risk level: high

### src/personality/prompt.rs
- Status: modified
- Purpose: personality-mode prompt shaping
- What changed: added `identity_style`, `capability_style`, and `tool_enumeration_style` helpers so personality affects more than tone
- What still needs work: add tests or explicit acceptance criteria for mode-specific behavior if the product depends on it
- Risk level: medium

### test_embedding_provider/Cargo.lock
- Status: untracked
- Purpose: lockfile for the helper embedding test crate
- What changed: generated as part of the untracked helper crate
- What still needs work: keep only if the helper crate is intended to stay in the repo
- Risk level: low

### test_embedding_provider/Cargo.toml
- Status: untracked
- Purpose: manifest for a helper crate used to test the Jina embedding API
- What changed: defines a tiny async binary depending on `reqwest`, `serde_json`, and `tokio`
- What still needs work: decide whether this helper crate should be tracked, documented, or removed
- Risk level: low

### test_embedding_provider/src/main.rs
- Status: untracked
- Purpose: helper binary to call Jina embeddings directly using `JINA_API_KEY`
- What changed: sends a request to `https://api.jina.ai/v1/embeddings` and prints status/embedding length
- What still needs work: decide whether this should remain as a reproducible diagnostic tool or be removed from the repo
- Risk level: low

## Session Changes
- Audited the current working tree with `git status --short`, `git status --short --untracked-files=all`, `git diff --stat`, and `git diff --name-status`.
- Replaced the previous stale `handoff.md` context with this repository-grounded audit.
- Verified from current file contents that the active work is centered on:
  - OpenRouter-first config/runtime wiring
  - embedding model support and memory-retrieval error propagation
  - runtime stack API + frontend consumption
  - PrometheOS Lite identity enforcement
  - personality-driven identity/capability/tool framing
  - live tool enumeration from the runtime registry
- Verified via reliable command output in this session that:
  - `cargo check` passed after the personality/tool wiring changes
  - `/runtime/stack` previously returned the configured OpenRouter runtime stack
  - API/DB checks produced stored assistant responses that identified as PrometheOS Lite and enumerated the live tools with examples
- Verified that all Copilot-started terminals were terminated and that no remaining `prometheos-lite`, `cargo`, or `rustc` processes were running at the end of the session.

## Failed Attempts
- Attempt: `cd /d e:\Projects\PrometheOS-Lite && cargo run -- serve`
- Result: PowerShell parser error
- Why it failed: `&&` is not a valid statement separator in Windows PowerShell 5.1
- Do not repeat because: use `Set-Location -LiteralPath ...; cargo run -- serve` in PowerShell

- Attempt: `cd /d e:\Projects\PrometheOS-Lite ; cargo run -- serve`
- Result: `Set-Location` positional parameter error
- Why it failed: `cd /d` is CMD syntax, not PowerShell syntax
- Do not repeat because: use `Set-Location -LiteralPath` instead of CMD-style `cd /d`

- Attempt: repeated `cargo run -- serve` invocations while an older server still owned port `3000`
- Result: socket bind conflict (`os error 10048`) or process exit `0xffffffff`
- Why it failed: multiple backend instances were started/stopped across sessions and port ownership became inconsistent
- Do not repeat because: check port/process ownership first or kill stale backend processes before restarting

- Attempt: long one-line PowerShell verification command with embedded Python/PowerShell interpolation for tool verification
- Result: command timed out and entered the background with malformed quoting
- Why it failed: mixed quoting/interpolation made the command string invalid and hard to observe
- Do not repeat because: use simpler staged commands or query the DB in a separate, shorter step

- Attempt: relying on browser history alone to verify identity fixes
- Result: UI still showed OWL/ZOO phrasing
- Why it failed: the browser was showing older stored conversation messages, not necessarily newly generated output
- Do not repeat because: use a fresh conversation and verify stored messages or API responses after the backend restarts

## Commands and Verification
```bash
git status --short
```
# Result: 20 modified tracked files plus untracked `request.json` and `test_embedding_provider/*`; `Cargo.toml` also appears modified in status.

```bash
git status --short --untracked-files=all
```
# Result: confirmed exact untracked files are `request.json`, `test_embedding_provider/Cargo.lock`, `test_embedding_provider/Cargo.toml`, and `test_embedding_provider/src/main.rs`.

```bash
git diff --stat
```
# Result: 20 tracked files with 640 insertions and 166 deletions across backend, frontend, flows, and config.

```bash
git diff --name-status
```
# Result: modified tracked files are the 20 files listed in the Active Files section.

```bash
git diff -- Cargo.toml
```
# Result: only emitted an LF/CRLF warning; no content diff was shown.

```bash
cargo check
```
# Result: passed after fixing a moved-string borrow in `src/api/flow_runs/handler.rs`.

```bash
Invoke-WebRequest -UseBasicParsing -Uri http://127.0.0.1:3000/runtime/stack
```
# Result: earlier in session returned HTTP 200 with provider `openrouter`, primary model `owl-alpha`, fallback models `qwen/qwen3-8b:free` and `mistralai/mistral-7b-instruct:free`, embedding model `openai/text-embedding-3-small`, and embedding dimension `1536`.

```bash
POST /projects
POST /conversations
POST /conversations/:id/run
GET /conversations/:id/messages
```
# Result: verified via API + DB queries that new assistant replies identified as PrometheOS Lite and no longer self-identified as OWL/ZOO.

```bash
python -c "import sqlite3; ... SELECT role,content FROM messages WHERE conversation_id=? ..."
```
# Result: verified stored assistant outputs for:
# - identity answer: PrometheOS Lite, not underlying model
# - tool enumeration answer: `git_diff`, `list_tree`, `patch_file`, `read_file`, `run_command`, `run_tests`, `search_files`, `write_file`, each with examples
# - direct/honest prompt: more direct identity framing, showing personality-mode influence

```bash
Get-Process prometheos-lite,cargo,rustc -ErrorAction SilentlyContinue
```
# Result: final cleanup check reported `none-found` after terminal/process shutdown.
