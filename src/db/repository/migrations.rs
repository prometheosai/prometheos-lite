//! Versioned, transactional SQLite schema migrations.
//!
//! Before this module, every schema change lived inside `Db::init_schema`
//! as one long imperative sequence, with several steps swallowing errors
//! (`.ok()` / `let _ =`) and no durable record of which schema revision any
//! given database had applied. This runner makes the lifecycle explicit:
//!
//! - every change is an ordered, named, idempotent [`Migration`];
//! - applied migrations are recorded in `schema_migrations`; for all steps
//!   except v2 the DDL and its record commit in one transaction, so a
//!   failed step leaves the database at the prior revision and retry-safe;
//! - step v2 (`work_context_events_seq`) suspends the FK pragma around its
//!   copy-and-rename and manages its own transaction, exactly as before;
//!   it is atomic on its own and its re-run is a no-op.
//!
//! `PRAGMA user_version` mirrors the current revision as advisory signal.
//! `schema_migrations` is the source of truth.

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// One named, idempotent schema migration.
pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    /// Steps that manage their own transaction and FK-pragma dance
    /// (migration 2 only) are wrapped here, never in the runner's transaction.
    self_managed: bool,
    apply: fn(&Connection) -> Result<()>,
}

pub(crate) const SCHEMA_VERSION: i64 = 6;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline_schema",
        self_managed: false,
        apply: super::db::migrate_baseline_schema,
    },
    Migration {
        version: 2,
        name: "work_context_events_seq",
        self_managed: true,
        apply: super::db::migrate_work_context_events_seq,
    },
    Migration {
        version: 3,
        name: "work_context_events_provenance_columns",
        self_managed: false,
        apply: super::db::migrate_provenance_columns,
    },
    Migration {
        version: 4,
        name: "work_context_events_indexes",
        self_managed: false,
        apply: super::db::migrate_events_indexes,
    },
    Migration {
        version: 5,
        name: "work_context_events_triggers",
        self_managed: false,
        apply: super::db::migrate_events_triggers,
    },
    Migration {
        version: 6,
        name: "compatibility_schema",
        self_managed: false,
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
        "INSERT OR REPLACE INTO schema_migrations (version, name, applied_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        rusqlite::params![migration.version, migration.name],
    )
    .context("failed to record applied schema migration")?;
    Ok(())
}

fn run_migration(conn: &Connection, migration: &Migration) -> Result<()> {
    if migration.self_managed {
        (migration.apply)(conn)?;
        record_applied(conn, migration)?;
        return Ok(());
    }
    let tx = conn
        .unchecked_transaction()
        .context("failed to begin migration transaction")?;
    (migration.apply)(conn)?;
    record_applied(conn, migration)?;
    tx.commit()
        .context("failed to commit migration transaction")?;
    Ok(())
}

/// Bring `conn` to the current schema. Safe on any historical state.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL
         )",
    )
    .context("failed to create schema_migrations table")?;
    let applied: HashSet<i64> = conn
        .prepare("SELECT version FROM schema_migrations")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<HashSet<i64>>>()?;
    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }
        run_migration(conn, migration).with_context(|| {
            format!(
                "schema migration v{} ({}) failed",
                migration.version, migration.name
            )
        })?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("failed to set user_version")?;
    Ok(())
}
