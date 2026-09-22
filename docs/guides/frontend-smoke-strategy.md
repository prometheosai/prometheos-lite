# Frontend Smoke / E2E Strategy

## Status

Experimental — design document only. Not implemented.

## Purpose

Define the smallest useful smoke and E2E strategy for the experimental PrometheOS Lite frontend.

The goal is to verify that the frontend can start, serve pages, and communicate with the API server, without claiming full frontend coverage or promoting the frontend to stable alpha.

## What the smoke test should prove

### Level 1 — Build & lint (enforced)

Already gated in CI:

- `npm run build` — confirms compilation, type checking, and static generation succeed.
- `npm run lint` — confirms no lint errors (warnings allowed, documented).

These prove the frontend is structurally sound.

### Level 2 — Server smoke (proposed)

Verify the frontend dev or production server can start and respond:

- Start `npm run start` (or equivalent production server).
- Confirm the process binds to the expected port.
- Confirm `GET /` returns HTTP 200.
- Confirm the response contains the expected HTML shell.

This proves the frontend is deployable and serves content.

### Level 3 — API connectivity smoke (proposed)

Verify the frontend can reach the API server:

- This requires both the API server and frontend running.
- Confirms the frontend's hardcoded API base URL (`http://127.0.0.1:3000`) is reachable.
- Does not assert specific API behavior — only that the frontend can connect.

### Level 4 — Minimal E2E (future)

A single Playwright test that:

- Starts both API server and frontend.
- Opens the browser to the frontend URL.
- Verifies the page renders without JavaScript errors.
- Confirms a known element or heading is present.

This level requires explicit Playwright approval and is not part of the current implementation plan.

## What the smoke test should NOT prove

- Full frontend coverage.
- Visual regression coverage.
- API behavior correctness.
- WebSocket functionality.
- Authentication or authorization.
- Production deployment readiness.
- Frontend/API route compatibility completeness.
- Multi-user or concurrent access.
- Mobile responsiveness.

## Proposed implementation path

### Step 1 — Server smoke script

Create a small Node.js script (`frontend/scripts/smoke.mjs`) that:

1. Spawns the production server (`npm run start`).
2. Waits for the server to be ready (poll `http://localhost:3001`).
3. Verifies `GET /` returns 200.
4. Verifies the response contains `<!DOCTYPE html>` or a known shell marker.
5. Kills the server process.
6. Exits 0 on success, non-zero on failure.

The script uses only Node.js built-in modules (`child_process`, `http`). No new dependencies.

### Step 2 — repository-native integration

Run the checked-in frontend suite:

```bash
python3 scripts/local_ci.py run --suite frontend
```

The suite runs install, build, lint, and smoke locally and records exact-commit evidence.

### Step 3 — API connectivity smoke (future)

After the basic server smoke is stable, add a second script that starts both `prometheos serve` and the frontend, then confirms the frontend can reach the API.

Note: This requires the Rust binary to be built, so it may be better as a separate CI job or post-merge check.

## Verification criteria

The smoke test passes when:

- `npm run build` passes.
- `npm run lint` passes.
- The server starts within a reasonable timeout (e.g., 15 seconds).
- `GET /` returns HTTP 200.
- The response contains valid HTML.

## Non-goals

- Adding Playwright without explicit approval.
- Adding npm test dependencies.
- Adding E2E to the required gate without explicit approval.
- Changing frontend source code.
- Changing API behavior.
- Promoting frontend or API server to stable alpha.
- Adding visual regression or snapshot tests.

## Relationship to other docs

- [Frontend Alpha Status](frontend-alpha-status.md) — frontend maturity and promotion criteria.
- [Serve / API Server Status](serve-api-status.md) — API server experimental status.
- [Local Frontend Demo](local-frontend-demo.md) — how to run the frontend locally.
- [Product Surface Inventory](product-surface-inventory.md) — full surface classification.
