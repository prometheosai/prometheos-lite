//! PR #359 / dispatch 382 regressions for the real FileWriter outbox path.
//! Portable: no process-global cwd changes and no host-specific temp root.

use anyhow::{Context, Result};
use prometheos_lite::db::repository::{Db, OutboxOperations};
use prometheos_lite::flow::factory::FileWriterNode;
use prometheos_lite::flow::{IdempotencyKey, Node, NodeConfig};
use prometheos_lite::tools::{ToolContext, ToolPolicy};
use serde_json::{Value, json};
use std::panic::catch_unwind;
use std::path::{Path, PathBuf};

struct Env {
    _dir: tempfile::TempDir,
    db_path: PathBuf,
    input: Value,
    file_path: PathBuf,
    run_id: String,
    node_id: String,
    canonical: String,
    legacy: String,
}

#[allow(deprecated)]
fn env(name: &str) -> Result<Env> {
    let dir = tempfile::Builder::new()
        .prefix(&format!("pr359-{name}-"))
        .tempdir()?;
    let root = dir.path().canonicalize()?;
    let db_path = root.join(".prometheos").join("runs.db");
    std::fs::create_dir_all(
        db_path
            .parent()
            .context("temporary database has no parent directory")?,
    )?;

    let run_id = format!("run-{name}");
    let trace_id = format!("trace-{name}");
    let node_id = format!("node-{name}");
    let content = format!("content-{name}");
    let file_path = root.join("written.txt");
    let file_path_text = file_path.to_string_lossy().into_owned();
    let operation_hash = IdempotencyKey::compute_operation_hash(
        "file_writer",
        &json!({"path": file_path_text, "content": content}),
    );
    let canonical = IdempotencyKey::compute_key(&run_id, &node_id, &operation_hash);
    let legacy = IdempotencyKey::compute_legacy_key(&run_id, &node_id, &operation_hash);
    let context = ToolContext::new(
        run_id.clone(),
        trace_id.clone(),
        node_id.clone(),
        "file_writer".to_string(),
        ToolPolicy::default(),
    );
    let input = json!({
        "content": content,
        "file_path": file_path_text,
        "tool_context": serde_json::to_value(context)?,
    });
    Ok(Env {
        _dir: dir,
        db_path,
        input,
        file_path,
        run_id,
        node_id,
        canonical,
        legacy,
    })
}

impl Env {
    fn db(&self) -> Result<Db> {
        Db::new(
            self.db_path
                .to_str()
                .context("temporary database path is not UTF-8")?,
        )
    }

    fn pending(&self, db: &Db, key: &str) -> Result<()> {
        db.create_outbox_entry(&self.run_id, "trace", &self.node_id, "file_writer", key)?;
        Ok(())
    }

    fn completed(&self, db: &Db, key: &str, output: &str) -> Result<()> {
        let entry =
            db.create_outbox_entry(&self.run_id, "trace", &self.node_id, "file_writer", key)?;
        db.mark_outbox_completed(&entry.id, output)?;
        Ok(())
    }

    async fn exec(&self) -> Result<Value> {
        FileWriterNode::with_outbox_db_path(NodeConfig::default(), self.db_path.clone())
            .exec(self.input.clone())
            .await
    }

    fn written(&self) -> bool {
        self.file_path.exists()
    }
}

