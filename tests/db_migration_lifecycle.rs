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
    let recorded: Vec<(i64, String)> = conn
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .expect("ledger query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("ledger map")
        .collect::<rusqlite::Result<Vec<(i64, String)>>>()
        .expect("ledger collect");
    let expected: Vec<(i64, String)> = [
        (1, "baseline_schema"),
        (2, "work_context_events_seq"),
        (3, "work_context_events_provenance_columns"),
        (4, "work_context_events_indexes"),
        (5, "work_context_events_triggers"),
        (6, "compatibility_schema"),
    ]
    .into_iter()
    .map(|(v, n)| (v, n.to_string()))
    .collect();
    assert_eq!(recorded, expected);
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("fk pragma"),
        1,
        "foreign_keys stays enforced after migration"
    );
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

#[test]
fn future_user_version_is_rejected_and_not_lowered() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("future.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    let error = {
        let conn = Connection::open(&path).expect("open fixture");
        conn.pragma_update(None, "user_version", 99)
            .expect("stamp future user_version");
        conn.execute("DROP TABLE IF EXISTS schema_migrations", [])
            .expect("drop ledger for fixture realism");
        match Db::new(path.to_str().expect("utf8 path")) {
            Err(error) => error,
            Ok(_) => panic!("a database newer than this binary must be refused"),
        }
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("user_version 99"),
        "names the future revision: {message}"
    );
    let conn = Connection::open(&path).expect("reopen fixture");
    assert_eq!(
        user_version(&conn),
        99,
        "older binary must not lower user_version"
    );
}

#[test]
fn unknown_future_migration_version_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("future-migration.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    {
        let db = Db::new(path.to_str().expect("utf8 path")).expect("fixture migrates");
        drop(db);
    }
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (7, 'future_step', '2027-01-01T00:00:00Z')",
            [],
        )
        .expect("stamp unknown future version");
    }
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a version newer than the registry must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("unknown version 7"),
        "names the unknown version: {message}"
    );
}

#[test]
fn migration_name_mismatch_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("name-mismatch.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    {
        let db = Db::new(path.to_str().expect("utf8 path")).expect("fixture migrates");
        drop(db);
    }
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute(
            "UPDATE schema_migrations SET name = 'bogus' WHERE version = 3",
            [],
        )
        .expect("corrupt recorded name");
    }
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a name mismatch must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("version 3"),
        "names the mismatched version: {message}"
    );
    assert!(
        message.contains("name mismatch"),
        "explains the problem: {message}"
    );
}

#[test]
fn invalid_history_gap_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("history-gap.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    {
        let db = Db::new(path.to_str().expect("utf8 path")).expect("fixture migrates");
        drop(db);
    }
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute("DELETE FROM schema_migrations WHERE version = 3", [])
            .expect("punch a hole in the ledger");
    }
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a gap in the ledger must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("missing versions [3]"),
        "names the missing version: {message}"
    );
}

#[test]
fn seq_column_of_wrong_type_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("seq-wrong-type.db");
    let conn = Connection::open(&path).expect("open fixture");
    conn.execute_batch(minimal_work_contexts_ddl())
        .expect("contexts");
    conn.execute_batch(
        "CREATE TABLE work_context_events (
             seq TEXT PRIMARY KEY,
             id TEXT NOT NULL,
             work_context_id TEXT NOT NULL,
             event_type TEXT NOT NULL,
             data TEXT NOT NULL,
             created_at TEXT NOT NULL
         )",
    )
    .expect("corrupt events shape");
    drop(conn);
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a non-INTEGER seq cursor must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("seq") && message.contains("not INTEGER"),
        "explains the refusal: {message}"
    );
}

#[test]
fn seq_column_missing_primary_key_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("seq-no-pk.db");
    let conn = Connection::open(&path).expect("open fixture");
    conn.execute_batch(minimal_work_contexts_ddl())
        .expect("contexts");
    conn.execute_batch(
        "CREATE TABLE work_context_events (
             seq INTEGER,
             id TEXT NOT NULL,
             work_context_id TEXT NOT NULL,
             event_type TEXT NOT NULL,
             data TEXT NOT NULL,
             created_at TEXT NOT NULL
         )",
    )
    .expect("corrupt events shape");
    drop(conn);
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a non-primary-key seq cursor must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("seq") && message.contains("primary key"),
        "explains the refusal: {message}"
    );
}

#[test]
fn partial_history_converges_without_data_loss() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("partial-history.db");
    bootstrap_legacy_db(&path, modern_events_ddl(), true);
    {
        let db = Db::new(path.to_str().expect("utf8 path")).expect("fixture migrates");
        drop(db);
    }
    // Simulate an older binary that had applied v1..v3 but recorded only v1..v3,
    // then a crash before later steps: delete later steps from the ledger.
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute("DELETE FROM schema_migrations WHERE version >= 4", [])
            .expect("punch later steps out of the ledger");
    }
    let db = Db::new(path.to_str().expect("utf8 path")).expect("partial history converges");
    let conn = db.conn();
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("count events");
    assert_eq!(events, 4);
}

