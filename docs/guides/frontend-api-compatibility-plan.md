# Frontend/API Compatibility Smoke Plan

## Status

Experimental — plan document only. Not implemented.

## Purpose

Define the narrowest frontend/API compatibility smoke path for the experimental PrometheOS Lite surfaces.

The goal is to verify that the frontend can reach the API server and exercise the simplest CRUD path, without claiming full route coverage, API stability, or frontend alpha promotion.

## Existing coverage context

The API server has unit-level smoke tests in `tests/api_server_smoke.rs` that verify:

- `GET /health` returns 200.
- `GET /runtime/stack` returns 200.
- The assembled router serves these endpoints.
- The assembled router rejects unknown routes (404).

These tests use in-memory state and do not require a running server or model provider. They prove the router wires correctly but do not exercise the routes the frontend actually calls.

## Frontend API surface

From `frontend/src/lib/api.ts`, the frontend uses:

| Route | Method | Purpose |
|---|---|---|
| `/projects` | GET | List all projects |
| `/projects` | POST | Create a new project |
| `/projects/:id/conversations` | GET | List conversations for a project |
| `/conversations` | POST | Create a new conversation |
| `/conversations/:id/messages` | GET | List messages in a conversation |
| `/messages` | POST | Create a new message |
| `/conversations/:id/run` | POST | Run flow execution (requires model provider) |
| `/ws/runs/:id` | WebSocket | Stream flow events (excluded from this plan) |
| `/runtime/stack` | GET | Get runtime model/provider stack |

## Proposed smoke path

The narrowest path exercises the simplest frontend-reachable CRUD cycle without requiring a model provider or flow execution:

### Step 1 — Server health (already covered)

```
GET /health → 200 {"status":"ok"}
```

### Step 2 — Project CRUD (proposed)

```
POST /projects → 201 { id, name, created_at, updated_at }
  Body: { "name": "smoke-test-project" }

GET /projects → 200 [ { id, name, ... } ]
  Verify the created project appears in the list.

GET /projects/:id → 200 { id, name, ... }
  Verify the single project endpoint returns the expected project.
```

### Step 3 — Conversation CRUD (proposed, after Step 2 stabilizes)

```
POST /conversations → 201 { id, project_id, title, ... }
  Body: { "project_id": "<project_id>", "title": "smoke-test-conversation" }

GET /projects/:id/conversations → 200 [ { id, title, ... } ]
  Verify the conversation appears in the project's conversation list.
```

### Step 4 — Message CRUD (future, after Steps 2–3 stabilize)

```
POST /messages → 201 { id, conversation_id, role, content, ... }
  Body: { "conversation_id": "<conv_id>", "role": "user", "content": "hello" }

GET /conversations/:id/messages → 200 [ { id, role, content, ... } ]
  Verify the message appears in the conversation.
```

## What the smoke test should prove

- The API server starts and serves requests.
- The simplest project CRUD path works end-to-end.
- The frontend's API client can successfully call these routes.
- Response formats match what the frontend expects.

## What the smoke test should NOT prove

- Full API route coverage.
- API behavior correctness beyond CRUD.
- Flow execution or model invocation.
- WebSocket functionality.
- Authentication or authorization.
- Production deployment readiness.
- Frontend/API route compatibility completeness.
- Visual or UI correctness.

## Implementation approach

### Phase 1 — Rust integration tests (preferred)

Add new tests to `tests/api_server_smoke.rs` or a new `tests/api_frontend_compatibility.rs`:

```rust
// Pseudocode for proposed test
#[tokio::test]
async fn frontend_compatibility_project_crud() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    // POST /projects
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/projects")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"name":"smoke-test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 201);

    // GET /projects
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    // Verify response body contains the created project
}
```

This approach:
- Uses the same test infrastructure as the existing smoke tests.
- No running server needed — uses the in-memory router directly.
- No model provider needed — project/conversation CRUD is data-only.
- No new dependencies.
- Runs in CI as part of `cargo test`.

### Phase 2 — Full-stack smoke (future)

After Phase 1 is stable, a shell script or integration job that:
1. Builds the Rust binary.
2. Starts `prometheos serve` in the background.
3. Uses `curl` to verify the frontend-reachable routes.
4. Shuts down the server.

This requires the Rust build and may be better as a separate CI job.

## What this plan does NOT do

- Add Playwright or browser automation.
- Add npm test dependencies.
- Run the frontend server as part of the test.
- Verify UI rendering or DOM output.
- Change API semantics.
- Add WebSocket tests.
- Promote frontend or API server to stable alpha.
- Claim alpha-level API compatibility.

## Relationship to other docs

- [Serve / API Server Status](serve-api-status.md) — API server experimental status.
- [Frontend Alpha Status](frontend-alpha-status.md) — frontend maturity and promotion criteria.
- [Frontend Smoke / E2E Strategy](frontend-smoke-strategy.md) — frontend smoke strategy (Level 3 covers API connectivity).
- [Product Surface Inventory](product-surface-inventory.md) — full surface classification.
- [API Server Smoke Test](../../tests/api_server_smoke.rs) — existing API smoke coverage.
