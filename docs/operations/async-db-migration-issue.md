# Issue: Migrate from synchronous rusqlite to async database driver

## Context

The current implementation uses synchronous `rusqlite` for database operations in API request handlers. While acceptable for local-first and low-volume scenarios, this becomes a problem for:
- Swarm/parallel agent workloads
- High-concurrency API scenarios
- Long-running database operations that block the reactor thread

## Problem

Synchronous database operations in async request handlers (tokio) can:
- Block the reactor thread, reducing throughput
- Cause thread pool exhaustion under load
- Prevent proper async/await semantics for database operations
- Make it difficult to implement connection pooling for high-concurrency scenarios

## Proposed Solution

Migrate to an async database driver such as:
- `sqlx` (recommended - async, type-safe, supports SQLite, PostgreSQL, MySQL)
- `tokio-postgres` (if moving to PostgreSQL)
- `sea-orm` (async ORM built on sqlx)

### Migration Steps

1. **Add async database dependency**
   - Add `sqlx` with SQLite runtime feature to Cargo.toml
   - Update database initialization to use async connection pool

2. **Update repository layer**
   - Modify all database operations in `src/db/repository/` to use async/await
   - Replace synchronous `rusqlite` calls with async `sqlx` queries
   - Update transaction handling to use async transactions

3. **Update service layer**
   - Modify `WorkContextService` and other services to use async database operations
   - Ensure all service methods are async where they interact with the database

4. **Update API handlers**
   - API handlers already use async, so they should work with the new async database layer
   - Remove any blocking calls or `.await` on synchronous operations

5. **Testing**
   - Update all database tests to use async test runners
   - Verify connection pooling works correctly under load
   - Test transaction rollback and error handling

## Priority

**Medium** - This is not a blocker for V1.2.5 or V1.3, but must be addressed before serious swarm/parallel agent work (V3/V4).

## Impact

- **Low risk**: The change is largely internal to the database layer
- **High benefit**: Enables proper async semantics and better concurrency
- **Migration effort**: Medium - requires updating all database operations

## Dependencies

- None blocking - this can be done in parallel with other V2+ work

## Timeline

Target for V3.0 or when swarm/parallel agent work begins.

## References

- Current synchronous database usage in:
  - `src/db/repository/work_context.rs`
  - `src/db/repository/domain_profile.rs`
  - `src/api/work_contexts.rs` (per-request Db::new calls)
  - `src/work/service.rs` (WorkContextService)
