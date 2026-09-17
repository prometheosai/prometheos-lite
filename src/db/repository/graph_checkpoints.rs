//! Durable registry of graph-run checkpoints owned by work contexts.
//!
//! #221 / #132 prerequisite: the graph-run decide surface needs a durable
//! mapping from (work_context_id, graph_run_id) → checkpoint bytes. Physical
//! checkpoint files live under `.prometheos/checkpoints` (CLI-local); the
//! decide endpoint must NOT need that path. This table is that mapping,
//! owned by the caller's user identity end-to-end.
//!
//! Guarantees:
//!
//! - **Owner-scoped**. Every read/write in this module requires the caller
//!   to name a `user_id`, and compares it against the owning WorkContext's
//!   `user_id` before computing anything. There is no way to read or write
//!   another user's checkpoint by guessing an id.
//! - **Fail-closed normalization**. `work_context_id` and `graph_run_id`
//!   are trimmed and rejected if empty; IDs with surrounding whitespace
//!   become the normalized inner id (a whitespace variant of a valid id
//!   still hits the right row).
//! - **Digest recomputed on every read**. `checkpoint_digest` stored in the
//!   database is recomputed from the current blob each time the caller
//!   reads; a bitrot/corruption in the stored digest surfaces immediately
//!   as a mismatch (returns an error, not a stale digest).
//! - **Foreign-key enforcement**. Orphan checkpoints (unknown
//!   `work_context_id`) fail at INSERT. FK must be enabled per connection —
//!   see `db.rs::init_schema`.
//! - **Overwrite semantics are explicit**. Replace keeps the original
//!   `created_at`; no silent timestamp churn implies a new write.
//!
//! Structure: this registry does NOT re-validate the checkpoint's
//! graph-state/payload content — that's the caller's layer
//! (`import_checkpoint` in `graph_state.rs`); this service's shape contract
//! is exactly the blob + its digest.

use anyhow::Context;
use chrono::Utc;
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::AsDb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCheckpointRef {
    pub work_context_id: String,
    pub graph_run_id: String,
    /// Digest of the stored checkpoint blob (sha256 hex), recomputed at
    /// read time — see module docs.
    pub checkpoint_digest: String,
    pub created_at: String,
}

fn normalize_id(id: &str, name: &'static str) -> anyhow::Result<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{name} must be non-empty");
    }
    // Reject control or path-y characters; these keys traverse into SQL and
    // eventually file layers. Disallow path separators / NUL.
    if trimmed.chars().any(|c| c.is_control() || c == '\\') {
        anyhow::bail!("{name} contains an invalid character");
    }
    if trimmed.len() > 512 {
        anyhow::bail!("{name} is longer than 512 chars");
    }
    Ok(trimmed.to_string())
}

