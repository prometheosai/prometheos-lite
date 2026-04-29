A regra de **100 linhas por módulo** é agressiva, mas ótima para forçar separação. Porque aparentemente deixar `server.rs` virar um animal mitológico era inevitável.

## Modules to split first

### 1. `src/api/server.rs`

Split into:

```txt
src/api/
  mod.rs
  router.rs
  health.rs
  projects.rs
  conversations.rs
  messages.rs
  flow_runs/
    mod.rs
    handler.rs
    direct_llm.rs
    planning.rs
    approval.rs
    codegen.rs
    events.rs
    errors.rs
```

Purpose:

* `router.rs`: only route registration.
* `projects.rs`: project endpoints.
* `conversations.rs`: conversation endpoints.
* `messages.rs`: message endpoints.
* `flow_runs/handler.rs`: entrypoint for run flow.
* `flow_runs/direct_llm.rs`: direct chat path.
* `flow_runs/planning.rs`: PRD/planning path.
* `flow_runs/approval.rs`: continue-approved-plan path.
* `flow_runs/codegen.rs`: planner/coder/reviewer execution.
* `flow_runs/events.rs`: WebSocket event helpers.
* `flow_runs/errors.rs`: shared error mapping.

---

### 2. `src/flow/memory.rs`

This is the next monster. Split into:

```txt
src/flow/memory/
  mod.rs
  models.rs
  db.rs
  schema.rs
  repository.rs
  service.rs
  write_queue.rs
  embeddings/
    mod.rs
    provider.rs
    local.rs
    external.rs
    fallback.rs
  vector/
    mod.rs
    backend.rs
    brute_force.rs
  search.rs
  dedup.rs
```

Purpose:

* `models.rs`: `Memory`, `MemoryKind`, `MemoryRelationship`.
* `db.rs`: connection wrapper.
* `schema.rs`: table/index creation.
* `repository.rs`: CRUD queries.
* `service.rs`: orchestration layer.
* `write_queue.rs`: async memory write tasks.
* `embeddings/*`: embedding provider logic.
* `vector/*`: vector search backend.
* `search.rs`: semantic/text search.
* `dedup.rs`: similar-memory detection.

---

### 3. `src/flow/intelligence.rs`

Split into:

```txt
src/flow/intelligence/
  mod.rs
  llm_provider.rs
  openai_provider.rs
  model_router.rs
  llm_utilities.rs
  tools/
    mod.rs
    tool.rs
    runtime.rs
    sandbox_profile.rs
    command_executor.rs
```

Purpose:

* `llm_provider.rs`: trait only.
* `openai_provider.rs`: provider adapter.
* `model_router.rs`: fallback routing.
* `llm_utilities.rs`: retry/stream helpers.
* `tools/tool.rs`: `Tool` trait.
* `tools/runtime.rs`: tool execution.
* `tools/sandbox_profile.rs`: permissions.
* `tools/command_executor.rs`: command running.

---

### 4. `src/flow/flow.rs`

Split into:

```txt
src/flow/execution/
  mod.rs
  flow.rs
  builder.rs
  lifecycle.rs
  retry.rs
  validation.rs
  cycle_detection.rs
  nested_flow.rs
```

Purpose:

* `flow.rs`: `Flow` struct + basic methods.
* `builder.rs`: `FlowBuilder`.
* `lifecycle.rs`: hooks.
* `retry.rs`: retry execution.
* `validation.rs`: structure validation.
* `cycle_detection.rs`: DFS cycle detection.
* `nested_flow.rs`: `FlowNode`.

---

### 5. `src/llm/mod.rs`

Split into:

```txt
src/llm/
  mod.rs
  client.rs
  config.rs
  request.rs
  response.rs
  streaming.rs
  retry.rs
```

Purpose:

* `client.rs`: `LlmClient`.
* `request.rs`: request structs.
* `response.rs`: response structs.
* `streaming.rs`: SSE parsing.
* `retry.rs`: retry wrapper.
* `config.rs`: `from_config`.

---

### 6. `src/config/mod.rs`

Split into:

```txt
src/config/
  mod.rs
  app_config.rs
  defaults.rs
  env.rs
  memory_budget.rs
  loader.rs
```

Purpose:

* Keep config dumb and readable. Humanity may recover.

---

### 7. `src/db/repository.rs`

Likely split into:

```txt
src/db/
  mod.rs
  models.rs
  connection.rs
  schema.rs
  projects.rs
  conversations.rs
  messages.rs
  flow_runs.rs
  artifacts.rs
  repository.rs
```

Purpose:

* One file per aggregate.
* `repository.rs` only exposes trait/interfaces.

---

## Suggested utils separation

Create:

```txt
src/utils/
  mod.rs
  time.rs
  ids.rs
  json.rs
  errors.rs
  paths.rs
  async_task.rs
  validation.rs
```

Use cases:

* `time.rs`: `Utc::now()`, timestamp helpers.
* `ids.rs`: UUID helpers.
* `json.rs`: safe JSON extraction/serialization.
* `errors.rs`: common error mapping.
* `paths.rs`: DB/config/file path helpers.
* `async_task.rs`: spawn/log failure wrappers.
* `validation.rs`: shared guard functions.

## Best refactor order

1. `api/server.rs`
2. `flow/memory.rs`
3. `flow/intelligence.rs`
4. `flow/flow.rs`
5. `db/repository.rs`
6. `llm/mod.rs`
7. `config/mod.rs`

Core rule: files under 100 lines is fine, but don’t split blindly. Split by **reason to change**, not just by line count, otherwise you create 80 tiny files and call it architecture, which is how software becomes confetti.
