# Overview / Product Surface Reconcile Queue

## Epic

Overview / Product Surface Reconcile

## Mode

Epic Completion Mode

## Status

Approved queue, not yet executed.

## Owner

Operator: Diego Rhoger

## Purpose

After #67 (frontend/API smoke checks), the implementation loop is stronger, but `OVERVIEW.md` still leads with the older "flow-centric orchestration system" framing. Before adding more runtime surface, the repo's front-door narrative must align with the now-enforced product boundary.

This queue reconciles `OVERVIEW.md` so it clearly states the product posture defined in `AGENTS.md`, `README.md`, and `docs/guides/product-surface-inventory.md`. It does not change behavior, surfaces, or verification scope.

## Source of truth

Agents must read these before executing this queue:

- `AGENTS.md`
- `docs/LOOP_ENGINEERING.md`
- `specs/loop-engineering/AGENT_PROTOCOL.md`
- `specs/loop-engineering/SAFETY_GATES.md`
- `README.md`
- `docs/guides/product-surface-inventory.md`

## Approved scope

Allowed:

- rewriting the `OVERVIEW.md` executive overview to lead with the stable alpha product surface
- adding a product-surface / maturity section to `OVERVIEW.md`
- progress/handoff updates for this queue

Not allowed:

- promoting frontend to stable alpha
- promoting API server to stable alpha
- changing stable alpha scope
- changing `prometheos work` behavior
- adding autonomous execution behavior
- modifying harness autonomous execution
- adding model/provider behavior
- adding Brain or Mnemosyne integration
- changing runtime/API behavior
- adding dependencies
- weakening CI
- rewriting the rest of `OVERVIEW.md` beyond the framing sections (architecture reference body stays)

## Queue

### Task 1 — Reconcile OVERVIEW.md with stable alpha product surface

Scope:

- Replace the opening "flow-centric orchestration system" framing with the product boundary posture.
- State explicitly:
  - `README.md` is the public product front door
  - `AGENTS.md` is the agent operating contract
  - `docs/guides/product-surface-inventory.md` is the maturity source of truth
  - stable alpha = `prometheos work` / Repo Workbench
  - `repo-workbench` = alias / historical surface for the same golden path
  - flow / harness / API / frontend = experimental
  - Brain / Mnemosyne / cloud / autonomous coding = future / not alpha
- Add a short product-surface & maturity table near the top of the document.

Boundaries:

- Do not rewrite the architecture reference body (module map, data flows, deep dives).
- Do not change any runtime behavior or file outside `OVERVIEW.md` and queue files.
- Do not promote any experimental surface.

Verification:

- Docs-only. No Rust/frontend build required by `docs/LOOP_ENGINEERING.md` docs-only rule.
- Confirm the edited `OVERVIEW.md` is valid Markdown and that boundary statements match `product-surface-inventory.md`.
- Rust baseline only if a root file changes behavior (it does not here).

Stop if:

- source-of-truth docs conflict on maturity classification
- the framing would require changing the architecture reference body beyond a clarifying note
- verification reveals the overview contradicts an enforced rule

### Task 2 — Queue handoff and next-loop recommendation

Scope:

- Update `PROGRESS.md`.
- Write final handoff.
- Recommend whether the next queue should focus on:
  - Phase 2 full-stack smoke (per #67 handoff)
  - ReviewOnly GitHub Action implementation
  - loop structure validator

Boundaries:

- Do not start the next queue.
- Do not modify runtime behavior.
- Do not claim completed work that was not verified.

Verification:

- Docs-only.

## Global stop conditions

Stop immediately and hand off if any hard blocker from `specs/loop-engineering/SAFETY_GATES.md` appears.

Also stop if:

- scope expands beyond this queue
- a task requires dependency changes
- a task requires API/runtime behavior changes
- a task requires autonomous execution changes
- verification fails
- source-of-truth docs conflict
- human approval is unclear

## Human approval gate

Agents must not merge PRs.

The operator reviews and approves final merges.
