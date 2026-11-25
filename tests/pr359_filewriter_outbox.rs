//! PR #359 regression: dual-version outbox lookup must consider completion
//! across canonical and legacy keys before deciding to execute an effect.

use prometheos_lite::db::repository::{Db, OutboxOperations};
use prometheos_lite::flow::factory::FileWriterNode;
use prometheos_lite::flow::{IdempotencyKey, Node, NodeConfig};
use prometheos_lite::tools::{ToolContext, ToolPolicy};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Case {
    _dir: tempfile::TempDir,
    db: Db,
    run_id: String,
    trace_id: String,
    node_id: String,
    content: String,
    file_path: String,
    canonical: String,
    legacy: String,
    context: ToolContext,
}

#[allow(deprecated)]
fn case(name: &str) -> anyhow::Result<Case> {
    let root = PathBuf::from("D:/PrometheOS-Lite/temp/pr-381");
    std::fs::create_dir_all(&root)?;
    let dir = tempfile::Builder::new().prefix(name).tempdir_in(&root)?;
    std::env::set_current_dir(dir.path())?;
    std::fs::create_dir_all(".prometheos")?;
    let db = Db::new(".prometheos/runs.db")?;

    let run_id = format!("run-{name}");
    let trace_id = format!("trace-{name}");
    let node_id = format!("node-{name}");
    let content = format!("content-{name}");
    let file_path = dir
        .path()
        .join("written.txt")
        .to_string_lossy()
        .into_owned();
    let operation_hash = IdempotencyKey::compute_operation_hash(
        "file_writer",
        &json!({"path": file_path, "content": content}),
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
    Ok(Case {
        _dir: dir,
        db,
        run_id,
        trace_id,
        node_id,
        content,
        file_path,
        canonical,
        legacy,
        context,
    })
}

impl Case {
    fn pending(&self, key: &str) -> anyhow::Result<()> {
        self.db.create_outbox_entry(
            &self.run_id,
            &self.trace_id,
            &self.node_id,
            "file_writer",
            key,
        )?;
        Ok(())
    }

    fn completed(&self, key: &str, output: &str) -> anyhow::Result<()> {
        let entry = self.db.create_outbox_entry(
            &self.run_id,
            &self.trace_id,
            &self.node_id,
            "file_writer",
            key,
        )?;
        self.db.mark_outbox_completed(&entry.id, output)?;
        Ok(())
    }

    async fn exec(&self) -> anyhow::Result<Value> {
        let input = json!({
            "content": self.content,
            "file_path": self.file_path,
            "tool_context": serde_json::to_value(&self.context)?,
        });
        FileWriterNode::new(NodeConfig::default()).exec(input).await
    }

    fn written(&self) -> bool {
        Path::new(&self.file_path).exists()
    }
}

#[tokio::test]
async fn dual_version_outbox_lookup_cases() -> anyhow::Result<()> {
    let original = std::env::current_dir()?;

    // 1. canonical PENDING must not hide legacy COMPLETED.
    let c = case("canonical-pending-legacy-completed")?;
    c.pending(&c.canonical)?;
    let receipt = json!({"result": "legacy-cache"}).to_string();
    c.completed(&c.legacy, &receipt)?;
    let out = c.exec().await?;
    assert_eq!(
        out["from_cache"],
        json!(true),
        "canonical pending hid completed legacy receipt: {out}"
    );
    assert_eq!(out["cached_output"], receipt);
    assert!(!c.written(), "completed legacy effect was re-executed");

    // 2. canonical COMPLETED wins over legacy PENDING.
    let c = case("canonical-completed-legacy-pending")?;
    c.pending(&c.legacy)?;
    let receipt = json!({"result": "canonical-cache"}).to_string();
    c.completed(&c.canonical, &receipt)?;
    let out = c.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!c.written());

    // 3. legacy-only completed.
    let c = case("legacy-only-completed")?;
    let receipt = json!({"result": "legacy-only"}).to_string();
    c.completed(&c.legacy, &receipt)?;
    let out = c.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!c.written());

    // 4. canonical-only completed.
    let c = case("canonical-only-completed")?;
    let receipt = json!({"result": "canonical-only"}).to_string();
    c.completed(&c.canonical, &receipt)?;
    let out = c.exec().await?;
    assert_eq!(out["from_cache"], json!(true));
    assert_eq!(out["cached_output"], receipt);
    assert!(!c.written());

    // 5. dual completed with conflicting receipts must be rejected.
    let c = case("dual-completed-conflict")?;
    c.completed(&c.canonical, &json!({"result": "first"}).to_string())?;
    c.completed(&c.legacy, &json!({"result": "second"}).to_string())?;
    let err = c.exec().await;
    assert!(
        err.is_err(),
        "conflicting completed receipts must fail closed"
    );
    assert!(!c.written(), "conflict must not execute the effect");

    // 6. neither version completed -> real execution, canonical-only new write.
    let c = case("neither-completed")?;
    c.pending(&c.legacy)?;
    let out = c.exec().await?;
    assert_eq!(out["from_cache"], json!(false));
    assert!(
        c.written(),
        "uncertain/pending effects must not be treated as completed"
    );
    let canonical_entry =
        c.db.get_outbox_entry_by_hash(&c.run_id, &c.node_id, &c.canonical)?;
    let canonical_entry = canonical_entry.expect("canonical outbox entry created");
    assert_eq!(
        canonical_entry.status, "completed",
        "new writes must use canonical key"
    );

    // 7. database lookup errors fail closed (unusable database file).
    let c = case("db-lookup-error")?;
    std::fs::write(".prometheos/runs.db", b"not a sqlite database")?;
    let err = c.exec().await;
    assert!(
        err.is_err(),
        "database lookup failure must not fall through to execution"
    );
    assert!(!c.written());

    std::env::set_current_dir(original)?;
    Ok(())
}
