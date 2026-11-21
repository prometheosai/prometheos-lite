//! Versioned, transactional SQLite schema migrations.
//!
//! Before this module, every schema change lived inside `Db::init_schema`
//! as one long imperative sequence, with several steps swallowing errors
//! (`.ok()` / `let _ =`) and no durable record of which schema revision any
//! given database had applied. This runner makes the lifecycle explicit and
//! defensive:
//!
//! - every change is an ordered, named, idempotent [`Migration`];
//! - applied migrations are recorded in `schema_migrations` in the same
//!   transaction as the DDL; a failed step rolls back to the prior recorded
//!   revision and is retryable (migration v2 needs the FK pragma suspended,
//!   which SQLite disallows inside a transaction, so the runner suspends
//!   the pragma around the step's transaction and restores it afterwards
//!   on every path — DDL and ledger record still commit atomically);
//! - idempotent steps re-converge on every open: drift introduced after
//!   the last recorded open is repaired in place. Steps already in the
//!   ledger converge without writing, so an already-migrated database
//!   opens read-only and never blocks on another process holding the
//!   writer lock;
//! - before any DDL runs, the runner validates the recorded history:
//!   unknown future versions, name mismatches, and gaps/overlaps are
//!   rejected, as is a database whose `PRAGMA user_version` is newer than
//!   this binary's revision (an older binary never downgrades or silently
//!   resets a newer database);
//! - `seq` of a text-affinity type is refused; intermediate
//!   integer-affinity spellings (INT/INTEGER/BIGINT, possibly non-pk
//!   after a copy-recreate) are required to converge.
//!
//! `PRAGMA user_version` mirrors the current revision as advisory signal
//! only (it is never lowered by an older binary). `schema_migrations` is
//! the source of truth.

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite::OptionalExtension;

/// One named, idempotent schema migration.
pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    /// Steps that need the foreign-key pragma suspended for their copy
    /// (migration 2 only): the runner suspends it before opening the
    /// step's transaction and restores it after commit or rollback.
    suspend_fk_outside: bool,
    /// Work that must run persistently and ahead of the transaction
    /// (retry hygiene for stragglers from aborted attempts). Runs after
    /// FK suspension, before the transaction. Never runs inside it.
    pre_tx: Option<fn(&Connection) -> Result<()>>,
    apply: fn(&Connection) -> Result<()>,
}

pub(crate) const SCHEMA_VERSION: i64 = 6;

/// Persist first: remove a straggler `_new` table left by an aborted
/// earlier attempt, so retried attempts start clean even if the copy
/// subsequently fails and rolls back.
fn drop_events_new_straggler(conn: &Connection) -> Result<()> {
    conn.execute("DROP TABLE IF EXISTS work_context_events_new", [])
        .context("failed to drop prior migration attempt's work_context_events_new")?;
    Ok(())
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline_schema",
        suspend_fk_outside: false,
        pre_tx: None,
        apply: super::db::migrate_baseline_schema,
    },
    Migration {
        version: 2,
        name: "work_context_events_seq",
        suspend_fk_outside: true,
        pre_tx: Some(drop_events_new_straggler),
        apply: super::db::migrate_work_context_events_seq,
    },
    Migration {
        version: 3,
        name: "work_context_events_provenance_columns",
        suspend_fk_outside: false,
        pre_tx: None,
        apply: super::db::migrate_provenance_columns,
    },
    Migration {
        version: 4,
        name: "work_context_events_indexes",
        suspend_fk_outside: false,
        pre_tx: None,
        apply: super::db::migrate_events_indexes,
    },
    Migration {
        version: 5,
        name: "work_context_events_triggers",
        suspend_fk_outside: false,
        pre_tx: None,
        apply: super::db::migrate_events_triggers,
    },
    Migration {
        version: 6,
        name: "compatibility_schema",
        suspend_fk_outside: false,
        pre_tx: None,
        apply: super::db::migrate_compatibility_schema,
    },
];

