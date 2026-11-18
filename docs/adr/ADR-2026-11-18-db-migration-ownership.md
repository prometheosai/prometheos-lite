# ADR - Database migration ownership and the schema lifecycle boundary

- **Status:** Accepted (2026-11-18). Implementation: PR #321 (`hardening/slice-b-migrations`, commits `6afe429`, `241c2a9`, `ad9291e`), hardened against review findings (atomic v2 under a runner-managed transaction with FK suspend/restore on every path, history/target-version rejection, never-lowered `user_version`, post-migration FK verification).
- **Context:** `ARCHITECTURE_HARDENING_001` dispatch, findings A-001..A-007; A-004 recorded "no clearly centralized versioned SQLite migration lifecycle". This ADR records the migration ownership decision and the dependency-direction boundaries accepted by the hardening pass. It does not settle runtime behavior or release posture.

## Context and problem statement

`Db::init_schema` was the single place that created and upgraded the SQLite schema: 23 baseline `CREATE TABLE IF NOT EXISTS`, one transactional copy-and-rename for `work_context_events` (the `seq` cursor), five additive provenance columns, and ~12 later compatibility columns applied with errors swallowed (`.ok()` / `let _ =`). There was no `schema_migrations` table, no `PRAGMA user_version`, and no ordered registry, so no database could report which schema revision it had applied, and a partially applied upgrade could not be distinguished from a converged one.

## Decision

1. **The `db` module owns the schema lifecycle exclusively.** Every schema change ships as an ordered, named, idempotent `Migration` step in `src/db/repository/migrations.rs`. No other module issues DDL.
2. Applied migrations are recorded in `schema_migrations(version, name, applied_at)` with the DDL and its record committing atomically in one transaction — including v2, whose FK-pragma suspension runs in the runner before the transaction (SQLite ignores the pragma inside one) and restores on every path; retry hygiene (dropping a straggler `work_context_events_new`) persists in a pre-transaction phase. Failures roll back to the prior recorded revision and are retryable.
3. Before any DDL runs, the runner validates recorded history: a database whose `PRAGMA user_version` is newer than the binary's revision is refused (never downgraded); `schema_migrations` rows must be exactly a prefix of the registry with exact name matches — unknown future versions, name mismatches and gaps are all refused. Column presence is not blind trust: an existing `work_context_events.seq` that is not an INTEGER primary key is refused. `user_version` is never lowered.
4. `PRAGMA user_version` mirrors the current revision as advisory signal only; `schema_migrations` is the source of truth. Additive columns use `ensure_column` (add only when missing; identifiers validated; real errors propagated, never swallowed).
5. The runner re-verifies `PRAGMA foreign_keys` after all migrations; a connection whose FK enforcement state is not healthy is refused, never handed out as ready.
6. Fresh databases converge by running v1..v6 in order; existing databases converge from any historical intermediate state, because every step is idempotent. Behavior for healthy databases is unchanged.

## Accepted dependency direction (recorded, not fully implemented)

- Flow: `cli` / `api` (facade, one-way) → `work` / domain services → `workflow` / `harness` engines → `db` (persistence) → SQLite; effects cross the filesystem only via `tools::repo` (`PathGuard`, in-flight `safe_path`).
- Feature gates are the supported experimental rail (`legacy` off by default); capability claims live in the product-surface inventory, not in this ADR.
- **Known accepted debt (not finished by this ADR):** module-level cycles `db` ↔ `work`, `work` ↔ `harness`, `work` ↔ `workflow`, `flow` ↔ `tools` (A-001/A-002), and the remaining `db`→domain import inversion. The accepted correction path is a domain-owned persistence contract with `db` behind it, one cohesive group per slice: PR #323 (`work::journal_contract`, db→`work::provenance` 15 → 0) is the first; `types` (24 refs), `playbook` (8), `artifact` (6), `service` (5), `domain` (2) remain; then engine→`work` event/port contracts for A-002.

## Consequences

- A failed migration never silently upgrades half a database: the previous revision stays recorded, and the next `Db::new` retries the failed step.
- Schema reviews are now reviewable data (`schema_migrations`), not folklore; new schema changes must arrive as new numbered steps with upgrade and rollback tests (`tests/db_migration_lifecycle.rs`).
- No hand edits of user database files are ever performed by tooling; migrations are code, tested against fixtures built in tempdirs.
- `local_ci.py` remains the authoritative verification; hosted status checks are not authoritative (unchanged).

## Non-decisions

- No dependency on a migration framework crate is introduced.
- This ADR does not authorize promotion of any experimental surface, CORS/API exposure changes, or the `safe_path` rewrite; those remain tracked in the dispatch (A-005/A-006).
