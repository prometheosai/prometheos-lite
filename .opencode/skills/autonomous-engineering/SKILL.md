---
name: autonomous-engineering
description: |
  Portable, repository-agnostic bounded engineering loop: plan, build in an
  isolated worktree, verify with fail-closed exact-revision evidence, review in
  a separate OpenCode context, repair at most twice, and prepare a PR. No hosted
  CI, paid service, deployment, merge, or product-specific path is required.
  Use when a repository needs independently verified, evidence-bound changes.
version: 2.0.0
user-invocable: true
argument-hint: "[discover|claim|status|advance|verify|review|recover|selftest]"
allowed-tools:
  - Bash(python3 .opencode/skills/autonomous-engineering/scripts/*)
  - Bash(python .opencode/skills/autonomous-engineering/scripts/*)
  - Bash(git worktree *)
  - Bash(git rev-parse *)
  - Bash(git status *)
---

# Autonomous engineering loop (v2)

A repository-local control loop that turns one bounded task into one
independently reviewed candidate. It does not merge, deploy, publish, or spend
money. Every claim is bound to an exact commit **and tree** and a reproducible
command.

## Roles

| Role | Context | May write | May approve |
|---|---|---|---|
| Planner | coordinator | contract, checkpoint | no |
| Builder | dedicated worktree | scoped implementation | no |
| Verifier | local runner | evidence file | no |
| Reviewer | separate OpenCode session/subagent | verdict record only | yes, at exact SHA/tree |
| Supervisor | coordinator scripts | checkpoint | no |
| Reporter | coordinator | status JSON/report | no |

Identifier checks are procedural guards. A reviewer identifier alone is not a
security boundary; use a genuinely separate context.

## State machine

```
DISCOVER → SELECT → CONTRACT → WORKTREE → IMPLEMENT → VERIFY
   → REVIEW → (REPAIR → REVERIFY → REVIEW)* → READY → SAVE_EVIDENCE → DONE
```

Illegal transitions, stale reviews, unverified checkpoints and exhausted repair
budgets are refused. `recover` resumes only from independently validated state.

## Commands

```bash
S=.opencode/skills/autonomous-engineering/scripts
python $S/supervisor.py discover --repo .
python $S/supervisor.py claim --repo . --contract task.json
python $S/checkpoint.py get --repo . --task-id TASK-001
python $S/verify.py run --repo . --contract task.json --output evidence.json
python $S/verify.py validate-evidence --repo . --output evidence.json
python $S/supervisor.py submit-review --repo . --task-id TASK-001 \
    --candidate <sha> --tree <tree> --evidence evidence.json
python $S/supervisor.py record-review --repo . --task-id TASK-001 \
    --candidate <sha> --tree <tree> --verdict APPROVE \
    --reviewer <id> --review-context <different-context>
python $S/supervisor.py recover --repo . --task-id TASK-001
python $S/supervisor.py selftest
```

Use `python3` where that is the platform interpreter. All commands are local;
none requires network, credentials, hosted CI or a hosted model service.

## Non-negotiable rules

1. Builder never approves its own work; review is a separate context.
2. A verdict is valid only for the exact SHA and tree it names. A new commit
   invalidates it.
3. Verification is fail-closed. An unrun check is `untested`, never `passed`.
4. Checkpoint is the only resume truth and is validated against the live repo.
5. Repair budget is two. There is no endless repair cycle.
6. No destructive command, deployment, privileged operation or external spend.
7. Respect AGENTS.md, security holds, protected branches and repository gates.

## Portability

No repository name, issue number, language, OS path, branch, model provider, test
framework or product vocabulary is hardcoded. Everything comes from CLI arguments
or the task contract. To adopt elsewhere, copy this directory and run
`python -m unittest discover -s tests -v` from the skill root.

## Honest limitations

No OS-level sandbox, no network fault injection, no true mid-write SIGKILL test,
and no measured throughput advantage without a comparable baseline. Do not claim
production readiness from self-tests alone.
