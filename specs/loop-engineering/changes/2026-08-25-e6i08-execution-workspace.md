# Change: E6/I08 — governed ExecutionWorkspace + GitWorktree adapter

**Issue:** #171
**Depends on:** #115 ✅, #118 ✅, #151 ✅.

## Scope

Versioned Rust-native `ExecutionWorkspace` / `WorkspaceAdapter` seam so a harness is never synonymous with a filesystem/worktree:

- **`WorkspaceManifestV1` (`lite.workspace.v1`)** — workspace identity (NOT agent identity), adapter kind/revision, repo identity + pinned base revision, isolation mode, declared writable scopes, resource-lock identity (#124-ready), canonical digest; fail-closed parse (version gate, structural checks, digest verify).
- **`WorkspaceRefV1`** — portable reference for PortableWorkState/checkpoints/evidence: durable identity only, no process state; serde round-trip tested.
- **`WorkspaceAdapter` trait** — acquire / validate / cleanup(preserve→evidence) / crash-recover.
- **`GitWorktreeWorkspace`**: deterministic isolated writable workspaces via `git worktree add --detach` pinned to the exact requested revision; teardown unconditional only AFTER referenced artifacts are preserved out.
- **`ExistingReadOnlyWorkspace`**: inspection/review mode over an existing checkout; never deletes; write authority structurally denied (`ensure_writable` refuses).

## Invariants enforced (fail closed)

- Writable acquisition has NO fallback path to the source checkout; failed acquisitions leave no directory residue.
- Resume rejects missing/stale/schema-incompatible references with typed errors; proceeding requires an explicit evidenced `RemapAuthorization` (reason + authorizer + timestamp carried in the recovery outcome).
- Adapter mismatch (worktree ref presented to read-only adapter and vice versa) rejected, not silently adapted.
- Cleanup preserves referenced proposals/checkpoints/evidence/journal artifacts before teardown; the user's checkout is never touched by cleanup.
- Resource-lock identity exposed for later parallel scheduling (#124).

## Non-goals honored

No scheduler, remote workers, containers/K8s, visual manager, or multi-agent fan-out. Current validation-isolation security controls untouched (orchestrator resource layer unchanged).

## Out of scope (follow-ups)

- Harness-selection integration through #154/#133 (consumes declared capabilities next).
- Container/remote adapters (seam supports them without forcing them here).

## Verification plan

`cargo test --lib workflow::workspace` — 8 tests: manifest seal/verify/tamper, worktree create+teardown+isolation+preservation, stale rejection, read-only denial + non-destructive cleanup, recovery matrix (missing/stale/remap/resume), adapter mismatch both directions, ref round-trip w/o process state, no-fallback-on-failed-acquisition. fmt/clippy -D warnings/full lib. No dependency changes.
