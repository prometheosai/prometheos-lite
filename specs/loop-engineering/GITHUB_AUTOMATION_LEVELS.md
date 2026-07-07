# GitHub Automation Levels

This spec defines the automation ladder for PrometheOS Lite. Each level increases the degree of automation while maintaining safety gates and human oversight.

| Level | Name              | Description                                      |
|-------|-------------------|--------------------------------------------------|
| 0     | Manual Queue      | Human invokes the coding agent manually           |
| 1     | ReviewOnly        | Read-only PR diff review with structured comments |
| 2     | Assisted Draft PR | Agent creates draft PRs from approved issues      |
| 3     | Epic Completion   | Agent executes approved queues with progress tracking |
| 4     | Self-hosted Runner| PrometheOS runs via self-hosted runner           |
| 5     | Autonomous Patch  | Blocked — requires graduated autonomy approval   |

## Level 0 — Manual Queue

Human invokes the coding agent manually with an approved queue.

No GitHub automation.

This is the current state after PR #64.

## Level 1 — ReviewOnly

GitHub PR automation reviews diffs and posts comments.

- No commits.
- No branch writes.
- No merges.
- No issue-to-PR.

See [ReviewOnly GitHub flow](../guides/prometheos-reviewonly-github-flow.md) for detailed behavior.

## Level 2 — Assisted Draft PR

A label or explicit human command can ask an agent to create a draft PR from an approved issue.

- No merge.
- No dependency changes without approval.
- No runtime promotion.

## Level 3 — Epic Completion

Agent can execute an approved queue, updating progress and handoff files.

- Still no merge.
- Stops at safety gates.

## Level 4 — Self-hosted Runner

PrometheOS can run from a self-hosted GitHub runner or local machine.

- Must still obey all protocol gates.
- Must use path filters and budgets.

## Level 5 — Autonomous Patch

**Blocked.** Only allowed after autonomous-loop graduation criteria are met.

See [Autonomous loop graduation criteria](../research/autonomous-loop-graduation-criteria.md).
