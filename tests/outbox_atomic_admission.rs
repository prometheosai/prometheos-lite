//! Dispatch 384 — real database-backed admission regressions.
//! These tests use only the public API that exists on main fc61744, so they
//! fail behaviorally (RED) before the atomic-admission repair.

use anyhow::{Context, Result};
use prometheos_lite::db::repository::{Db, OutboxAdmission, OutboxOperations};
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

#[test]
fn concurrent_canonical_claims_admit_one_effect() -> Result<()> {
    use std::sync::{Arc, Barrier};

    let e = env("concurrent-canonical")?;
    let _ = e.db()?;
    let barrier = Arc::new(Barrier::new(2));
    let db_path = Arc::new(e.db_path.clone());
    let canonical = Arc::new(e.canonical.clone());
    let run_id = Arc::new(e.run_id.clone());
    let node_id = Arc::new(e.node_id.clone());

    let outcomes: Vec<anyhow::Result<String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let db_path = db_path.clone();
                let canonical = canonical.clone();
                let run_id = run_id.clone();
                let node_id = node_id.clone();
                let input = e.input.clone();
                scope.spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .expect("test runtime");
                    barrier.wait();
                    runtime.block_on(async {
                        let out = FileWriterNode::with_outbox_db_path(
                            NodeConfig::default(),
                            PathBuf::from(db_path.as_ref()),
                        )
                        .exec(input)
                        .await?;
                        let _ = (&*canonical, &*run_id, &*node_id);
                        Ok(format!("from_cache={}", out["from_cache"]))
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("contender thread panicked"))
            .collect()
    });

    let admitted = outcomes
        .iter()
        .filter(|result| matches!(result, Ok(value) if value == "from_cache=false"))
        .count();
    let cached = outcomes
        .iter()
        .filter(|result| matches!(result, Ok(value) if value == "from_cache=true"))
        .count();
    assert_eq!(
        admitted, 1,
        "exactly one effect must be admitted: {outcomes:?}"
    );
    // A loser arriving while the winner's claim is still pending must refuse
    // an uncertain replay; one arriving after acknowledgement reuses the
    // completed receipt. Both outcomes prove no second effect was admitted.
    let loser_safe = cached == 1
        || outcomes.iter().any(|result| match result {
            Err(error) => error.to_string().contains("unresolved prior outbox claim"),
            _ => false,
        });
    assert!(
        loser_safe,
        "loser neither reused nor safely refused: {outcomes:?}"
    );

    let fresh = e.db()?;
    let claims = fresh.list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.canonical)?;
    assert_eq!(claims.len(), 1, "only one durable claim may exist");
    assert_eq!(claims[0].status, "completed");
    Ok(())
}

#[test]
fn canonical_versus_legacy_claim_race_admits_one_claim() -> Result<()> {
    use std::sync::{Arc, Barrier};

    let e = env("canonical-vs-legacy-race")?;
    let _ = e.db()?;
    let barrier = Arc::new(Barrier::new(2));
    let db_path = Arc::new(e.db_path.clone());
    let legacy = Arc::new(e.legacy.clone());
    let run_id = Arc::new(e.run_id.clone());
    let node_id = Arc::new(e.node_id.clone());
    let input = e.input.clone();

    let outcomes: Vec<anyhow::Result<String>> = std::thread::scope(|scope| {
        let file_handle = {
            let barrier = barrier.clone();
            let db_path = db_path.clone();
            let input = input.clone();
            scope.spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("test runtime");
                barrier.wait();
                runtime.block_on(async {
                    let out = FileWriterNode::with_outbox_db_path(
                        NodeConfig::default(),
                        PathBuf::from(db_path.as_ref()),
                    )
                    .exec(input)
                    .await?;
                    Ok(format!("file:{}", out["from_cache"]))
                })
            })
        };
        let canonical = Arc::new(e.canonical.clone());
        let legacy_handle = {
            let barrier = barrier.clone();
            let db_path = db_path.clone();
            let canonical = canonical.clone();
            let legacy = legacy.clone();
            let run_id = run_id.clone();
            let node_id = node_id.clone();
            scope.spawn(move || {
                let db = Db::new(db_path.to_str().context("non-UTF8 path")?)?;
                barrier.wait();
                match OutboxOperations::admit_outbox_operation(
                    &db,
                    &run_id,
                    "legacy-trace",
                    &node_id,
                    "file_writer",
                    &canonical,
                    &legacy,
                ) {
                    Ok(OutboxAdmission::Admitted(_)) => Ok("dual-admitted".to_string()),
                    Ok(OutboxAdmission::Completed(_)) => Ok("dual-completed".to_string()),
                    Err(error) => Ok(format!("dual-refused:{error}")),
                }
            })
        };
        vec![file_handle, legacy_handle]
            .into_iter()
            .map(|handle| handle.join().expect("contender thread panicked"))
            .collect()
    });

    let fresh = e.db()?;
    let canonical_claims =
        fresh.list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.canonical)?;
    let legacy_claims = fresh.list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.legacy)?;
    assert_eq!(
        canonical_claims.len() + legacy_claims.len(),
        1,
        "one claim total across both key versions: {outcomes:?}"
    );
    let admitted = outcomes
        .iter()
        .filter(|result| matches!(result, Ok(value) if value == "file:false" || value == "dual-admitted"))
        .count();
    assert_eq!(
        admitted, 1,
        "exactly one dual-version contender wins: {outcomes:?}"
    );
    Ok(())
}

