# Risk-based, proof-driven verification record (Skill v3)

This document records verification decisions for the Skill itself, not for any
product task. It is local, stdlib-only and introduces no paid or external
infrastructure. Tests are tests: no mathematical proof is claimed.

## Functional requirements and traceability

| Req | Invariant | Implementation | Test / proof | Evidence | Review |
|---|---|---|---|---|---|
| FR-1 select/bound one task | single owner, dependency readiness | `supervisor.py claim`, atomic lock, `checkpoint.py` closed schema | `test_claim_binds_contract_digest`, `test_duplicate_claim_different_owner_refused`, `test_repair_budget_enforcement` (claim/state guards) | checkpoint JSON with contract digest | independent Skill review at exact SHA |
| FR-2 build in isolated worktree | worktree belongs to repo, distinct path | `verify.py` git-common-dir identity | `test_missing_worktree_is_refused`, pilot worktree | evidence carries worktree identity via git checks | pilot reviewer |
| FR-3 fail-closed verification | exact HEAD+tree pre/post, clean tree, no skipped/untested, passed>0, mandatory checks present | `verify.py validate_evidence`, `run`, `cm.worktree_clean` | `test_forged_evidence_refused`, `test_all_skipped_not_green`, `test_no_pass_not_green`, `test_bad_policy_digest_refused`, `test_dirty_worktree_invalidates_evidence` | recomputed `summary`, `green=true`, digests | independent Skill review |
| FR-4 independent review | separate context/id, live SHA+tree re-check | `record-review`, `check_live_revision` | `test_builder_cannot_review`, `test_review_context_must_differ`, `test_stale_review_after_new_commit_refused` | review_result bound to SHA+tree | pilot reviewer (separate subagent) |
| FR-5 bounded repair | max 2 attempts | `record-review` REPAIR_REQUIRED, `repair` | `test_repair_budget_enforcement` | checkpoint attempt_count / BLOCKED | independent Skill review |
| FR-6 completion authority | only bound evidence + APPROVE enter READY/DONE | protected transitions, `submit-review`, `complete`, restricted `checkpoint.patch` | `test_advance_bypass_to_ready_refused`, `test_advance_bypass_to_done_refused`, `test_checkpoint_state_forgery_refused`, `test_complete_without_approval_refused` | state machine + evidence digest | independent Skill review |
| FR-7 bounded execution | output capped while running; timeout terminates | `cm.run_bounded` streaming readers + termination | `test_bounded_output_terminates_while_running`, `test_fault_injection_timeout_terminates` | `termination` and `capped` fields | independent Skill review |
| FR-8 command authorization | exact reviewed argv templates, trusted executable path | `verify.py authorize`, policy v2 | `test_arbitrary_python_execution_rejected`, `test_unsafe_git_options_rejected`, `test_unknown_pattern_rejected`, `test_executable_spoofing_rejected` | policy digest in evidence | independent Skill review |

## Safety / security invariants

- A builder cannot approve its own work; a verdict is valid only for the exact
  commit and tree it names.
- Contract-supplied argv never grants authority; only reviewed templates do.
- No skipped, untested, forged, stale or dirty evidence is green.
- No state, candidate, tree, owner, review or evidence field is publicly patchable.
- No unattended merge, deployment, paid service or privilege escalation.

## Forbidden states

- `DONE`, `READY`, `SAVE_EVIDENCE` or `REVIEW` reached via generic `advance`.
- `READY` without an independent APPROVE bound to candidate and tree.
- `DONE` without a bound, revalidated evidence package.
- Green evidence with zero passed checks, all skipped, untested, corrupt summary,
  stale HEAD/tree, dirty worktree, or a policy digest that does not match.

## Preconditions / postconditions

- `submit-review` pre: state VERIFY, clean live worktree at candidate+tree, green
  evidence bound to task/contract/policy. Post: state REVIEW, evidence digest stored.
- `record-review` pre: state REVIEW, live revision unchanged, reviewer and context
  differ from builder. Post: READY / REPAIR / BLOCKED with bound verdict.
