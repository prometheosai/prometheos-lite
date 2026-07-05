<!-- PrometheOS™ Proprietary Software
Copyright © 2026 Memento Mori Labs LLC. All Rights Reserved.
This document contains proprietary and confidential information. Unauthorized use, modification, distribution, or copying is prohibited.
See the LICENSE file in the repository root for full terms.
-->

# PrometheOS Design System

## Status

This document defines the active design system for the PrometheOS frontend and records the legacy stylesheet that remains in the repository for reference only.

## Active Design System

### Canonical Style Source

The active design system is driven by the following sources:

* `frontend/app/globals.css`
* `frontend/app/layout.tsx`
* `frontend/components/ui/*`
* `frontend/components/prometheos/page-shell.tsx`
* `frontend/components/prometheos/auth-shell.tsx`
* `frontend/components/prometheos/workspace-dashboard.tsx`
* `backend/crates/gateway/src/lib.rs` `APP_HTML` fallback surface

### Visual Direction

PrometheOS uses a dark command-surface aesthetic with restrained neon accents, translucent panels, mono operational labels, and a display typographic layer for major page titles.

### Color Tokens

Defined in `frontend/app/globals.css`.

* Background: `#000000`
* Foreground: `#ffffff`
* Primary accent: `#3b82f6`
* Secondary glow: `#06b6d4`
* Panels: translucent white overlays on black
* Borders: low-contrast white with cyan glow on emphasis states

### Typography

Defined in `frontend/app/layout.tsx` and `frontend/app/globals.css`.

* Sans: `Inter`
* Mono: `Geist Mono`
* Display: `Space Grotesk`

Usage rules:

* Use display type for major page titles and feature headlines.
* Use mono text for status chips, labels, telemetry tags, and compact metadata.
* Use sans for body content and controls.

### Surface Language

Primary surfaces should use:

* rounded corners in the `xl` to `2xl` range
* translucent dark fills
* subtle white borders
* ambient cyan glow via `workspace-panel-glow`
* backdrop blur where panels sit over gradients

The shared page-shell helpers centralize this treatment:

* `PrometheosPageShell`
* `PrometheosPageHeader`
* `PrometheosStatusBadge`
* `prometheosSurfaceClass`
* `AuthShell`

### Interaction Language

* Buttons use the shadcn primitives configured in `frontend/components.json` with PrometheOS token overrides.
* Hover states should feel mechanical and restrained, not playful.
* Motion should be ambient and low-amplitude.
* Operational pages should prefer dense but readable layouts over marketing whitespace.

## Route-Level Usage

### Public Marketing Surface

The landing page uses the active PrometheOS design system with a more expressive hero presentation:

* `frontend/app/page.tsx`
* `frontend/app/(public)/page.tsx`

### Dashboard Surface

Dashboard routes must use the PrometheOS command-surface shell:

* `frontend/app/(dashboard)/layout.tsx`
* `frontend/app/(dashboard)/app/home/page.tsx`
* `frontend/app/(dashboard)/app/workspace/page.tsx`
* `frontend/app/(dashboard)/app/memory/page.tsx`
* `frontend/app/(dashboard)/app/settings/page.tsx`
* `frontend/app/(dashboard)/app/profiles/page.tsx`
* `frontend/app/(dashboard)/app/integrations/page.tsx`

### Auth Surface

Auth routes use the same tokens and panel language, but through the dedicated centered shell:

* `frontend/app/(auth)/login/page.tsx`
* `frontend/app/(auth)/signup/page.tsx`

### Gateway Fallback Surface

The Rust gateway still serves a direct HTML fallback for operational access. That surface must follow the same token and panel language even though it is inline HTML instead of a Next route:

* `backend/crates/gateway/src/lib.rs` `APP_HTML`
* served at `/app/workspace`
* served at `/app/memory`

## Component Rules

### Use These

* shadcn UI primitives under `frontend/components/ui/*`
* shared PrometheOS shells under `frontend/components/prometheos/*`
* tokenized Tailwind variables from `frontend/app/globals.css`

### Avoid These

* introducing a separate font stack outside the root layout
* hardcoding a second page shell for dashboard-style routes
* reintroducing light-theme-neutral card styling on operational routes
* adding another standalone global stylesheet as an active source of truth
* treating the gateway fallback HTML as exempt from the PrometheOS command-surface rules

## Legacy Reference

### Dormant Stylesheet

`frontend/styles/globals.css` is not imported by the app and is considered legacy reference material only.

It represents an older neutral shadcn baseline and must not be used as the active source of truth unless it is intentionally reworked and wired back into the app.

## Maintenance Notes

When adding a new page:

* dashboard routes should start from `PrometheosPageShell`
* auth routes should start from `AuthShell`
* shared panels should inherit `prometheosSurfaceClass`
* new tokens belong in `frontend/app/globals.css`, not page-local CSS

When changing the design system:

* update the source files listed under Canonical Style Source
* update this document in the same change
* explicitly note whether a legacy surface was migrated or deprecated
* if the change affects `APP_HTML`, keep the gateway fallback visually aligned with the Next dashboard rather than introducing a separate fallback theme

## Visual Regression Coverage

Playwright screenshot coverage for the normalized dashboard and auth routes lives in:

* `frontend/tests/e2e/design-system-visual.spec.ts`

Snapshots are intentionally based on deterministic API mocks so style drift is caught without requiring live backend services.
