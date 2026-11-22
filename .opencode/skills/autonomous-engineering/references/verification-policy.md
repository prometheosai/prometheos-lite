# Verification policy

Verification is local, deterministic and fail-closed.

1. The command allowlist is `policies/approved-commands.json`. A denylist is not a
   sandbox and is not used. `verify.py` executes only commands present in the
   task contract AND approved by the policy. No shell is used.
2. Before a check runs, the worktree HEAD must equal the candidate SHA and its
   tree must equal the candidate tree. After all checks, both are checked again.
   Any drift invalidates the evidence and returns exit code 2.
3. Evidence stores only digests, status, exit code, duration and resource limits —
   never command output, which may contain private content.
4. `skipped` and `untested` are distinct from `passed` and count as not green.
   A check excluded by `--only` is `untested`; a check excluded by `--skip` needs a
   recorded reason. Neither makes a candidate READY.
5. Timeouts and output caps are enforced from the policy file, not from a
   configuration field that is never read.
6. Strong OS sandboxing (filesystem/network/resource restrictions) is NOT provided
   by this skill. Where the platform offers it, run the verifier inside an external
   sandbox and bind that fact into evidence. Without it, repository commands are
   trusted only after human inspection; do not claim a sandbox guarantee.
