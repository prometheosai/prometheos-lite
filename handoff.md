# Handoff

_Last updated: 2026-09-11 (post-#214 merge). Previous handoff documented a
stale OpenRouter/identity session and has been replaced with the current
ground truth._

## Authority state

- `main` = `311f7c8` — squash-merge of PR #214 (E6/I03 Slice B), landed on
  top of `5ddbe01` (#216, SOMA canonical fail-closed number policy).
- Tally: **23 issues closed · 43 total merges · 40 independently approved
  merges.**
- Exact reviewed head of #214 was `d4f07b7`; all 13 CI jobs green.
- Local merged branches pruned: `feat/e6i03-event-cursor-api`,
  `feat/soma-canonical-fail-closed-numbers` (both verified content-equal to
  `main` before deletion; squash merges defeat ancestry checks, so `-D` was
  used after verification).

## Recently merged (slice train)

| PR | Slice | Head of land |
|----|-------|--------------|
| #209 | E6/I01 Slice B — AppConfig versioning | merged |
| #210 | E6/I01 Slice C — six workflow templates | merged |
| #211 | E6/I01 Slice A — CLI contract tests | merged |
| #212 | E6/I02 Slice A — read-only run inspector | merged |
| #213 | E6/I03 Slice A — API read-model rebuild tests | merged |
| #216 | SOMA canonical fail-closed number policy | merged |
| #214 | E6/I03 Slice B — durable cursorable event stream | `311f7c8` |

## Open work items

### #132 (E6/I03) — remains OPEN after #214

GitHub auto-closed it on the #214 mention; operator reopened it and
documented the correction. Remaining acceptance items:

- Headless **cancel/decide** operations through the runtime control
  boundary (next slice candidate — resume after #215 resolves).
- Versioned, fail-closed `WorkEvent` mapping to SOMA++ SPEC 006.
- Multi-client observation semantics (multiple authorized clients
  observing the same run without becoming state authorities).
- Capability/compatibility projections mapped to SOMA #83.

### #215 — portable_state canonical-number parity (ACTIVE)

Investigation/decision phase. `src/workflow/portable_state.rs` implements
a second canonical-number path (`to_canonical_json` / `state_digest` via
`serde_json::to_string`) not checked against `src/workflow/soma/canonical.rs`.
Deliverable for the current phase: an investigation/decision report — not
code changes. Hard constraints:

- Do not silently change persisted digest compatibility.
- `serde_json/arbitrary_precision` stays gated until product need and
  compatibility consequences are documented.

### #217 — Dependabot `postcss-selector-parser` bump

Untouched. Per AGENTS.md, dependency changes require explicit operator
approval. No action until the operator explicitly authorizes.

## Known environment caveat

Full-parallel `cargo test --lib` on the Windows host intermittently fails
git temp-worktree tests (`failed to insert into database` on `git add`)
with a varying failure set; every affected test passes in isolation and
serialized (`--test-threads=1`) runs are fully green (1003/1003). Host
filesystem/AV contention, predates #214; cross-platform CI is the
authoritative gate. See PR #214 body for the documented note.

## Failed attempts (command hygiene)

- Do not use `cd /d`, `&&`, `head`, `tail`, `xargs`, `cat << 'EOF'` —
  Windows PowerShell 5.1. Use `Set-Location -LiteralPath`,
  `; if ($?) {}` chains, `Select-Object -First/-Last`, and the Write tool
  for files.
- Always use `git <...>` with `; echo "EXIT:$?"` to detect native-command
  exit failures (PowerShell native-command stderr surfaces as
  RemoteException noise; `$?` on the pipeline, not `$LASTEXITCODE` of the
  pipeline, is unreliable here — check the tool's stderr text instead).

## Next actions (operator-approved)

1. This PR (handoff refresh).
2. #215 investigation → decision report as issue comment.
3. After #215 resolves: resume #132 with the cancel/decide slice.
