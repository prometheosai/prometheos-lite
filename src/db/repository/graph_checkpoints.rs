//! Durable registry of graph-run checkpoints owned by work contexts.
//!
//! #221 / #132 prerequisite: the graph-run decide surface needs a way to
//! answer "which checkpoint is authoritative for run R of context C?"
//! from durable SQL state instead of the CLI's local `.prometheos/checkpoints`
//! directory. This repository is the minimum bearer of that contract:
//! each checkpoint write preserves the raw bytes and recomputes the
//! authoritative content digest (sha256) from them — the digest is never
//! accepted from the caller's memory of the checkpoint.
//!
//! Ownership: the foreign key on `work_context_id` cascades deletion with
//! the parent context; authorization happens at the service/caller boundary
//! (same user_id guard as every other work-context read).

use anyhow::Context;
use chrono::Utc;
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::AsDb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCheckpointRef {
    pub work_context_id: String,
    pub graph_run_id: String,
    /// Digest (sha256 hex) of the stored blob bytes.
    pub checkpoint_digest: String,
    pub created_at: String,
}

/// Register a checkpoint under (context, run), replacing any existing one.
/// Returns the recomputed digest. Fail-closed on an unknown context
/// (FK-enforced) — orphaned checkpoints are refused.
pub fn upsert_checkpoint<T: AsDb>(
    db: &T,
    work_context_id: &str,
    graph_run_id: &str,
    checkpoint_json: &str,
) -> anyhow::Result<String> {
    if work_context_id.trim().is_empty() || graph_run_id.trim().is_empty() {
        anyhow::bail!("work_context_id and graph_run_id must be non-empty");
    }
    let digest = crate::workflow::soma::canonical::sha256_hex(checkpoint_json.as_bytes());
    let now = Utc::now().to_rfc3339();

    db.as_db().conn().execute(
        "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(work_context_id, graph_run_id) DO UPDATE SET checkpoint_json = excluded.checkpoint_json, checkpoint_digest = excluded.checkpoint_digest",
        params![work_context_id, graph_run_id, checkpoint_json, digest, now],
    )
    .context("failed to upsert graph checkpoint")?;

    Ok(digest)
}

/// Snapshot the checkpoint blob and its authoritative digest for a given (context, run).
pub fn get_checkpoint<T: AsDb>(
    db: &T,
    work_context_id: &str,
    graph_run_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    db.as_db()
        .conn()
        .query_row(
            "SELECT checkpoint_json, checkpoint_digest FROM graph_checkpoints
             WHERE work_context_id = ?1 AND graph_run_id = ?2",
            params![work_context_id, graph_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
}

/// List every registered checkpoint for a context, oldest first.
pub fn list_checkpoints<T: AsDb>(
    db: &T,
    work_context_id: &str,
) -> anyhow::Result<Vec<GraphCheckpointRef>> {
    let conn = db.as_db().conn();
    let mut stmt = conn
        .prepare(
            "SELECT work_context_id, graph_run_id, checkpoint_digest, created_at
             FROM graph_checkpoints WHERE work_context_id = ?1
             ORDER BY created_at ASC, graph_run_id ASC",
        )
        .context("failed to prepare checkpoints list")?;
    let rows = stmt
        .query_map(params![work_context_id], |row| {
            Ok(GraphCheckpointRef {
                work_context_id: row.get(0)?,
                graph_run_id: row.get(1)?,
                checkpoint_digest: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .context("failed to query graph checkpoints")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("failed to parse checkpoint row")?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_in(dir: &tempfile::TempDir) -> (std::sync::Arc<crate::db::Db>, String) {
        let db_path = dir.path().join("gc.db").to_str().unwrap().to_string();
        let db = std::sync::Arc::new(crate::db::Db::new(&db_path).expect("db init"));
        (db, db_path)
    }

    fn seed_context(db_path: &str) -> String {
        let db = std::sync::Arc::new(crate::db::Db::new(db_path).expect("db"));
        let svc = crate::work::WorkContextService::new(db);
        svc.create_context(
            "owner".into(),
            "ctx".into(),
            crate::work::types::WorkDomain::General,
            "goal".into(),
        )
        .expect("create")
        .id
    }

    #[test]
    fn checkpoint_registry_is_durable_and_hash_pinned() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&tmp);
        let ctx_id = seed_context(&db_path);

        let d1 = upsert_checkpoint(&*db, &ctx_id, "run-1", "{\"a\":1}").expect("write");
        assert_eq!(d1.len(), 64, "sha256 hex");

        let (blob, digest) = get_checkpoint(&*db, &ctx_id, "run-1")
            .expect("read")
            .expect("present");
        assert_eq!(blob, "{\"a\":1}");
        assert_eq!(digest, d1, "digest recomputed on write survives to read");

        let list = list_checkpoints(&*db, &ctx_id).expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].graph_run_id, "run-1");

        let d2 = upsert_checkpoint(&*db, &ctx_id, "run-1", "{\"a\":2}").expect("upsert");
        assert_ne!(d1, d2, "content change must change digest");
        let (blob2, digest2) = get_checkpoint(&*db, &ctx_id, "run-1").unwrap().unwrap();
        assert_eq!(blob2, "{\"a\":2}");
        assert_eq!(digest2, d2);
    }

    #[test]
    fn checkpoint_registry_refuses_orphan() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, _path) = db_in(&tmp);
        let err = upsert_checkpoint(&*db, "no-such-context", "run-1", "{}")
            .expect_err("FK-enforced refuse");
        assert!(
            format!("{:?}", err).contains("FOREIGN KEY"),
            "expected FK violation, found: {err:?}"
        );
    }
}