/// Defense in depth for format!-built DDL; all call sites pass constants.
fn check_ident(ident: &str) -> Result<()> {
    anyhow::ensure!(
        ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "unsafe identifier in schema migration: {ident:?}"
    );
    Ok(())
}

/// Add a column unless it already exists; real failures are errors, never
/// swallowed.
pub(crate) fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    type_ddl: &str,
) -> Result<()> {
    check_ident(table)?;
    check_ident(column)?;
    let has_column: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2")?
        .query_row([table, column], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)
        .context("failed to inspect column existence")?;
    if !has_column {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {type_ddl}"),
            [],
        )
        .with_context(|| format!("failed to add {table}({column})"))?;
    }
    Ok(())
}

fn record_applied(conn: &Connection, migration: &Migration) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        rusqlite::params![migration.version, migration.name],
    )
    .context("failed to record applied schema migration")?;
    Ok(())
}

/// Run a migration step. `record` is true only when the version is not
/// already in the ledger; already-recorded steps converge in place and
/// must not write (opening a migrated database has to stay read-only).
fn apply_record_commit(conn: &Connection, migration: &Migration, record: bool) -> Result<()> {
    let tx = conn
        .unchecked_transaction()
        .context("failed to begin migration transaction")?;
    (migration.apply)(conn)?;
    if record {
        record_applied(conn, migration)?;
    }
    tx.commit()
        .context("failed to commit migration transaction")?;
    Ok(())
}

fn run_migration(conn: &Connection, migration: &Migration, record: bool) -> Result<()> {
    if migration.suspend_fk_outside {
        let fk_on: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .context("failed to read foreign_keys pragma")?;
        if fk_on != 0 {
            conn.execute("PRAGMA foreign_keys=OFF", [])
                .context("failed to suspend foreign_keys pragma")?;
        }
        // Persisted before the transaction so an aborted attempt still
        // converges on retry; deliberately not covered by the rollback.
        if let Some(pre) = migration.pre_tx
            && let Err(error) = pre(conn)
        {
            let restore = if fk_on != 0 {
                Some(conn.execute("PRAGMA foreign_keys=ON", []))
            } else {
                None
            };
            return match restore {
                Some(Err(restore_error)) => Err(error).context(format!(
                    "pre-transaction migration phase failed; foreign_keys restoration also failed: {restore_error:#}"
                )),
                _ => Err(error).context("pre-transaction migration phase failed"),
            };
        }
        // DDL and the ledger record commit atomically inside the transaction;
        // the FK pragma is restored on every path afterwards. Restoration
        // errors are never swallowed: on the success path they fail the open,
        // on the failure path both errors are preserved.
        let outcome = apply_record_commit(conn, migration, record);
        let restore = if fk_on != 0 {
            Some(
                conn.execute("PRAGMA foreign_keys=ON", [])
                    .context("failed to restore foreign_keys pragma"),
            )
        } else {
            None
        };
        match (outcome, restore) {
            (Ok(()), Some(Err(restore_error))) => Err(restore_error),
            (Ok(()), _) => Ok(()),
            (Err(error), Some(Err(restore_error))) => Err(error).context(format!(
                "migration failed; foreign_keys restoration also failed: {restore_error:#}"
            )),
            (Err(error), _) => Err(error),
        }
    } else {
        apply_record_commit(conn, migration, record)
    }
}

/// Refuse a database whose recorded history this binary cannot trust.
fn validate_history(conn: &Connection) -> Result<std::collections::HashSet<i64>> {
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (version, name) in &rows {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == *version)
            .with_context(|| {
                format!(
                    "schema_migrations records unknown version {version} (as {name:?}); \
                     refusing to open a database written by an incompatible binary"
                )
            })?;
        anyhow::ensure!(
            migration.name == *name,
            "schema_migrations name mismatch for version {version}: recorded {name:?}, expected {:?}",
            migration.name
        );
    }
    let expected = (1..=rows.len() as i64).collect::<Vec<i64>>();
    let recorded: Vec<i64> = rows.iter().map(|(version, _)| *version).collect();
    if recorded != expected {
        let registry_len = MIGRATIONS.len() as i64;
        let missing: Vec<i64> = (1..=registry_len)
            .filter(|version| !recorded.contains(version))
            .collect();
        anyhow::bail!(
            "schema_migrations recorded versions {recorded:?}, expected a prefix of 1..={registry_len}; \
             missing versions {missing:?}; refusing to guess missing steps"
        );
    }
    Ok(recorded.into_iter().collect())
}

