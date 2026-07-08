# Agent Budgets and Path Controls

This spec defines the default budgets and path controls that coding agents must follow when working in this repository.

## Default file budget

- Prefer **5 files or fewer** per PR.

## Default line budget

- Prefer **200 net changed lines or fewer** per PR.

## Large-change escalation

If a PR exceeds either budget:

1. Explain why the change exceeds budget.
2. Confirm the change is within approved scope.
3. Split into smaller PRs when possible.

## Path allowlist

Agents should only touch paths required by the approved task.

## Path risk levels

| Risk level | Paths                                               |
|------------|------------------------------------------------------|
| low        | docs, specs, comments                                |
| medium     | CI configs, tests, frontend docs, scripts            |
| high       | runtime Rust behavior, provider/model paths, API semantics, autonomous execution, release/governance docs |

## Dependency budget

- Default: **no new dependencies**.
- Any dependency change requires explicit approval.

## Verification budget

- Verification must match the surfaces touched.
- Must not claim skipped checks passed.

## Stop conditions

Agents must stop and escalate when:

- Budget exceeded without approval.
- Dependency required.
- Public API change required.
- Product boundary unclear.
- Verification too expensive or unavailable.
