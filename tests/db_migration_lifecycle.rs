//! Characterization and rollback tests for the centralized, versioned
//! SQLite schema migration lifecycle (`src/db/repository/migrations.rs`).
//!
//! These tests pin the historical upgrade paths that `Db::init_schema`
//! previously applied inline, now as ordered, named, recorded steps:
//!
//! - a legacy database whose `work_context_events` table predates the
//!   durable `seq` cursor is rebuilt with data preserved, including events
//!   whose parent context row is absent (FK suspension);
//! - a partially migrated database (seq present, provenance columns absent)
//!   converges to the current schema with its rows intact;
//! - a failing migration leaves the database at the previous revision and
//!   remains retryable.
//!
//! Every fixture is a fresh temporary database; no user database is touched.

use std::path::Path;

use prometheos_lite::db::Db;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use tempfile::TempDir;

const SCHEMA_VERSION: i64 = 6;

fn schema_versions(conn: &Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .expect("schema_migrations query");
    stmt.query_map([], |row| row.get(0))
        .expect("schema_migrations map")
        .collect::<rusqlite::Result<Vec<i64>>>()
        .expect("schema_migrations collect")
}

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user_version pragma")
}

fn column_names(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .expect("table_info");
    stmt.query_map([table], |row| row.get(0))
        .expect("table_info map")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("table_info collect")
}

fn trigger_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
        [name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

fn trigger_sql(conn: &Connection, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
        [name],
        |row| row.get(0),
    )
    .optional()
    .expect("trigger sql query")
}

fn index_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

fn modern_events_ddl() -> &'static str {
    "CREATE TABLE work_context_events (
         seq INTEGER PRIMARY KEY AUTOINCREMENT,
         id TEXT UNIQUE NOT NULL,
         work_context_id TEXT NOT NULL,
         event_type TEXT NOT NULL,
         data TEXT NOT NULL,
         created_at TEXT NOT NULL,
         provenance_json TEXT,
         source_digest TEXT,
         run_id TEXT,
         principal_id TEXT,
         correlation_id TEXT,
         FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
     )"
}

fn legacy_events_ddl() -> &'static str {
    "CREATE TABLE work_context_events (
         id TEXT PRIMARY KEY,
         work_context_id TEXT NOT NULL,
         event_type TEXT NOT NULL,
         data TEXT NOT NULL,
         created_at TEXT NOT NULL
     )"
}

fn minimal_work_contexts_ddl() -> &'static str {
    "CREATE TABLE work_contexts (
         id TEXT PRIMARY KEY,
         user_id TEXT NOT NULL,
         title TEXT NOT NULL,
         domain TEXT NOT NULL,
         context_type TEXT NOT NULL,
         priority TEXT NOT NULL,
         goal TEXT NOT NULL,
         status TEXT NOT NULL,
         current_phase TEXT NOT NULL,
         autonomy_level TEXT NOT NULL,
         approval_policy TEXT NOT NULL,
         last_activity_at TEXT NOT NULL,
         created_at TEXT NOT NULL,
         updated_at TEXT NOT NULL
     )"
}

fn bootstrap_legacy_db(path: &Path, events_ddl: &str, insert_provenance_columns: bool) {
    let conn = Connection::open(path).expect("open legacy fixture");
    // Seed data includes an event whose parent context is intentionally
    // absent; FK enforcement must not be in the way.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("disable FK for fixture seeding");
    conn.execute_batch(minimal_work_contexts_ddl())
        .expect("legacy contexts");
    conn.execute_batch(events_ddl).expect("legacy events");
    if insert_provenance_columns {
        conn.execute_batch(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN SELECT RAISE(ABORT, 'journal rows are append-only'); END",
        )
        .expect("append-only trigger");
    }
    conn.execute(
        "INSERT INTO work_contexts (id, user_id, title, domain, context_type, priority, goal, status, current_phase, autonomy_level, approval_policy, last_activity_at, created_at, updated_at)
         VALUES ('ctx-1', 'user', 'title', 'domain', 'task', 'normal', 'goal', 'open', 'plan', 'supervised', 'ask', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        [],
    )
    .expect("seed context");
    for i in 1..=3 {
        conn.execute(
            "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at)
             VALUES (?1, ?2, 'test', '{}', '2026-01-01T00:00:00Z')",
            [format!("evt-{i}"), "ctx-1".to_string()],
        )
        .expect("seed events");
    }
    // An orphan event referencing a missing context must survive migration.
    conn.execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at)
         VALUES ('evt-orphan', 'ctx-missing', 'test', '{}', '2026-01-01T00:00:00Z')",
        [],
    )
    .expect("seed orphan event");
}

