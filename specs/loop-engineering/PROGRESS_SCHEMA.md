# Progress Schema

Progress tracking format for loop-engineering tasks.

```markdown
# Loop Progress

## Mode

<!-- Task Mode or Epic Completion Mode. -->

## Approved scope

<!-- Link or description of the approved epic or task. -->

## Current queue

- [ ] Task 1 <!-- Brief description -->
- [ ] Task 2 <!-- Brief description -->
- [ ] Task 3 <!-- Brief description -->

## Completed tasks

| Task | Commit | Files | Verification | Notes |
|---|---|---|---|---|
| <!-- Task name --> | <!-- Commit hash --> | <!-- Files changed --> | <!-- Verif. results --> | <!-- Notes --> |

## Current task

<!-- What is being worked on now. -->

## Blockers

<!-- Any blockers. None if no blockers. -->

## Verification evidence

<!-- Summary of verification results for the current or most recent task. -->

## Stop / continue decision

<!--
For Epic Completion Mode:
- Continue to next task
- Stop: queue complete
- Stop: verification failed
- Stop: blocker appeared
- Stop: scope expansion detected
- Stop: high-risk escalation
- Stop: human review gate reached
-->

## Next recommended action

<!-- What the next agent or human should do. -->
```

## Usage

- Create this file at the start of an Epic Completion Mode run.
- Update after each completed task.
- Include in the handoff report when stopping.
- Archive to `handoff.md` at the repository root when the epic is complete.
