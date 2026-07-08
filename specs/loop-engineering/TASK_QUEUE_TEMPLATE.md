# Task Queue Template

Use this template to define an approved queue of bounded tasks for Epic Completion Mode.

```markdown
# Epic Queue

## Epic

<!-- Short name for the epic. -->

## Mode

Epic Completion Mode

## Approved scope

<!-- Description of the approved scope boundary. -->

## Queue

1. **Task 1** — <!-- Brief description -->
   - Scope: <!-- What is included -->
   - Boundaries: <!-- What is excluded -->
   - Verification: <!-- Which verification bundle to run -->

2. **Task 2** — <!-- Brief description -->
   - Scope:
   - Boundaries:
   - Verification:

3. **Task 3** — <!-- Brief description -->
   - Scope:
   - Boundaries:
   - Verification:

## Stop conditions

- Stop if any hard blocker from SAFETY_GATES.md appears.
- Stop if scope expands beyond the approved queue.
- Stop if a task verification fails.
- Stop if a dependency change is needed.
- Stop if public API or governance changes are needed.
- Stop at human review gate.

## Human approval

<!-- Required before merge. -->
```