- `complete` pre: state READY, APPROVE bound to candidate+tree, same evidence file
  digest, green evidence, clean live worktree. Post: DONE.
- `recover` pre: closed-schema checkpoint, resolvable SHAs, existing clean worktree
  at the exact candidate+tree, and (for completion states) bound approval/evidence.
  Post: validated resume action or `NO_OP_ALREADY_DONE`.

## Concurrency and failure scenarios

- Two claimants: atomic lock allows one claim; the other gets RECLAIMED or refusal.
- Interrupted write: closed-schema validation plus SHAs; no silent resume.
- Wrong SHA / stale tree / new commit: refused at review and recovery.
- Dirty index or untracked file: verification and completion refuse.
- Timeout / output flood: process terminated; evidence records the reason.
- Windows process-tree kill cannot be guaranteed: recorded in evidence and policy
  (`process_tree_termination_guaranteed=false`); POSIX kills the process group.

## Method selection by risk

| Method | Applied to | Result |
|---|---|---|
| Unit/integration tests | normal control flow, guards | executed |
| Property-based (seeded sequences) | random public command sequences cannot forge DONE | executed, fixed seed 20261122 |
| Fault injection | timeout, output flood, wrong SHA, stale tree, dirty tree, partial JSON | executed |
| Model checking (explicit state enumeration) | protected gates unreachable via generic advance | executed |
| Mutation testing | flip protected-gate guard and confirm the model-check detects it | executed (one targeted mutant) |
| Bounded symbolic / formal proof | critical state-machine properties | NOT RUN — no suitable local tool installed; no proof claimed |
| End-to-end simulation | disposable-repository single-worker pilot | executed |

## Counterexamples found and repaired during qualification

1. `advance` could move VERIFY→READY→DONE without evidence or review (P0 bypass).
2. `checkpoint.py patch` could rewrite state/candidate/review fields (P0 forgery).
3. All-skipped / zero-pass evidence was treated as green.
4. `capture_output=True` collected unbounded subprocess output before the size check.
5. Executable authorization checked only the name, permitting argv and PATH spoofing.
6. Worktree identity used a path-prefix check that rejected legitimate sibling worktrees.
7. Output-cap termination waited for process exit and could deadlock on a full pipe.
8. Generic `advance REVIEW→REPAIR` bypassed `attempt_count` and `MAX_REPAIRS`,
   allowing unlimited repair cycles (independent review finding R-1).
9. Required negative-control ids were read from the evidence file itself rather
   than the authoritative policy (N-1).
10. `checkpoint init` could overwrite an existing checkpoint and drop an APPROVE
    binding (N-2).
11. The verifier validated an absolute executable path but launched the original
    bare name, permitting a second PATH lookup to select a different binary.
    The verified absolute path is now substituted into Popen and the binary
    digest is pinned in reviewed policy. A pathname alone is not claimed to
    prove immutability.

Each counterexample has a corresponding regression test in `tests/test_loop.py`.

## Assumptions, limitations, residual risk

- Checkpoint files are trusted local state; they are schema-validated but not
  cryptographically signed. Direct filesystem tampering by a hostile local user
  is outside the threat model.
- Evidence is locally generated; integrity derives from recomputed counts, exact
  bindings and the current policy digest, not a signature.
- No OS-level sandbox; the command guard is an authorization policy, not isolation.
- Reviewer independence is a harness property (separate OpenCode context), not a
  cryptographic or OS security boundary.
- No mathematical proof; formal/symbolic methods were not available locally.
- Parallelism and comparative throughput remain unmeasured.

## Overhead and defects caught

- Qualification suite: 49 tests, ~220 s wall clock on this Windows host.
- Defects caught by qualification before publication: 7 (listed above), all fixed.
- Measured verification overhead for the final pilot: 0.87 s for two bounded
  commands (unit test and diff check), recorded by `scripts/benchmark.py` as
  `4137.931 accepted_tasks_per_verification_hour`. This is verification-only;
  overall development throughput remains INCONCLUSIVE because no comparable
  baseline was measured. No external spend.
