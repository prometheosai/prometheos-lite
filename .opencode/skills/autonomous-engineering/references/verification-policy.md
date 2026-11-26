# Verification policy

Verification is local, deterministic and fail-closed.

1. Authorization is `policies/approved-commands.json`: exact, reviewed argument
   templates plus trusted executable resolution. A contract cannot invent
   authority by supplying argv. A denylist is not a sandbox and is not used;
   no shell is used, and no filesystem/network/privilege isolation is claimed.
2. Before a check runs, the worktree HEAD must equal the candidate SHA and its
   tree must equal the candidate tree. After all checks, both are checked again.
   Any drift invalidates the evidence and returns exit code 2.
3. Evidence stores only digests, status, exit code, duration and resource limits —
   never command output, which may contain private content.
4. `skipped` and `untested` are distinct from `passed` and count as not green.
   A check excluded by `--only` is `untested`; a check excluded by `--skip` needs a
   recorded reason. Neither makes a candidate READY.
5. Output is bounded WHILE a command runs by streaming readers with a byte cap;
   the process is terminated on cap or timeout. Full process-tree termination is
   guaranteed only on POSIX; on Windows the evidence records this limitation.
6. Evidence validation recomputes result counts from results, requires passed>0,
   rejects skipped-only/untested/failed runs, requires every mandatory check to
   be present and passed, validates the current policy digest, and requires a
   clean worktree before and after.
7. Strong OS sandboxing (filesystem/network/resource restrictions) is NOT provided
   by this skill. Do not claim a sandbox guarantee.
