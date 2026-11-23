# Recovery policy

`recover` re-reads the checkpoint, validates its closed schema, confirms the base
and candidate commits and tree exist in the repository, confirms the worktree is
a real git worktree on the recorded branch, and confirms HEAD and tree still equal
the checkpoint. Only then does it print a `RESUME:` action.

It is deliberately a no-op for `DONE` tasks, refuses stale checkpoints, refuses a
missing worktree, and refuses partial or malformed checkpoint/evidence files.
A task claimed by another owner is refused by the atomic claim lock.

Tested recovery cases: interrupted execution (RECLAIMED), already-completed task,
wrong SHA, stale tree, missing worktree, duplicate claim, partial JSON, and
exhausted repair budget. A real SIGKILL during a write, full network outage, and
OS-level sandbox escape are NOT TESTED and are not claimed.
