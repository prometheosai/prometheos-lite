# Autonomous engineering Skill v3 — qualification evidence

Date: 2026-11-22. Harness: OpenCode v2.3.2. Python 3.14, Git 2.56.1 (Windows).
Host: 16 logical CPUs, 32 GB RAM. No new dependencies; stdlib only.

## Classification and scope

Repository classification: NEW. This is a tooling-only addition under
`.opencode/skills/`. It does not modify product code, promote the product's
autonomous execution loop, or change any graduation hold. It exceeds the
repository's default 5-file / 200-line PR budget; the universal dispatch is the
explicit scope for that exception. No merge or deployment is performed.

The DishMise PR #359 reference supplied the role/state-machine shape only. Its
`fcntl`-only lock, denylist-style guard, optional HEAD enforcement and
identifier-only reviewer check were replaced.

## Qualification results

`python -m unittest discover -s tests -v`: **45 tests, 0 failures** (exit 0),
also via `supervisor.py selftest`. `compileall` and JSON parse checks pass.

Gates:
- GATE 1 command authorization: negative controls for arbitrary Python, unsafe
  git options, unknown pattern, executable spoofing, shell metacharacters, and
  path traversal — PASS.
- GATE 2 exact-SHA integrity: wrong SHA, stale tree, dirty worktree, evidence
  from another worktree/repository — PASS.
- GATE 3 completeness: forged summary, all-skipped, zero-pass, duplicate id,
  unrecognized status, missing negative control, bad policy digest — PASS.
- GATE 4 reviewer isolation: builder self-approval, identical context, stale
  approval, reviewer-modified candidate, no evidence — PASS.
- GATE 5 recovery/idempotency: partial checkpoint, partial evidence, wrong SHA,
  missing worktree, repair-budget exhaustion, already-DONE no-op — PASS.
- GATE 6 risk-based evidence: state-machine model check, seeded property
  sequences, fault injection, one mutation control — PASS.
- GATE 7 benchmarking: local recorder emits machine-readable results; overall
  status INCONCLUSIVE (no comparable baseline, no model cost data).
- GATE 8 repository regression: no product or test file in the repository was
  modified by this change.

## Real pilot (disposable repository)

Base `566528c8bafe369d2fcae07c5772cb1a3e43b24d` /
tree `eb8c606caa0030c88180585a73e4ff6cd0c2e99f`; buggy `add()` with a failing
unittest (exit 1 baseline).

- Builder: separate `general` subagent, scoped to `calc.py`.
- Candidate `ec6a5279fb07ca07815f69c21dbcf4fbed559afa` /
  tree `4526aaad564ba7e851312eb2b67e8e4324566a21`.
- Verifier: 2 passed / 0 failed / 0 skipped / 0 untested; pre=post=candidate for
  HEAD and tree; clean worktree; contract and policy digests bound.
- Reviewer: separate read-only `explore` subagent, verdict `APPROVE`, independent
  HEAD/tree/scope/invariant/test/evidence checks at the exact candidate.
- Supervisor: READY → DONE via `complete` with the bound evidence package;
  `recover` returned `NO_OP_ALREADY_DONE`.
- Measured verification time: 0.87 s (`scripts/benchmark.py`), overall
  throughput INCONCLUSIVE. 0 repairs used.

## Known limitations

No OS-level sandbox; the command guard is an authorization policy, not
isolation. Reviewer independence is a procedural harness property, not a
security boundary. Evidence is locally generated and not cryptographically
signed. Full process-tree termination is POSIX-only; Windows records this in
evidence (`process_tree_termination_guaranteed=false`). No formal/symbolic proof
tool was available; bounded model checking is limited to explicit state
enumeration. Parallelism and comparative speedup remain unmeasured.

See `VERIFICATION.md` for requirement-to-evidence traceability, counterexamples,
assumptions and residual risks.