#[tokio::test]
async fn dual_version_outbox_lookup_cases() -> Result<()> {
    // 1. canonical PENDING must not hide legacy COMPLETED.
    let e = env("canonical-pending-legacy-completed")?;
    let db = e.db()?;
    e.pending(&db, &e.canonical)?;
    let receipt = json!({"result": "legacy-cache"}).to_string();
    e.completed(&db, &e.legacy, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(
        out["from_cache"],
        json!(true),
        "canonical pending hid completed legacy receipt: {out}"
    );
    assert_eq!(out["cached_output"], receipt);
    assert!(!e.written(), "completed legacy effect was re-executed");

    // 2. canonical COMPLETED wins over legacy PENDING.
    let e = env("canonical-completed-legacy-pending")?;
    let db = e.db()?;
    e.pending(&db, &e.legacy)?;
    let receipt = json!({"result": "canonical-cache"}).to_string();
    e.completed(&db, &e.canonical, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!e.written());

    // 3. legacy-only completed.
    let e = env("legacy-only-completed")?;
    let db = e.db()?;
    let receipt = json!({"result": "legacy-only"}).to_string();
    e.completed(&db, &e.legacy, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert!(!e.written());

    // 4. canonical-only completed.
    let e = env("canonical-only-completed")?;
    let db = e.db()?;
    let receipt = json!({"result": "canonical-only"}).to_string();
    e.completed(&db, &e.canonical, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert!(!e.written());

    // 5. both versions completed with identical receipts: reuse, no conflict.
    let e = env("dual-completed-identical")?;
    let db = e.db()?;
    let receipt = json!({"result": "same"}).to_string();
    e.completed(&db, &e.canonical, &receipt)?;
    e.completed(&db, &e.legacy, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!e.written());

    // 6. dual completed with conflicting receipts must be rejected.
    let e = env("dual-completed-conflict")?;
    let db = e.db()?;
    e.completed(&db, &e.canonical, &json!({"result": "first"}).to_string())?;
    e.completed(&db, &e.legacy, &json!({"result": "second"}).to_string())?;
    assert!(
        e.exec().await.is_err(),
        "conflicting receipts must fail closed"
    );
    assert!(!e.written(), "conflict must not execute the effect");

    // 7. neither version completed but a pending claim exists: an unresolved
    //    prior attempt must block re-execution (crash-after-effect boundary).
    let e = env("neither-completed")?;
    let db = e.db()?;
    e.pending(&db, &e.legacy)?;
    assert!(
        e.exec().await.is_err(),
        "a pending claim must not be treated as completion"
    );
    assert!(!e.written(), "uncertain effect must not be re-executed");

    // 8. database lookup errors fail closed (unusable database file).
    let e = env("db-lookup-error")?;
    let _ = e.db()?;
    std::fs::write(&e.db_path, b"not a sqlite database")?;
    assert!(e.exec().await.is_err());
    assert!(!e.written());

    // 9. absent database: no effect without durable admission (R2).
    let e = env("no-database")?;
    assert!(!e.db_path.exists());
    let err = e.exec().await;
    assert!(err.is_err(), "absent outbox database must fail closed");
    assert!(!e.written(), "effect executed without durable admission");

    // 10. database creation failure: parent path is a file.
    let e = env("db-creation-failure")?;
    let parent = e.db_path.parent().context("missing parent")?;
    std::fs::remove_dir_all(parent)?;
    std::fs::write(parent, b"not a directory")?;
    assert!(e.exec().await.is_err());
    assert!(!e.written());

    // 11. completion-mark failure: the effect is uncertain and reported as error.
    let e = env("mark-failure")?;
    let db = e.db()?;
    db.conn().execute(
        "CREATE TRIGGER fail_mark BEFORE UPDATE ON tool_outbox BEGIN SELECT RAISE(ABORT, 'mark failure'); END;",
        [],
    )?;
    assert!(
        e.exec().await.is_err(),
        "unrecorded completion must be an error"
    );
    assert!(
        e.written(),
        "the effect happened but its completion was not recorded"
    );

    Ok(())
}

#[test]
fn temporary_directory_is_removed_after_panic() {
    let path = std::sync::Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let captured = path.clone();
    let result = catch_unwind(move || {
        let dir = tempfile::Builder::new()
            .prefix("pr359-panic-")
            .tempdir()
            .unwrap();
        *captured.lock().unwrap() = dir.path().to_path_buf();
        panic!("intentional cleanup test");
    });
    assert!(result.is_err());
    let path = path.lock().unwrap().clone();
    assert!(
        !path.exists(),
        "temporary directory leaked after test failure"
    );
}

#[test]
fn test_source_is_portable() -> Result<()> {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pr359_filewriter_outbox.rs"),
    )?;
    let drive = format!("{}:/PrometheOS-Lite", 'D');
    let cwd = format!("set_{}", "current_dir");
    assert!(
        !source.contains(&drive),
        "host-specific path in test source"
    );
    assert!(
        !source.contains(&cwd),
        "process-global cwd mutation in test source"
    );
    assert!(source.contains("with_outbox_db_path"));
    Ok(())
}