#[test]
fn completed_receipt_cannot_be_overwritten() -> Result<()> {
    let e = env("overwrite-completed")?;
    let db = e.db()?;
    let original = json!({"result": "original"}).to_string();
    e.completed(&db, &e.canonical, &original)?;
    let entry = e
        .db()?
        .get_outbox_entry_by_hash(&e.run_id, &e.node_id, &e.canonical)?
        .context("completed entry missing")?;
    let replacement = json!({"result": "overwrite"}).to_string();
    assert!(
        e.db()?
            .mark_outbox_completed_checked(&entry.id, &replacement)
            .is_err(),
        "a completed receipt must not be overwritten"
    );
    let after = e
        .db()?
        .get_outbox_entry_by_hash(&e.run_id, &e.node_id, &e.canonical)?
        .context("completed entry disappeared")?;
    assert_eq!(after.output.as_deref(), Some(original.as_str()));
    Ok(())
}

#[test]
fn malformed_persisted_timestamp_fails_closed() -> Result<()> {
    let e = env("malformed-timestamp")?;
    let db = e.db()?;
    db.conn().execute(
        "INSERT INTO tool_outbox (id, run_id, trace_id, node_id, tool_name, input_hash, status, created_at) VALUES ('bad-time', ?1, 'trace', ?2, 'file_writer', ?3, 'pending', 'not-a-timestamp')",
        rusqlite::params![&e.run_id, &e.node_id, &e.canonical],
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let result = runtime.block_on(e.exec());
    assert!(result.is_err(), "malformed persisted data must fail closed");
    assert!(!e.written());
    Ok(())
}

#[test]
fn dual_version_admission_refuses_when_canonical_receipt_exists() -> Result<()> {
    let e = env("dual-version-refusal")?;
    let db = e.db()?;
    e.completed(
        &db,
        &e.canonical,
        &json!({"result": "canonical"}).to_string(),
    )?;
    let second = e.db()?;
    let decision = OutboxOperations::admit_outbox_operation(
        &second,
        &e.run_id,
        "legacy-trace",
        &e.node_id,
        "file_writer",
        &e.canonical,
        &e.legacy,
    )?;
    assert!(
        matches!(decision, OutboxAdmission::Completed(_)),
        "a legacy-side contender must reuse a canonical receipt"
    );
    let entries = e
        .db()?
        .list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.canonical)?;
    assert_eq!(entries.len(), 1, "a second claim was admitted");
    Ok(())
}

#[test]
fn deterministic_canonical_first_admission() -> Result<()> {
    let e = env("canonical-first")?;
    let first = OutboxOperations::admit_outbox_operation(
        &e.db()?,
        &e.run_id,
        "trace",
        &e.node_id,
        "file_writer",
        &e.canonical,
        &e.legacy,
    )?;
    let admitted = match first {
        OutboxAdmission::Admitted(entry) => entry,
        other => panic!("expected a canonical admission, got {other:?}"),
    };
    // The admitted effect must be durably acknowledged before a later
    // contender can legitimately reuse its completed receipt.
    e.db()?
        .mark_outbox_completed_checked(&admitted.id, &json!({"result": "canonical"}).to_string())?;
    let second = OutboxOperations::admit_outbox_operation(
        &e.db()?,
        &e.run_id,
        "trace",
        &e.node_id,
        "file_writer",
        &e.canonical,
        &e.legacy,
    )?;
    assert!(matches!(second, OutboxAdmission::Completed(_)));
    let entries = e
        .db()?
        .list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.canonical)?;
    assert_eq!(entries.len(), 1);
    Ok(())
}

#[test]
fn deterministic_legacy_first_admission() -> Result<()> {
    let e = env("legacy-first")?;
    let db = e.db()?;
    e.completed(&db, &e.legacy, &json!({"result": "legacy"}).to_string())?;
    let decision = OutboxOperations::admit_outbox_operation(
        &e.db()?,
        &e.run_id,
        "trace",
        &e.node_id,
        "file_writer",
        &e.canonical,
        &e.legacy,
    )?;
    assert!(matches!(decision, OutboxAdmission::Completed(_)));
    let canonical_entries =
        e.db()?
            .list_outbox_entries_by_hash(&e.run_id, &e.node_id, &e.canonical)?;
    assert!(
        canonical_entries.is_empty(),
        "legacy receipt must prevent a canonical claim"
    );
    Ok(())
}