#[test]
fn v2_failure_is_atomic_and_fk_is_restored() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("v2-atomic.db");
    // Legacy shape without UNIQUE on id and a duplicate id, so the
    // rebuild's UNIQUE(_new.id) fails deterministically mid-migration.
    {
        let conn = Connection::open(&path).expect("open fixture");
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("fixture fk");
        conn.execute_batch(
            "CREATE TABLE work_context_events (
                 id TEXT,
                 work_context_id TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 data TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             INSERT INTO work_context_events VALUES
                 ('dup-id', 'ctx-f', 'context_created', '{\"a\":1}', '2026-09-01T00:00:00+00:00'),
                 ('dup-id', 'ctx-f', 'status_changed', '{\"a\":2}', '2026-09-02T00:00:00+00:00');",
        )
        .expect("legacy fixture");
    }
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("duplicate ids must fail the rebuild"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("migration v2"),
        "migration v2 is named: {message}"
    );
    let conn = Connection::open(&path).expect("reopen fixture");
    assert_eq!(schema_versions(&conn), vec![1], "only v1 is recorded");
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("original table intact");
    assert_eq!(events, 2);
    let stragglers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'work_context_events_new'",
            [],
            |row| row.get(0),
        )
        .expect("straggler probe");
    assert_eq!(stragglers, 0, "no _new table may survive");
    // Retry with a deduplicated fixture converges; the surviving Db forces
    // foreign_keys ON and verifies it at open.
    {
        let conn = Connection::open(&path).expect("dedupe fixture");
        conn.execute(
            "DELETE FROM work_context_events WHERE rowid IN (SELECT MAX(rowid) FROM work_context_events)",
            [],
        )
        .expect("dedupe");
    }
    let db = Db::new(path.to_str().expect("utf8 path")).expect("retry converges");
    assert_eq!(schema_versions(db.conn()), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(
        db.conn()
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("fk pragma"),
        1,
        "foreign_keys stays enforced after a successful open"
    );
}

#[test]
fn orphan_only_legacy_db_rebuilds_events() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("orphan-only.db");
    // No work_contexts table at all; journal rows may reference anything.
    let conn = Connection::open(&path).expect("open fixture");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("fixture fk");
    conn.execute_batch(legacy_events_ddl()).expect("events");
    conn.execute(
        "INSERT INTO work_context_events (id, work_context_id, event_type, data, created_at)
         VALUES ('evt-1', 'ctx-missing', 'context_created', '{\"a\":1}', '2026-09-01T00:00:00+00:00')",
        [],
    )
    .expect("seed orphan");
    drop(conn);
    let db = Db::new(path.to_str().expect("utf8 path")).expect("orphan-only fixture migrates");
    let conn = db.conn();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("count events");
    assert_eq!(events, 1, "orphan event preserved");
    assert_eq!(schema_versions(conn), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn pre_transaction_cleanup_failure_is_fail_closed_and_retryable() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("pre-tx.db");
    bootstrap_legacy_db(&path, legacy_events_ddl(), false);
    // A view where the straggler _new table should live: DROP TABLE IF EXISTS
    // fails, the pre-transaction phase aborts, nothing is committed or recorded.
    {
        let conn = Connection::open(&path).expect("create conflicting view");
        conn.execute_batch("CREATE VIEW work_context_events_new AS SELECT 1 AS dummy")
            .expect("create conflicting view");
    }
    let error = match Db::new(path.to_str().expect("utf8 path")) {
        Err(error) => error,
        Ok(_) => panic!("a failing pre-transaction phase must be refused"),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("migration v2"),
        "pre-transaction failure is attributed: {message}"
    );
    assert_eq!(
        schema_versions(&Connection::open(&path).expect("reopen")),
        vec![1]
    );
    // The view is still there; FK must be enforced again after the failure.
    let conn = Connection::open(&path).expect("reopen for pragma check");
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("fk pragma"),
        1
    );
    let straggler_view: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND name = 'work_context_events_new'",
            [],
            |row| row.get(0),
        )
        .expect("view probe");
    assert_eq!(straggler_view, 1);
    // Removing the blocker converges.
    {
        let conn = Connection::open(&path).expect("remove view");
        conn.execute_batch("DROP VIEW work_context_events_new")
            .expect("drop view");
    }
    let db = Db::new(path.to_str().expect("utf8 path")).expect("retry converges");
    assert_eq!(schema_versions(db.conn()), vec![1, 2, 3, 4, 5, 6]);
    let events: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM work_context_events", [], |row| {
            row.get(0)
        })
        .expect("count events");
    assert_eq!(events, 4);
}

#[test]
fn post_migration_connection_enforces_fk() {
    // Every Db handed out by this crate must have FK enforcement active; the
    // runner re-verifies after migrations as the last step.
    let db = Db::in_memory().expect("fresh db");
    assert_eq!(
        db.conn()
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("fk pragma"),
        1
    );
}
