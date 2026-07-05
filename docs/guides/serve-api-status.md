# `prometheos serve` / API Server Status

`prometheos serve` starts the experimental PrometheOS Lite API server.

The stable alpha surface remains `prometheos work` / Repo Workbench. The API server exists in the codebase and may be useful for local experimentation, but it is not yet the primary supported alpha workflow.

## Status

Status: **experimental**

Reason:

- API server exists and is wired into the CLI.
- Only work-context routes have dedicated integration tests.
- Health endpoint has a placeholder test (skipped in CI).
- Projects, conversations, messages, playbooks, WebSocket, and control panel routes have no HTTP-level integration coverage.
- The stable alpha golden path does not require the API server.
- End-to-end API smoke coverage is not yet part of the alpha release gate.
- Frontend/API integration is not yet promised as stable.

## When to use it

Use `prometheos serve` if you want to experiment with the local API surface.

Use `prometheos work` if you want the stable alpha workflow.

## Starting the server

```bash
prometheos serve
```

Binds to `127.0.0.1:3000` by default.

Flags:

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | Bind address |
| `-p`, `--port` | `3000` | Bind port |

Example with custom port:

```bash
prometheos serve -p 8080
```

## Health check

```bash
curl http://localhost:3000/health
```

Response: `{"status":"ok"}`

Runtime stack info:

```bash
curl http://localhost:3000/runtime/stack
```

Returns configured provider, primary model, fallback models, and embedding model.

## Known API surfaces

| Surface | Status | Notes |
|---------|--------|-------|
| `GET /health` | experimental | Returns `{"status":"ok"}`. No integration test. |
| `GET /runtime/stack` | experimental | Returns provider/model stack. No test. |
| `GET /projects`, `POST /projects` | experimental | Project CRUD. No integration test. |
| `GET /projects/:id/conversations`, `POST /conversations` | experimental | Conversation CRUD. No integration test. |
| `GET /conversations/:id/messages`, `POST /messages` | experimental | Message CRUD. No integration test. |
| `GET /playbooks`, `POST /playbooks`, `GET /playbooks/:id`, `POST /playbooks/:id/update` | experimental | Playbook CRUD. No integration test. |
| `POST /conversations/:id/run` | experimental | Runs flow execution; sends WebSocket events. Indirect integration coverage. |
| `GET /ws/runs/:id` | experimental | WebSocket upgrade for streaming execution events. No test. |
| `GET /work-contexts`, `POST /work-contexts`, `POST /work-contexts/submit-intent`, `GET /work-contexts/:id`, `POST /work-contexts/:id/status`, `GET /work-contexts/:id/artifacts`, `POST /work-contexts/:id/continue`, `POST /work-contexts/:id/run-until-complete` | experimental | Work context lifecycle. 12 ownership/validation integration tests. |
| `GET /work-contexts/:id/harness/run`, `/harness/evidence`, `/harness/patches`, `/harness/validation`, `/harness/review`, `/harness/risk`, `/harness/completion` | experimental | Harness views — read-only result access. Ownership-enforced. Harness/autonomous execution surfaces remain experimental and are governed by the [autonomous loop graduation criteria](../research/autonomous-loop-graduation-criteria.md). |
| `GET /work-contexts/:id/work-quality`, `/work-cost`, `/traces`, `/traces/:run_id` | experimental | Quality, cost, and trace metadata endpoints. |
| `GET /control-panel/stats`, `/metrics`, `/skills`, `/evolutions`, `/job-queue/stats` | experimental | Internal monitoring surfaces. No tests. |

All work-context routes enforce user ownership via `?user_id=` query parameter.

## Relationship to the frontend

A Next.js frontend exists in `frontend/`. It runs on port `3001` and is hardcoded to call the API at `http://127.0.0.1:3000`.

The frontend uses:

- `GET /projects`, `POST /projects`
- `GET /projects/:id/conversations`, `POST /conversations`
- `GET /conversations/:id/messages`, `POST /messages`
- `POST /conversations/:id/run` (sends user messages into flow execution)
- WebSocket at `ws://127.0.0.1:3000/ws/runs/:id` (streams flow events)

The frontend is an **experimental** web UI surface. It is not the stable alpha entrypoint. The stable alpha path remains the CLI.

See the [Frontend Alpha Status](frontend-alpha-status.md) guide for the full decision.

## Relationship to Repo Workbench

Repo Workbench (`prometheos work`) does not require the API server.

The API server provides programmatic access to similar primitives (work contexts, harness, playbooks), but the deterministic golden path runs entirely through the CLI.

## Safety model

The API server does not change the stable alpha safety model.

Current stable alpha guarantees remain:

- no automatic patch application
- no source modification during analysis
- approvals record decisions only
- artifacts are reviewable
- provenance records whether a model was invoked

## Limitations

- Not part of stable alpha golden path.
- A minimal API smoke test verifies `/health` and `/runtime/stack`.
- An assembled router smoke test verifies those safe endpoints are wired through the API router used by `prometheos serve`.
- Broader API route coverage remains experimental.
- Frontend is not stable alpha.
- Routes may change.
- No authentication/authorization middleware — user identity is a query parameter.
- WebSocket has no test coverage.
- Not a cloud/team control plane.
- Not Brain/Mnemosyne integration.
- Not designed for production deployment.

## Next step

Now that handler-level and assembled-router smoke tests exist for safe endpoints, the next step is adding broader API route coverage.
