# Handoff

_Last updated: October 7, 2026, after PR #241 (E4/X07 Slice 2 — graph projection with nested composite disclosure boundaries) merged as `d71cf0d`; prior authority wave #237/#236/#233 recorded on `b1120a6`._

## Authority state

- `main` = `d71cf0d849695100b65225a62ad0a825cd595c2b` — the MERGE COMMIT that merged PR #241. Its tree is byte-identical to the reviewed head `1af765d` (verified: `git diff d71cf0d..1af765d` is empty), so the reviewed head's tree is the authoritative content; PR #241's 9-commit history is preserved behind it (no squash).
- **Post-merge verification, actually run on checked-out merged main `d71cf0d`** (not merely projected): `cargo test --lib` 1082 passed / 0 failed (1 ignored); Slice-2 projection conformance 64/64 passed; working tree clean.
- Tally: **29 issues closed · 58 total merges · 55 independently approved** (carried state was 29/57/54 at `b1120a6`; +1 independently-reviewed merge = PR #241; the intermediate docs-state PR #238 is included in the carried 57).

## PR #241 evidence record (authoritative)

Reviewed head `1af765d` (full: `1af765db6e7e341b6293267fc7412fd9440a71b0`). Exact-head final-pass gates: G1/G2/G4 passed on their first attempts; G3 reached its authoritative green on attempt 4 at this head (attempt 3 hit the pre-existing Windows `cpu_breach` race disclosed below; attempts 1-2 hit documented disk/host failures and wrote no evidence) with 105 suite groups / 2207 passed / 0 failed; G4 doc tests exit 0. Slice-2 conformance suite 64/64 (includes the item-17 verify-half). local_ci evidence digests at that head: core `0b6eb3d656f203d453aff9f8e95158771d966fefd6bcee4a19253e86b8079671` · platform `1f06395c1563851ae96087b94da1a1f82e2abfd835e894221a9718e2604fdf94` · smoke `889c39eb22d5123f89b76776a4b4e9f6a14d8c78d64854d800b3cda96199a8ad` · 3-file verify PASS. Reconvergence: merge of `b1120a6b` into the branch with zero conflicts; upstream's 22 changed files disjoint from Slice 2's 13; no Slice-2 path altered. One disclosed pre-existing host issue surfaced during verification: Windows job-time-limit vs 100 ms monitor-poll race in `cpu_breach_via_orchestrator_carries_typed_durable_evidence` (main-scoped; no worktree change in that path) — documented in the PR body's honest-history entry; not fixed in this PR by design. Merge commit: `d71cf0d849695100b65225a62ad0a825cd595c2b` on 2026-10-07.

## PR #237 evidence record (authoritative)

Reviewed head `dc6ac7f` (full: `dc6ac7f2cd3ff465b1fdc3e7ece3d34be43f9af8`). Exact single-pass evidence digests recorded AT that head: core 11/11 `ca7349ffe36c93e4325b815ed88ad197f61aa1a90e2b1b7ceaebffc669aa1d05`, platform 8/8 `d38b33259a440bf9add73896083036039ab7850bdba738f7ec83c3d67dbe5c15`, smoke 9/9 `981c40ab954542ab136cd12e42dbce008482fa4fd0a069c57b1123347ee53694` — 3-file verify PASS. Scope: +1,515/−27, accepted. #214 host-flake disclosure: markedly worse AV-vs-`.git/objects` interference this session — rotating git-fixture victims across attempts, each green in isolation; all published evidence is single-pass at the exact heads.

## SOMA projection + observation contract (authoritative after Slice 2)

- **Projections** (Slice 1B): verified-records-only mapping to SPEC 006 `WorkEvent`s; typed `RunKey` identities (equal strings never merge); per-run causally-closed batches with ancestor inclusion; raw stored-byte source digest (`data`/`createdAt` bind as the raw SQLite TEXT); the read gate binds semantics, the projection binds bytes.
- **Observation endpoints** (Slice 2, experimental router): `GET …/work-events` (reconnectable stream segments; `X-Next-Cursor` ALWAYS present, `X-More-Available` separate), `GET …/work-event-runs` (typed enumeration), `GET …/work-event-runs/:kind/:run_id` (run batches).
- **Transport validators**: bodies ARE canonical envelope bytes; stream ETags bind **body + continuation cursor + availability** (a metadata flip invalidates the cached representation — stale exhaustion cannot survive revalidation); stream 304s carry both paging headers; `If-None-Match` supports lists, weak tags, `*`; non-paged resources bind the body alone.
- **Error mapping**: 400 (unknown kind lists kinds) → 403/404 ownership → 500 gate failures (gate text verbatim) → 422 unprojectable records (`ApiError::Unprocessable`) → 404 unrecorded run keys (`ProjectionError::UnknownRun`) → 500 audit-gated batches. No partial artifacts on any path.

## Provenance enforcement contract (authoritative after #232/#233, refined by Slice 1B)

Write boundary: canonical-bytes fixpoint, never-widen invariants, complete source digest; `internal_system` records `Harness` (absent principal is the honest distinction; `System` is parse-only). Trigger `provenance-trigger-v3` with `PRAGMA schema_version` untouched-proof. Read verification: source digest + column drift + mixed-state refusals; legacy → `LegacyUnverified`. Cancellation: exact-event-id → token payload (SeqCst, payload-before-flag); durable lookup is cross-process fallback only. `detect_repo_binding`: Bound/Dirty/Unbound, fails closed.

## Open follow-ups (operator-approved order)

1. **#132 Slice 3** — SPEC 007 capability negotiation (`CompatibilityDecision` read/control projections; capability metadata is never authority). Plan review before implementation; plan to be posted to #132.
2. **#217** — Dependabot bump: requires explicit operator approval.
3. **Change records** for Slice 2 posted at `specs/loop-engineering/changes/2026-10-04-e6i03-slice2-observation-endpoint.md` (this PR).
4. **E4/X07 Slice 3 candidate** (post-#241, issue #164): review-report and evidence-timeline projections remain the undelivered required-stable projections in #164's issue text (Slice 1 shipped canonical/envelope/human-plan, Slice 2 shipped the graph data model). No governing spec beyond the parent issue currently exists; next step is a Slice 3 SPEC (rev) drafted against issue #164 + the Slice 2 spec, put through plan review, before any implementation PR — i.e., that node is a SPEC/PLAN node, not an implementer node. #164 itself is CLOSED; a child-issue will need to be opened to host the node.

## Verification baseline

- fmt/clippy: clean at `1af765d` per core evidence; covers merged main `d71cf0d`.
- `cargo test --lib`: 1083 discovered (1082 passed, 0 failed, 1 ignored); plus 64 Slice-2 graph-projection conformance tests, 24 API observation tests, 23 Slice 1B conformance tests, 36 enforcement tests. Full `--all-targets --all-features` sweep at `1af765d`: 105 groups, 2207 passed, 0 failed.
- Any new PR: three local suites at the exact head, digests recorded, independent fresh-context review before merge authorization; immutable expected-head guards at merge time. No merge without a fresh local-evidence review cycle.
- Host note: builds/temp routed to D:; C: near capacity; the #214 Norton-AV-vs-`.git/objects` flake rotates victims under parallel load (severe this session — cooldowns and isolated victim reruns are the documented protocol).
