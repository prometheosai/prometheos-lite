# Task contract

Every task is one JSON file following `schemas/task.schema.json`. It must declare:

- `task_id` — unique, `[A-Za-z0-9._-]+`.
- `issue` — issue number, or `0` for a tooling/qualification task.
- `base_sha` and `tree_sha` — exact base commit and tree of the target repository.
- `worktree`, `branch` — dedicated isolated workspace for this task only.
- `owner` and `builder_context` — builder identity and the OpenCode context/session id.
- `allowed_scope` — path globs the builder may change. Anything outside is a violation.
- `acceptance` — measurable conditions; no acceptance may be "looks fine".
- `invariants` — security/product constraints that must not change.
- `dependencies` — tasks/issues that must be merged first; unmerged blocks selection.
- `resource_budget` and `repair_limit` — declared limits, enforced by the supervisor.
- `commands` — the complete list of verification commands. This list is the allowlist
  of checks that will run; no other command is executed.

The contract is untrusted data. The builder does not approve it; the coordinator
validates it against repository policy before `claim`.
