# Loop Engineering Protocol

A repository-native operating protocol for PrometheOS Lite that defines how coding agents execute bounded work, continue through approved queues, preserve evidence, stop at safety gates, and hand off cleanly.

This protocol is **documentation-only**. It does not add runtime agent-loop behavior, GitHub Actions automation, or tool-specific scripts.

## Purpose

The default workflow for agent-assisted development requires the human operator to prompt every next task:

```
Human → agent → PR → human → Human → agent → PR → human → ...
```

The human acts as the loop scheduler. This protocol replaces that pattern with a durable operating protocol stored in the repository:

```
Human approves an epic goal
→ agent reads repo protocol
→ agent executes queued tasks
→ agent opens PRs or one final epic PR depending on mode
→ agent stops only for blockers, failed verification, or human review gates
```

The repository, not the chat, becomes the source of truth.

## Operating modes

Defines two modes:

### Task Mode

One bounded task. Agent stops after:

- task implementation
- verification
- PR body / report
- handoff

Use for:

- high-risk work
- narrow fixes
- ambiguous tasks
- tasks requiring human direction

### Epic Completion Mode

Approved queue of bounded tasks. Agent continues through the queue until:

- queue complete
- verification fails
- blocker appears
- scope expands
- high-risk escalation appears
- dependency/security/governance/public API change is needed
- human review gate is reached

Agent must preserve evidence per task. Human still performs final review and merge. No unattended merges.

## Lifecycle

Every task follows this lifecycle:

```
INTAKE
→ CONTEXT_SYNC
→ PLAN
→ BRANCH
→ EXECUTE_TASK
→ VERIFY
→ COMMIT
→ REPORT
→ CONTINUE_OR_STOP
→ FINALIZE_PR
→ HUMAN_REVIEW
→ MERGE_ONLY_WITH_APPROVAL
```

For Epic Completion Mode, `CONTINUE_OR_STOP` continues automatically to the next queued bounded task unless a stop condition appears.

## Roles

Defined as responsibilities, not mandatory separate tools:

| Role               | Responsibility                                                   |
| ------------------ | ---------------------------------------------------------------- |
| Operator           | Human owner. Approves scope, high-risk changes, and final merge. |
| Orchestrator Agent | Maintains queue, scope, progress, PR plan, and handoff.          |
| Worker Agent       | Executes one bounded task at a time.                             |
| Verification Agent | Runs checks and records exact evidence.                          |
| Review Agent       | Reviews scope, safety, docs, CI, and product boundary.           |
| Tool Runtime       | Codex, Claude Code, Cursor, OpenCode, or human terminal.         |

One tool may perform multiple roles, but the worker must not approve its own work.

## Sources of truth

1. GitHub issue / PR / human-approved prompt
2. `docs/LOOP_ENGINEERING.md`
3. `specs/loop-engineering/AGENT_PROTOCOL.md`
4. relevant product docs:

   - `README.md`
   - `docs/guides/product-surface-inventory.md`
   - `docs/guides/frontend-alpha-status.md`
   - `docs/guides/serve-api-status.md`
   - `docs/research/model-layer-positioning.md`
   - `docs/research/autonomous-loop-graduation-criteria.md`
5. current PR body
6. CI output
7. handoff/progress file

When sources disagree, stop and report the conflict.

Do not invent intent from chat memory.

## Verification bundles

### Rust/core/docs touching Rust behavior

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

### Docs-only

Docs-only PRs may skip frontend checks unless frontend docs or frontend paths are touched.

### Frontend

```bash
cd frontend
npm ci
npm run build
```

Lint is not required yet unless explicitly scoped (current lint status is a known follow-up).

### API server

```bash
cargo test api_server_smoke
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

### Local model / Ornith validation

Use the documented manual endpoint path. Do not claim benchmark results unless the validation plan has been completed.

### Autonomous loop

Do not promote. Only work on it under explicit scope and with the autonomous loop graduation criteria open.

## Related files

- [Agent Protocol](../specs/loop-engineering/AGENT_PROTOCOL.md) — detailed agent instructions
- [Safety Gates](../specs/loop-engineering/SAFETY_GATES.md) — hard blockers and soft warnings
- [PR Template](../specs/loop-engineering/PR_TEMPLATE.md) — required PR body format
- [Handoff Template](../specs/loop-engineering/HANDOFF_TEMPLATE.md) — handoff report format
- [Progress Schema](../specs/loop-engineering/PROGRESS_SCHEMA.md) — progress tracking format
- [Task Queue Template](../specs/loop-engineering/TASK_QUEUE_TEMPLATE.md) — queue definition format
- [Comment Templates](../specs/loop-engineering/COMMENT_TEMPLATES.md) — standardized issue/PR comments

## Active queues

Active queues live under:

```text
specs/active/
```

The first active queue is:

* [Frontend/API Experimental Surface Hardening](../specs/active/frontend-api-hardening/QUEUE.md)

## Agent instructions and automation levels

Coding agents should read [AGENTS.md](../AGENTS.md) before modifying the repository.

GitHub automation levels are defined in [GitHub Automation Levels](../specs/loop-engineering/GITHUB_AUTOMATION_LEVELS.md).

Agent budgets are defined in [Agent Budgets](../specs/loop-engineering/AGENT_BUDGETS.md).

## See also

- [Product Surface Inventory](guides/product-surface-inventory.md)
- [Autonomous Loop Graduation Criteria](research/autonomous-loop-graduation-criteria.md)
