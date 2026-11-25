//! Dispatch 384 — real database-backed admission regressions.
//! These tests use only the public API that exists on main fc61744, so they
//! fail behaviorally (RED) before the atomic-admission repair.

use anyhow::{Context, Result};
use prometheos_lite::db::repository::{Db, OutboxOperations};
use prometheos_lite::flow::factory::FileWriterNode;
use prometheos_lite::flow::{IdempotencyKey, Node, NodeConfig};
use prometheos_lite::tools::{ToolContext, ToolPolicy};
use serde_json::{Value, json};
use std::path::PathBuf;

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
        .prefix(&format!("pr384-{name}-"))
        .tempdir()?;
    let root = dir.path().canonicalize()?;
    let db_path = root.join(".prometheos").join("runs.db");
    std::fs::create_dir_all(db_path.parent().context("missing db parent")?)?;
    let run_id = format!("run-{name}");
    let trace_id = format!("trace-{name}");
    let node_id = format!("node-{name}");
    let content = format!("content-{name}");
    let file_path = root.join("written.txt");
    let path_text = file_path.to_string_lossy().into_owned();
    let operation_hash = IdempotencyKey::compute_operation_hash(
        "file_writer",
        &json!({"path": path_text, "content": content}),
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
        "file_path": path_text,
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
        Db::new(self.db_path.to_str().context("non-UTF8 path")?)
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
async fn admission_regressions() -> Result<()> {
    // 1. A pending canonical row is an unresolved prior attempt, not permission
    //    to re-execute a protected filesystem effect.
    let e = env("pending-only")?;
    let db = e.db()?;
    e.pending(&db, &e.canonical)?;
    assert!(
        e.exec().await.is_err(),
        "pending must not authorize execution"
    );
    assert!(!e.written(), "uncertain effect must not be re-executed");

    // 2. Duplicate pending claims for the same operation must be detected.
    let e = env("duplicate-pending")?;
    let db = e.db()?;
    e.pending(&db, &e.canonical)?;
    e.pending(&db, &e.canonical)?;
    assert!(
        e.exec().await.is_err(),
        "duplicate pending claims must fail closed"
    );
    assert!(!e.written());

    // 3. Completed receipt on one version wins over a pending row on the other.
    let e = env("completed-versus-pending")?;
    let db = e.db()?;
    e.pending(&db, &e.legacy)?;
    let receipt = json!({"result": "done"}).to_string();
    e.completed(&db, &e.canonical, &receipt)?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!e.written());

    // 4. Conflicting completed receipts are refused.
    let e = env("conflicting-completed")?;
    let db = e.db()?;
    e.completed(&db, &e.canonical, &json!({"result": "a"}).to_string())?;
    e.completed(&db, &e.legacy, &json!({"result": "b"}).to_string())?;
    assert!(e.exec().await.is_err());
    assert!(!e.written());

    // 5. Crash after effect but before acknowledgement: replay must not repeat
    //    the write once the completion trigger is removed.
    let e = env("crash-before-ack")?;
    let db = e.db()?;
    db.conn().execute(
        "CREATE TRIGGER fail_mark BEFORE UPDATE ON tool_outbox BEGIN SELECT RAISE(ABORT, 'mark failure'); END;",
        [],
    )?;
    let first = e.exec().await;
    assert!(
        first.is_err(),
        "unacknowledged effect must surface an error"
    );
    assert!(e.written());
    db.conn().execute("DROP TRIGGER fail_mark", [])?;
    let replay = e.exec().await;
    assert!(
        replay.is_err(),
        "unresolved replay must not re-execute the effect"
    );

    // 6. Completion update affecting zero rows must be an error, not success.
    let e = env("zero-row-update")?;
    let db = e.db()?;
    db.conn().execute(
        "CREATE TRIGGER drop_on_mark BEFORE UPDATE ON tool_outbox BEGIN DELETE FROM tool_outbox WHERE id = OLD.id; SELECT RAISE(IGNORE); END;",
        [],
    )?;
    assert!(
        e.exec().await.is_err(),
        "zero-row completion must fail closed"
    );

    // 7. Persistence/reconnect: a clean first execution records a completed
    //    receipt that a fresh connection can read.
    let e = env("persistence-reconnect")?;
    let _ = e.db()?;
    let out = e.exec().await?;
    assert_eq!(out["from_cache"], json!(false));
    assert!(e.written());
    let fresh = e.db()?;
    let entry = fresh
        .get_outbox_entry_by_hash(&e.run_id, &e.node_id, &e.canonical)?
        .context("canonical completion row missing")?;
    assert_eq!(entry.status, "completed");

    Ok(())
}
