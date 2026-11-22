# Autonomous engineering skill v2 — qualification evidence

Date: 2026-11-22. Harness: OpenCode v2.3.2. Runtime: Python 3.14, Git 2.56.1 (Windows).
Host: 16 logical CPUs, 32 GB RAM. No new dependencies; stdlib only.

## Classification

Repository classification: **NEW** (no prior autonomous-engineering skill; existing
Loop Engineering Protocol and `scripts/local_ci.py` retained untouched).

This is a **tooling-only** addition under `.opencode/skills/`. It does not modify
product code, does not promote the product's autonomous execution loop, and does
not change the repository's graduation holds. Note: 13 files / ~800 lines exceed
the repository's default 5-file / 200-line PR budget; the universal dispatch is
the explicit scope for this exception. No merge is performed by this work.

## Reused capabilities

- Repository-owned `scripts/local_ci.py` and Rust/frontend baselines stay authoritative.
- Existing git worktree support, issue/PR policy and AGENTS.md rules are respected.
- The DishMise PR #359 reference supplied the role/state-machine shape only; its
  `fcntl`-only lock, denylist-style command guard, optional HEAD enforcement and
  identifier-only reviewer check were replaced.

## Repairs over the reference

1. Cross-platform atomic claim lock (`msvcrt` / `fcntl`), not POSIX-only.
2. Command **allowlist** from `policies/approved-commands.json`; no denylist, no
   shell. Only contract-declared commands that are also policy-approved run.
3. Exact commit **and tree** binding before and after verification; drift
   invalidates evidence. The reference had optional HEAD enforcement and no
   post-run check.
4. Review requires SHA, tree, reviewer id **and a different review context**;
   reviewer also re-checks the live worktree HEAD/tree. This is a procedural
   guard, not a security boundary.
5. Recovery checks worktree existence, branch, HEAD and tree before resuming.
6. Resource limits (count, timeout, output bytes) are read from the policy file
   and recorded in evidence; zero external spend is never claimed as free model
   inference.

## Qualification results

`python -m unittest discover -s tests -v`: **19 tests, 0 failures** (exit 0), also
via `supervisor.py selftest` (exit 0). `compileall` and JSON parse checks: exit 0.

Negative controls include: unknown checkpoint field, malformed SHA, duplicate
claim with a different owner, illegal state transition, already-DONE no-op,
builder as reviewer, identical review context, stale review after a new commit,
repair-budget exhaustion, wrong-SHA recovery, missing worktree recovery, partial
checkpoint JSON, unapproved command rejection, missing worktree in contract,
and untested check not being green.

## Real pilot (disposable repository, not production)

Task: fix `add(a, b)` returning `a - b` in a purpose-built two-file git repo with
a failing unittest. Base `1889867f447899ee3c302b7a5ba0d00a44363622` /
tree `3377d8a2e59bb71d3b2dfdca01568f399e06124c`.

- Builder: separate `general` subagent context, scoped to `calc.py` only.
- Candidate: `f4bbb2d60ac6756aebd6e8e8abfd8f052d0224f3` /
  tree `29f2c7ea4cbb61d53aa272799f933c3d53c086bf`.
- Verifier: 2 passed / 0 failed / 0 skipped / 0 untested; pre=post=candidate for
  both HEAD and tree; policy digest `1f169bc9…53fa3f0`.
- Reviewer: separate read-only `explore` subagent context. Verdict `APPROVE`,
  with independent HEAD/tree confirmation, scope check and test re-run.
- Supervisor: DONE; `recover` afterwards returned `NO_OP_ALREADY_DONE`.
- No repair was needed (0 / 2 attempts).

The pilot repository and all checkpoints/evidence live outside the production
repository. Nothing was merged or deployed.

## Parallelism and benchmark

- Parallelism: **NOT TESTED** — one safe disposable task existed. No concurrency
  result is claimed.
- Throughput: **INCONCLUSIVE** — no measured baseline for the existing workflow
  under comparable tasks exists. No percentage improvement is claimed.
- External cost: no paid service was used; model inference cost was not measured
  and is not claimed to be zero.

## Known limitations

No OS-level sandbox, no network fault injection, no true SIGKILL-mid-write test,
no measured throughput advantage, and identifier/context checks are procedural.
The reviewer's independence is a harness property, not a cryptographic boundary.
Production readiness must not be inferred from these self-tests.