/// Column-level sanity for the cursor column: refuse an explicitly
/// TEXT-affinity `seq` (unambiguously broken for a monotonic cursor) but
/// accept any integer-affinity spelling — intermediate historical states
/// carry `INT`/`INTEGER`/`BIGINT` spellings, possibly without the rowid
/// alias primary key, and they are required to converge.
fn validate_events_seq(conn: &Connection) -> Result<()> {
    let has_events: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name = 'work_context_events'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)?;
    if !has_events {
        return Ok(());
    }
    let seq: Option<String> = conn
        .query_row(
            "SELECT type FROM pragma_table_info('work_context_events') WHERE name = 'seq'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(declared_type) = seq {
        let t = declared_type.to_uppercase();
        let text_affinity = ["TEXT", "CHAR", "CLOB", "BLOB", "JSON", "UUID"]
            .iter()
            .any(|needle| t.contains(needle));
        anyhow::ensure!(
            !text_affinity,
            "work_context_events.seq exists but is declared {declared_type:?}, \
             which is not an integer-affinity column; refusing to continue with broken durable cursor semantics"
        );
    }
    Ok(())
}

/// Bring `conn` to the current schema. Safe on any historical state that
/// this binary can validate; refuses on anything newer or inconsistent.
///
/// Every idempotent step re-runs on every open - that is how a database
/// that drifted after its last recorded open (an interrupted init, a
/// manual hotfix, an older binary) converges again. Steps already in the
/// ledger converge in place and are never re-recorded, so an
/// already-migrated database opens read-only and never contends with
/// another process holding the writer lock. `user_version` is only
/// written when a step was first applied. `schema_migrations` records
/// and validates version identity; it never gates re-application.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    // An older binary must never reset a newer database's user_version.
    let user_version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("failed to read user_version pragma")?;
    anyhow::ensure!(
        user_version <= SCHEMA_VERSION,
        "database reports user_version {user_version}, newer than this binary's schema revision {SCHEMA_VERSION}; \
         refusing to open or downgrade it"
    );

    // Read-only probe: only create the ledger when it is missing (the
    // read costs no writer lock; the create does, so we avoid it on the
    // already-migrated happy path).
    let ledger_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)?;
    if !ledger_exists {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version INTEGER PRIMARY KEY,
                 name TEXT NOT NULL,
                 applied_at TEXT NOT NULL
             )",
        )
        .context("failed to create schema_migrations table")?;
    }

    let applied = validate_history(conn)?;
    validate_events_seq(conn)?;

    let mut applied_new = false;
    for migration in MIGRATIONS {
        let is_new = !applied.contains(&migration.version);
        // Every step is idempotent: converge unconditionally on every
        // open. Only never-yet-recorded steps write the ledger; already
        // recorded steps converge in place without writing a byte.
        run_migration(conn, migration, is_new).with_context(|| {
            format!(
                "schema migration v{} ({}) failed",
                migration.version, migration.name
            )
        })?;
        applied_new |= is_new;
    }
    // A connection whose FK enforcement state is not healthy must not be
    // treated as ready for use: re-verify after every migration.
    let fk_enforced: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .context("failed to read foreign_keys pragma after migration")?;
    anyhow::ensure!(
        fk_enforced == 1,
        "foreign_keys pragma is off after schema migration (state unreadable or corrupted); \
         refusing to hand out the connection"
    );
    if applied_new {
        // Advisory mirror of the ledger. Written only when this binary
        // applied at least one step (which means it held the writer);
        // otherwise opening is read-only and never blocks on a locked
        // database. `schema_migrations` remains the source of truth and
        // is never downgraded or rewritten here.
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("failed to set user_version")?;
    }
    Ok(())
}
