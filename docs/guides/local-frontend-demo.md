# Local Frontend Demo

The PrometheOS Lite frontend is experimental but runnable locally.

This guide shows how to run the local frontend together with the experimental API server.

The stable alpha workflow remains `prometheos work`.

## Status

Status: experimental.

This demo is for local exploration only.

It does not promote the frontend to stable alpha.

## What this demo shows

This local demo can show:

- the experimental Next.js frontend
- the experimental API server
- project and conversation UI surfaces
- frontend/API communication against a local server
- local WebSocket event wiring if the corresponding backend route is exercised

## What this demo does not claim

This demo does not claim:

- stable frontend support
- production deployment support
- full frontend/API compatibility
- E2E coverage
- visual regression coverage
- authentication or authorization coverage
- cloud/team control plane behavior
- Brain or Mnemosyne integration
- autonomous execution safety

## Prerequisites

- Rust toolchain
- Node.js 20
- npm
- PrometheOS Lite repo checked out locally
- frontend dependencies installable with `npm ci`

## Terminal 1: start the API server

From the repo root:

```bash
prometheos serve
```

By default, the API server binds to:

```text
http://127.0.0.1:3000
```

Verify the API server:

```bash
curl http://127.0.0.1:3000/health
```

Expected response:

```json
{"status":"ok"}
```

Optional runtime stack check:

```bash
curl http://127.0.0.1:3000/runtime/stack
```

## Terminal 2: start the frontend

From the repo root:

```bash
cd frontend
npm ci
npm run dev
```

The frontend dev server runs at:

```text
http://localhost:3001
```

Open that URL in a browser.

## Build check

To verify the frontend build locally:

```bash
cd frontend
npm ci
npm run build
```

The same build path is checked in CI by `.github/workflows/frontend-ci.yml`.

## Current limitations

The frontend remains experimental.

Known limitations:

* no E2E test is enforced yet
* no Playwright coverage yet
* frontend/API route compatibility is not fully covered
* no visual regression coverage
* no production deployment guarantee
* lint is not enforced in CI yet
* routes and UI flows may change

## Relationship to stable alpha

The stable alpha workflow does not require the frontend.

Use this for the stable local workflow:

```bash
prometheos work create --repo . --goal "Review this repository" --mode review
prometheos work run <work_id>
prometheos work artifacts <work_id>
prometheos work memory show <work_id>
prometheos work continue <work_id>
```

Use the frontend demo only for local exploration of the experimental UI.
