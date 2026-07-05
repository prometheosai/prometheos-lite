# Frontend Alpha Status

The PrometheOS Lite frontend is **experimental but present**.

It exists as a local web UI surface for exploring PrometheOS Lite concepts, but it is not part of the stable alpha promise.

The stable alpha path remains the CLI:

```bash
prometheos work ...
```

## Status

Status: **experimental**

Decision: present, documented, not stable alpha.

## Why it is not stable alpha yet

- Frontend CI (build & typecheck) is now gated on PRs via `.github/workflows/frontend-ci.yml`.
- Playwright/E2E coverage is not enforced in CI yet.
- Frontend/API integration is not part of the stable alpha golden path.
- The stable alpha release can be used entirely through the CLI.
- Routes and UI flows may change.

## What exists

- Next.js 15 application (TypeScript, Tailwind CSS, shadcn/ui)
- Project pages (CRUD UI via `GET/POST /projects`)
- Conversation UI (chat interface with sidebar layout)
- Settings and profile pages
- Search command palette (CMDK-based)
- WebSocket flow event UI (streams execution events from `ws://127.0.0.1:3000/ws/runs/:id`)

## How it relates to the API server

The frontend depends on the experimental API server.

The default configuration is:

| Component | Default URL | Notes |
|-----------|-------------|-------|
| API server | `http://127.0.0.1:3000` | Started via `prometheos serve` |
| Frontend dev server | `http://localhost:3001` | Started via `npm run dev` |
| WebSocket | `ws://127.0.0.1:3000/ws/runs/:id` | Hardcoded in `src/lib/api.ts` |

To run both locally:

```bash
# Terminal 1: start the API server
prometheos serve

# Terminal 2: start the frontend
cd frontend
npm install
npm run dev
```

Then open `http://localhost:3001`.

## How it relates to Repo Workbench

Repo Workbench does not require the frontend.

The stable alpha workflow remains:

```bash
prometheos work create ...
prometheos work run ...
prometheos work artifacts ...
prometheos work memory show ...
prometheos work continue ...
```

## What is safe to rely on

Safe to rely on for alpha:

- CLI golden path
- Repo Workbench artifacts
- Provenance
- Local file-backed workbench state
- Documented provider configuration
- Frontend build & typecheck (enforced in CI via `.github/workflows/frontend-ci.yml`)

Not safe to rely on yet:

- Frontend UI flows
- Frontend/API route compatibility
- Visual regression coverage
- Production deployment behavior

## Current decision

The frontend stays in the repo as an experimental surface.

It should not be removed.

It should not be promoted to stable alpha until at least:

- [x] Frontend build is verified in CI (PR #60).
- [ ] Lint/typecheck pass in CI.
- [ ] At least one smoke or E2E test is enforced.
- [ ] API server compatibility is covered by smoke tests.
- [ ] README and alpha docs are updated accordingly.

## Next recommended PRs

- Add frontend lint/typecheck CI.
- Add minimal frontend smoke/E2E test.
- Document local frontend demo once CI proves it.
