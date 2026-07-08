# Handoff Template

Use this template when handing off between agents or between tasks.

```markdown
# Handoff

## Current state

<!-- Brief description of the current state of work. -->

## Completed work

<!-- What was implemented, changed, or delivered. -->

## Verification run

<!-- Exact commands run and their exit codes / key output. -->

## What was not run

<!-- Verification steps that were intentionally or unavoidably skipped. -->

## Blockers

<!-- Any blockers encountered. None if no blockers. -->

## Risks

<!-- Any risks, concerns, or areas that need human attention. -->

## Next task

<!-- The next task in the queue, if applicable. -->

## Stop reason

<!-- Why execution stopped: queue complete, verification failed, blocker, scope expansion, etc. -->

## Confidence

<!--
High / Medium / Low

High: all checks pass, scope is contained, docs accurate.
Medium: minor concern (e.g., partial verification, manual demo not run).
Low: significant concern that needs human review before proceeding.
-->
```