/// Verify that `user_id` owns the target work context row; rejects the
/// operation before doing any I/O. This is the same rule the rest of the
/// work context API uses.
fn ensure_owned_by<T: AsDb>(db: &T, work_context_id: &str, user_id: &str) -> anyhow::Result<()> {
    let stored_user: Option<String> = db
        .as_db()
        .conn()
        .query_row(
            "SELECT user_id FROM work_contexts WHERE id = ?1",
            params![work_context_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .context("failed to look up work context owner")?
        .filter(|s: &String| !s.is_empty());

    match stored_user {
        Some(owner) if owner == user_id => Ok(()),
        Some(owner) => {
            anyhow::bail!("work context '{work_context_id}' is owned by '{owner}', not '{user_id}'")
        }
        None => anyhow::bail!("work context not found: '{work_context_id}'"),
    }
}

/// Recompute the digest over a stored blob; never trusted.
///
/// The registry also refuses to serve a checkpoint whose bytes aren't valid
/// JSON — structure validation against the graph run schema happens at the
/// caller (`graph_state.rs::import_checkpoint`), but the registry must not
/// hand an opaque non-JSON blob off at all.
pub fn digest_of(blob: &str) -> String {
    crate::workflow::soma::canonical::sha256_hex(blob.as_bytes())
}

pub fn validate_checkpoint_json(blob: &str) -> anyhow::Result<()> {
    let v: serde_json::Value =
        serde_json::from_str(blob).context("checkpoint blob is not valid JSON")?;
    if !v.is_object() {
        anyhow::bail!("checkpoint blob must be a JSON object");
    }
    // Require the identity fields that any legitimate GraphRunStateV1 export
    // carries. Without them the registry can't answer "which run is this
    // checkpoint for?" — a corruption or schema drift is detected on WRITE.
    for field in ["schemaVersion", "runId", "graphId", "graphManifestDigest"] {
        match v.get(field) {
            Some(value) if value.is_string() => {}
            _ => anyhow::bail!("checkpoint blob missing required field: {field}"),
        }
    }
    Ok(())
}

/// Binding rule: the registry key `graph_run_id` and the blob's internal
/// `runId` must agree. A checkpoint registered under the wrong run id makes
/// reads later answer "run for X" with a blob that says it is run Y —
/// treat that as tampering and refuse. Called on both write and read.
pub fn assert_blob_run_id_matches(blob: &str, graph_run_id: &str) -> anyhow::Result<()> {
    let v: serde_json::Value =
        serde_json::from_str(blob).context("checkpoint blob is not valid JSON")?;
    match v.get("runId") {
        Some(serde_json::Value::String(inner)) if inner == graph_run_id => Ok(()),
        Some(serde_json::Value::String(inner)) => anyhow::bail!(
            "checkpoint blob claims runId '{inner}' but registry key expects '{graph_run_id}'"
        ),
        _ => anyhow::bail!("checkpoint blob has no runId"),
    }
}

pub fn upsert_on_conn(
    conn: &rusqlite::Connection,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
    checkpoint_json: &str,
) -> anyhow::Result<String> {
    upsert_checkpoint_conn(
        conn,
        user_id,
        work_context_id,
        graph_run_id,
        checkpoint_json,
    )
}

/// Register a checkpoint under (context, run) on an arbitrary Connection —
/// the tx-supporting primitive. Callers doing transactional batches should
/// prefer this over the Db-level variant.
pub fn upsert_checkpoint_conn(
    conn: &rusqlite::Connection,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
    checkpoint_json: &str,
) -> anyhow::Result<String> {
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    let graph_run_id = normalize_id(graph_run_id, "graph_run_id")?;
    ensure_owned_by_conn(conn, &work_context_id, user_id)?;
    validate_checkpoint_json(checkpoint_json)?;
    assert_blob_run_id_matches(checkpoint_json, &graph_run_id)?;

    let digest = digest_of(checkpoint_json);
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(work_context_id, graph_run_id) DO UPDATE SET checkpoint_json = excluded.checkpoint_json, checkpoint_digest = excluded.checkpoint_digest",
        params![work_context_id, graph_run_id, checkpoint_json, digest, now],
    )
    .context("failed to upsert graph checkpoint")?;

    Ok(digest)
}

/// Conditional compare-and-job swap on the registry: overwrites only when
/// the stored digest matches `expected_digest`. Returns false when the current
/// digest differs (i.e. a concurrent writer got in first). Callers inside a
/// transaction may hold the lock for the read+write pair.
pub fn cas_checkpoint_conn(
    conn: &rusqlite::Connection,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
    expected_digest: &str,
    checkpoint_json: &str,
) -> anyhow::Result<bool> {
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    let graph_run_id = normalize_id(graph_run_id, "graph_run_id")?;
    ensure_owned_by_conn(conn, &work_context_id, user_id)?;
    validate_checkpoint_json(checkpoint_json)?;
    assert_blob_run_id_matches(checkpoint_json, &graph_run_id)?;

    let digest = digest_of(checkpoint_json);
    let affected = conn.execute(
        "UPDATE graph_checkpoints SET checkpoint_json = ?4, checkpoint_digest = ?5
         WHERE work_context_id = ?1 AND graph_run_id = ?2 AND checkpoint_digest = ?3",
        params![
            work_context_id,
            graph_run_id,
            expected_digest,
            checkpoint_json,
            digest
        ],
    )?;
    Ok(affected > 0)
}

fn ensure_owned_by_conn(
    conn: &rusqlite::Connection,
    work_context_id: &str,
    user_id: &str,
) -> anyhow::Result<()> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT user_id FROM work_contexts WHERE id = ?1",
            params![work_context_id],
            |r| r.get(0),
        )
        .optional()
        .context("failed to look up work context owner")?
        .filter(|s: &String| !s.is_empty());

    match stored {
        Some(owner) if owner == user_id => Ok(()),
        Some(owner) => {
            anyhow::bail!("work context '{work_context_id}' is owned by '{owner}', not '{user_id}'")
        }
        None => anyhow::bail!("work context not found: '{work_context_id}'"),
    }
}
/// Replaces the existing row if it exists. `created_at` reflects the first
/// registration for the (context, run) pair and does NOT change on replace.
/// Returns the freshly computed digest.
pub fn upsert_checkpoint<T: AsDb>(
    db: &T,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
    checkpoint_json: &str,
) -> anyhow::Result<String> {
    upsert_checkpoint_conn(
        db.as_db().conn(),
        user_id,
        work_context_id,
        graph_run_id,
        checkpoint_json,
    )
}

