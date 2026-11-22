# Independent review policy

The reviewer must be a separate OpenCode context (a different session or a
subagent), not a role label inside the builder's context.

Provide the reviewer with: the task contract, exact candidate SHA and tree, the
diff and relevant source, independent verification evidence, and applicable
security/architecture contracts. Do not provide the builder's final conclusion as
proof. The reviewer returns one of `APPROVE`, `REPAIR_REQUIRED`, `BLOCKED` plus
findings, bound to the exact SHA and tree.

`record-review` refuses a verdict when the reviewer equals the owner or the review
context equals the builder context. These checks are procedural guards, not a
security boundary: a genuinely independent context must actually be used.

Any new commit invalidates the verdict because both SHA and tree are rebound.
Repair is capped at two attempts; exhausted repair budget becomes BLOCKED.
Generic `advance` cannot move a task into REVIEW, READY or DONE: only
`submit-review` (bound evidence), `record-review` (independent APPROVE) and
`complete` (same approved evidence package) can cross those gates. The public
checkpoint `patch` command cannot write state, candidate, tree, owner or review
fields.
