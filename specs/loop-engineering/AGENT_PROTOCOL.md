# Agent Protocol

Defines how agents execute work under the PrometheOS Lite Loop Engineering Protocol.

## Sources of truth (in order)

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

When sources disagree, stop and report the conflict. Do not invent intent from chat memory.

## Operating modes

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

```
INTAKE
→ Read approved scope and queue from GitHub issue, PR, or human prompt.
→ Confirm mode (Task Mode or Epic Completion Mode).
→ If no queue file exists, create one using TASK_QUEUE_TEMPLATE.md.

CONTEXT_SYNC
→ Read all source-of-truth documents.
→ Read current progress file if continuing.
→ Check CI status on current branch.
→ Resolve any source-of-truth conflicts or stop.

PLAN
→ For the current bounded task, produce a short plan.
→ Confirm the plan does not exceed approved scope.

BRANCH
→ Create a branch from the latest main.
→ Branch naming: `{type}/{description}` (e.g., `docs/loop-engineering-protocol`, `feat/frontend-lint-ci`).

EXECUTE_TASK
→ Implement the bounded task.
→ Do not make changes outside the approved scope.
→ Do not add dependencies without escalation.
→ Do not refactor unrelated code.
→ Do not change stable alpha behavior.

VERIFY
→ Run the relevant verification bundle from docs/LOOP_ENGINEERING.md.
→ Record exact verification output.
→ Do not claim checks that were not run.
→ Do not skip or narrow tests only to pass.

COMMIT
→ Commit atomically: one commit per bounded task where possible.
→ Write a clear commit message describing what changed and why.
→ Do not commit secrets, credentials, or API keys.

REPORT
→ Update progress file with completed task, commit hash, files changed, verification results, and notes.
→ If using Epic Completion Mode, update the queue.

CONTINUE_OR_STOP
→ If Task Mode: stop, create PR, write handoff.
→ If Epic Completion Mode:
  → If queue is complete: stop, create final PR or epic PR, write handoff.
  → If stop condition appears: stop, create PR for completed tasks, write handoff with blocker description.
  → Otherwise: continue to next task (go to CONTEXT_SYNC).

FINALIZE_PR
→ Use PR_TEMPLATE.md.
→ Fill in all sections accurately.
→ Do not claim unrun checks passed.
→ List known limitations and follow-ups.

HUMAN_REVIEW
→ Leave PR open for human review.
→ Do not merge.
→ Respond to review requests.

MERGE_ONLY_WITH_APPROVAL
→ Do not merge to main without explicit human approval.