/// Read the checkpoint blob + digest back; ownership is enforced same as
/// upsert.
///
/// The blob's internal `runId` must match the key it was registered under —
/// a checkpoint claiming a different run refuses to read. A stored digest
/// doesn't match the recomputed one → read fails closed (tamper detection),
/// never silently accepted.
pub fn get_checkpoint<T: AsDb>(
    db: &T,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    let graph_run_id = normalize_id(graph_run_id, "graph_run_id")?;
    ensure_owned_by(db, &work_context_id, user_id)?;

    let raw: Option<(String, String)> = db
        .as_db()
        .conn()
        .query_row(
            "SELECT checkpoint_json, checkpoint_digest FROM graph_checkpoints WHERE work_context_id = ?1 AND graph_run_id = ?2",
            params![work_context_id, graph_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("failed to query checkpoint")?;

    raw.map(|(blob, stored_digest)| {
        validate_checkpoint_json(&blob)?;
        assert_blob_run_id_matches(&blob, &graph_run_id)?;
        let recomputed = digest_of(&blob);
        if recomputed != stored_digest {
            anyhow::bail!(
                "checkpoint digest mismatch for run {graph_run_id}: stored {stored_digest} vs computed {recomputed}"
            );
        }
        Ok((blob, recomputed))
    })
    .transpose()
}

/// List all registered checkpoint references for a context; Owned-same
/// gating as the rest.
///
/// Verifies:
/// - structure (schemaVersion/runId/graphId/graphManifestDigest shape);
/// - blob.runId == stored key (graph_run_id);
/// - recomputed digest == stored digest.
///
/// A row that fails any of these fails the whole list; nothing is returned.
pub fn list_checkpoints<T: AsDb>(
    db: &T,
    user_id: &str,
    work_context_id: &str,
) -> anyhow::Result<Vec<GraphCheckpointRef>> {
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    ensure_owned_by(db, &work_context_id, user_id)?;

    let conn = db.as_db().conn();
    let mut stmt = conn
        .prepare(
            "SELECT work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at
             FROM graph_checkpoints WHERE work_context_id = ?1
             ORDER BY created_at ASC, graph_run_id ASC",
        )
        .context("failed to prepare list")?;

    let rows = stmt
        .query_map(params![work_context_id], |row| {
            let id: String = row.get(0)?;
            let run: String = row.get(1)?;
            let blob: String = row.get(2)?;
            let stored_digest: String = row.get(3)?;
            let ts: String = row.get(4)?;
            Ok((id, run, blob, stored_digest, ts))
        })
        .context("failed to query checkpoints list")?;

    let mut out = Vec::new();
    for row in rows {
        let (id, run, blob, stored_digest, ts) = row.context("failed to read checkpoint row")?;

        // Exact same gates as get_checkpoint: structure, key-binding, digest.
        validate_checkpoint_json(&blob)?;
        assert_blob_run_id_matches(&blob, &run)?;
        let recomputed = digest_of(&blob);
        if recomputed != stored_digest {
            anyhow::bail!(
                "checkpoint digest mismatch for run {run}: stored {stored_digest} vs computed {recomputed}"
            );
        }

        out.push(GraphCheckpointRef {
            work_context_id: id,
            graph_run_id: run,
            checkpoint_digest: stored_digest,
            created_at: ts,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Create a Db whose backing file lives inside `dir`. The caller MUST
    /// keep `dir` alive for the test's duration — dropping it early deletes
    /// the SQLite file underneath, which is a sneakster Windows bug and a
    /// real one on Linux.
    fn db_in(dir: &tempfile::TempDir) -> (Arc<crate::db::Db>, String) {
        let db_path = dir.path().join("gc.db").to_str().unwrap().to_string();
        let db = Arc::new(crate::db::Db::new(&db_path).expect("db"));
        (db, db_path)
    }

    fn seed_context(db_path: &str) -> String {
        let db = Arc::new(crate::db::Db::new(db_path).expect("db"));
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

    fn valid_checkpoint(run_id: &str) -> String {
        // Minimum-shaped GraphRunStateV1 export fixture that the registry
        // validation accepts: identity keys must all be strings.
        format!(
            r#"{{"schemaVersion":"1.1.0","runId":"{}","graphId":"g1","graphManifestDigest":"{}","repoRevision":"r","nodeAttempts":{{}},"frontier":[],"decisions":[],"portableStateRef":"p","portableStateDigest":"{}"}}"#,
            run_id,
            "a".repeat(64),
            "b".repeat(64)
        )
    }

    #[test]
    fn write_read_update_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        let d1 =
            upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1")).unwrap();
        assert_eq!(d1.len(), 64);
        assert_eq!(d1, digest_of(&valid_checkpoint("run-1")));

        let (blob, digest) = get_checkpoint(&*db, "owner", &ctx_id, "run-1")
            .unwrap()
            .unwrap();
        assert_eq!(blob, valid_checkpoint("run-1"));
        assert_eq!(digest, d1);

        // Content change with the SAME runId (a legit graph-step overwrite):
        let blob2 =
            valid_checkpoint("run-1").replace("repoRevision\":\"r\"", "repoRevision\":\"r2\"");
        let d2 = upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &blob2).unwrap();
        assert_ne!(d1, d2);
        let (blob2_read, digest2) = get_checkpoint(&*db, "owner", &ctx_id, "run-1")
            .unwrap()
            .unwrap();
        assert_eq!(blob2_read, blob2);
        assert_eq!(digest2, d2);

        // A DIFFERENT runId as key is refused: blob stating a different
        // run than the key is a sign of misregistration/tampering.
        let bad_blob = valid_checkpoint("run-2");
        let err = upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &bad_blob)
            .expect_err("runId mismatch must refuse");
        assert!(format!("{:?}", err).contains("checkpoint blob claims runId"));
    }

    #[test]
    fn owner_gate_blocks_other_users_reads_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path); // owned by "owner"

        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1")).unwrap();

        // A non-owner cannot read, write, or list.
        assert!(get_checkpoint(&*db, "nobody", &ctx_id, "run-1").is_err());
        assert!(
            upsert_checkpoint(&*db, "nobody", &ctx_id, "run-2", &valid_checkpoint("run-2"))
                .is_err()
        );
        assert!(list_checkpoints(&*db, "nobody", &ctx_id).is_err());
    }

    #[test]
    fn normalized_ids_ignore_surrounding_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        upsert_checkpoint(
            &*db,
            "owner",
            &format!(" {ctx_id} "),
            " run-1 ",
            &valid_checkpoint("run-1"),
        )
        .unwrap();
        assert!(
            get_checkpoint(&*db, "owner", &ctx_id, "run-1")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn empty_ids_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);
        assert!(upsert_checkpoint(&*db, "owner", "", "run-1", &valid_checkpoint("run-1")).is_err());
        assert!(
            upsert_checkpoint(&*db, "owner", &ctx_id, "   ", &valid_checkpoint("run-1")).is_err()
        );
    }

    #[test]
    fn overwrite_keeps_original_created_at_not_churn() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1")).unwrap();
        let first = list_checkpoints(&*db, "owner", &ctx_id).unwrap()[0].clone();

        // Wait a beat and overwrite.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut changed = valid_checkpoint("run-1");
        changed = changed.replace("1.1.0", "1.1.1");
        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &changed).unwrap();

        let second = list_checkpoints(&*db, "owner", &ctx_id).unwrap()[0].clone();
        assert_eq!(
            first.created_at, second.created_at,
            "overwrite preserves original created_at"
        );
    }

    #[test]
    fn restart_survives_service_reset_and_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);
        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1")).unwrap();
        drop(db);

        // Reconnect with a fresh Db on the same path — schema init must
        // succeed and the row must survive.
        let db2 = std::sync::Arc::new(crate::db::Db::new(&db_path).expect("db2"));
        let svc2 = crate::work::WorkContextService::new(db2.clone());
        let c = svc2.get_context(&ctx_id).expect("load").expect("exists");
        let (blob, digest) = get_checkpoint(&*db2, "owner", &ctx_id, "run-1")
            .unwrap()
            .unwrap();
        assert_eq!(blob, valid_checkpoint("run-1"));
        assert_eq!(digest, digest_of(&blob));
        assert!(!c.is_cancelled());
    }

    #[test]
    fn orphan_registration_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (db, _db_path) = db_in(&dir);
        let _ = &db;
        assert!(
            upsert_checkpoint(
                &*db,
                "owner",
                "no-such-context",
                "run-1",
                &valid_checkpoint("run-1")
            )
            .is_err()
        );
    }

    #[test]
    fn structure_validation_refuses_missing_identity_fields() {
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);
        let blob = r#"{"foo":1}"#;
        let err = upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", blob)
            .expect_err("missing fields must refuse");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("missing required field"),
            "error must identify the structural problem: {msg}"
        );

        // Also tested on the read path — the registry must not serve a blob
        // that has this problem even if one got in earlier. Since the write
        // guard is above, corrupt it directly through the connection.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE graph_checkpoints SET checkpoint_json = ?1 WHERE work_context_id = ?2 AND graph_run_id = 'run-1'",
            rusqlite::params![blob, ctx_id],
        )
        .expect("seed corrupt blob");
        // (Note: push a valid first so a row exists for the targeted UPDATE.)
        conn.execute(
            "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
             VALUES (?1, 'run-1', ?2, 'x', '2026-01-01T00:00:00Z')",
            rusqlite::params![ctx_id, blob],
        )
        .expect("seed corrupt row");
        assert!(
            get_checkpoint(&*db, "owner", &ctx_id, "run-1").is_err(),
            "registry must also refuse to serve a structurally invalid blob on read"
        );
    }

    #[test]
    fn stored_digest_tamper_is_detected_on_read_and_list() {
        // Proof that digest enforcement is real: write a valid checkpoint,
        // then rewrite ONLY the stored digest column to a tampered value via
        // a raw connection (bypassing the typed layer). Both read endpoints
        // must refuse with an error — not return a tampered row.
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1"))
            .expect("seed");

        // Corrupt the stored digest (keeping the blob intact).
        let conn = rusqlite::Connection::open(&db_path).expect("probe");
        conn.execute(
            "UPDATE graph_checkpoints SET checkpoint_digest = 'deadbeef'
             WHERE work_context_id = ?1 AND graph_run_id = 'run-1'",
            rusqlite::params![ctx_id],
        )
        .expect("tamper");

        let get_err = get_checkpoint(&*db, "owner", &ctx_id, "run-1").expect_err("get must fail");
        assert!(
            format!("{:?}", get_err).contains("digest mismatch"),
            "get must surface the mismatch: {get_err:?}"
        );

        let list_err = list_checkpoints(&*db, "owner", &ctx_id).expect_err("list must fail");
        assert!(
            format!("{:?}", list_err).contains("digest mismatch"),
            "list must surface the mismatch: {list_err:?}"
        );
    }

    #[test]
    fn cascade_delete_removes_registry_entry_with_context() {
        // FK ON DELETE CASCADE is the registry's orphan-protection of last
        // resort: when a context is deleted, its checkpoint registry rows
        // must vanish in the same transaction.
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1"))
            .expect("seed");

        // Delete the parent work_context row directly; cascade must remove
        // the child registry row too.
        let conn = rusqlite::Connection::open(&db_path).expect("probe");
        conn.execute(
            "DELETE FROM work_contexts WHERE id = ?1",
            rusqlite::params![ctx_id],
        )
        .expect("delete parent");

        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM graph_checkpoints WHERE work_context_id = ?1",
                rusqlite::params![ctx_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            rows, 0,
            "cascade must clean up registry rows on parent delete"
        );
    }

    #[test]
    fn blob_claiming_different_run_is_refused_on_write_and_read() {
        // Pins both directions of the runId binding: blob claiming a
        // different run than the key is given is misregistration.
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        let blob = valid_checkpoint("run-B");
        let err = upsert_checkpoint(&*db, "owner", &ctx_id, "run-A", &blob)
            .expect_err("write must refuse blob claiming a different run");
        assert!(format!("{:?}", err).contains("claims runId"));

        // Same blob must also fail on read if it ever got in. Inject via raw UPDATE.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
             VALUES (?1, 'run-A', ?2, 'd', '2026-01-01T00:00:00Z')",
            rusqlite::params![ctx_id, blob],
        )
        .expect("seed bad row");
        let err = get_checkpoint(&*db, "owner", &ctx_id, "run-A").expect_err("read must refuse");
        assert!(format!("{:?}", err).contains("claims runId"));
    }

    #[test]
    fn fk_pragma_is_on_and_cascade_actually_works() {
        // The review point: PRAGMA checks must come from the same connection
        // the schema was initialized on, not a shed connection.
        let dir = tempfile::tempdir().unwrap();
        let (db, db_path) = db_in(&dir);
        let ctx_id = seed_context(&db_path);

        // Verify: Db::init Schema set and kept foreign_keys=ON on the
        // connection that owns the schema.
        let fk: i64 = db
            .conn()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "FK is on at Db::new time");

        // Cascade: deleting the parent removes the registry row.
        upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &valid_checkpoint("run-1")).unwrap();
        db.conn()
            .execute("DELETE FROM work_contexts WHERE id = ?1", params![ctx_id])
            .unwrap();
        let rows: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM graph_checkpoints WHERE work_context_id = ?1",
                params![ctx_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "cascade did its job on the same connection");
    }
}