#[test]
fn fresh_in_memory_db_migrates_to_current_version() {
    let db = Db::in_memory().expect("fresh in-memory db");
    let conn = db.conn();
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(user_version(conn), SCHEMA_VERSION);
    assert!(column_names(conn, "work_context_events").contains(&"seq".to_string()));
    assert!(column_names(conn, "work_context_events").contains(&"provenance_json".to_string()));
    assert!(column_names(conn, "playbook_evolutions").contains(&"updated_at".to_string()));
    assert!(trigger_exists(conn, "work_context_events_append_only"));
    assert!(
        trigger_sql(conn, "work_context_events_provenance_required")
            .map(|sql| sql.contains("provenance-trigger-v3"))
            .unwrap_or(false)
    );
    assert!(index_exists(conn, "idx_work_context_events_context_seq"));
    assert!(index_exists(conn, "idx_work_context_events_run_id"));
}

#[test]
fn legacy_db_without_seq_is_rebuilt_and_rows_are_preserved() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("legacy.db");
    bootstrap_legacy_db(&path, legacy_events_ddl(), false);

    let db = Db::new(path.to_str().expect("utf8 path")).expect("legacy db migrates");
    let conn = db.conn();
    let columns = column_names(conn, "work_context_events");
    assert!(
        columns.contains(&"seq".to_string()),
        "seq column added: {columns:?}"
    );
    let seq_pk: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('work_context_events') WHERE name = 'seq' AND pk > 0",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .expect("seq pk check");
    assert!(seq_pk, "seq must be the primary key");
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("count events");
    assert_eq!(
        events, 4,
        "all legacy events, including the orphan, are preserved"
    );
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
    assert!(trigger_exists(conn, "work_context_events_append_only"));
}

#[test]
fn partially_migrated_db_converges_with_rows_intact() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("partial.db");
    // Craft the state left by an older partial migration: seq and the
    // append-only trigger exist, provenance columns do not.
    bootstrap_legacy_db(&path, modern_events_ddl(), true);

    let db = Db::new(path.to_str().expect("utf8 path")).expect("partial db migrates");
    let conn = db.conn();
    let columns = column_names(conn, "work_context_events");
    for required in [
        "provenance_json",
        "source_digest",
        "run_id",
        "principal_id",
        "correlation_id",
    ] {
        assert!(
            columns.contains(&required.to_string()),
            "{required} added: {columns:?}"
        );
    }
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("count events");
    assert_eq!(events, 4);
    assert!(
        trigger_sql(conn, "work_context_events_provenance_required")
            .map(|sql| sql.contains("provenance-trigger-v3"))
            .unwrap_or(false)
    );
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn failing_migration_rolls_back_and_is_retryable() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("failing.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    // Block v6: CREATE TABLE IF NOT EXISTS is a no-op over an existing
    // view, and ALTER TABLE on a view fails, so the migration aborts.
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute_batch("CREATE VIEW playbook_evolutions AS SELECT 'x' AS dummy")
            .expect("create conflicting view");
    }

    let error = Db::new(path.to_str().expect("utf8 path"))
        .err()
        .expect("migration must fail");
    let message = format!("{error:#}");
    assert!(
        message.contains("migration v6"),
        "error names the failing migration: {message}"
    );

    {
        let conn = Connection::open(&path).expect("open after failure");
        assert_eq!(
            schema_versions(&conn),
            vec![1, 2, 3, 4, 5],
            "v6 is unrecorded after rollback"
        );
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
                row.get(0)
            })
            .expect("events still readable");
        assert_eq!(events, 4, "failed migration leaves journal rows intact");
    }

    {
        let conn = Connection::open(&path).expect("open for conflict removal");
        conn.execute_batch("DROP VIEW playbook_evolutions")
            .expect("drop conflict");
    }

    let db = Db::new(path.to_str().expect("utf8 path")).expect("retry converges");
    let conn = db.conn();
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
    assert!(
        trigger_sql(conn, "work_context_events_provenance_required")
            .map(|sql| sql.contains("provenance-trigger-v3"))
            .unwrap_or(false)
    );
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("events after retry");
    assert_eq!(events, 4);
}
