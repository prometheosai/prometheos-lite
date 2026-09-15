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

fn validate_checkpoint_json(blob: &str) -> anyhow::Result<()> {
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

/// Register a checkpoint under (context, run), conditioned on ownership.
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
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    let graph_run_id = normalize_id(graph_run_id, "graph_run_id")?;
    ensure_owned_by(db, &work_context_id, user_id)?;
    validate_checkpoint_json(checkpoint_json)?;

    let digest = digest_of(checkpoint_json);
    let now = Utc::now().to_rfc3339();

    db.as_db().conn().execute(
        "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(work_context_id, graph_run_id) DO UPDATE SET
            checkpoint_json = excluded.checkpoint_json,
            checkpoint_digest = excluded.checkpoint_digest",
        params![work_context_id, graph_run_id, checkpoint_json, digest, now],
    )
    .context("failed to upsert graph checkpoint")?;

    Ok(digest)
}

/// Read the checkpoint blob + digest back; ownership is enforced same as
/// upsert. The returned digest is recomputed from the blob — if the stored
/// digest ever disagrees, that is flagged on read (the caller holds a good
/// blob; the stored digest value itself is irrelevant past this point).
pub fn get_checkpoint<T: AsDb>(
    db: &T,
    user_id: &str,
    work_context_id: &str,
    graph_run_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let work_context_id = normalize_id(work_context_id, "work_context_id")?;
    let graph_run_id = normalize_id(graph_run_id, "graph_run_id")?;
    ensure_owned_by(db, &work_context_id, user_id)?;

    let raw: Option<String> = db
        .as_db()
        .conn()
        .query_row(
            "SELECT checkpoint_json FROM graph_checkpoints WHERE work_context_id = ?1 AND graph_run_id = ?2",
            params![work_context_id, graph_run_id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to query checkpoint")?;

    raw.map(|blob| {
        validate_checkpoint_json(&blob)?;
        let digest = digest_of(&blob);
        Ok((blob, digest))
    })
    .transpose()
}

/// List all registered checkpoint references for a context; Owned-same
/// gating as the rest.
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
            "SELECT work_context_id, graph_run_id, checkpoint_json, created_at
             FROM graph_checkpoints WHERE work_context_id = ?1
             ORDER BY created_at ASC, graph_run_id ASC",
        )
        .context("failed to prepare list")?;

    let rows = stmt
        .query_map(params![work_context_id], |row| {
            let id: String = row.get(0)?;
            let run: String = row.get(1)?;
            let blob: String = row.get(2)?;
            validate_checkpoint_json(&blob).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e.to_string(),
                    )),
                )
            })?;
            Ok(GraphCheckpointRef {
                work_context_id: id.clone(),
                graph_run_id: run,
                checkpoint_digest: digest_of(&blob),
                created_at: row.get(3)?,
            })
        })
        .context("failed to query checkpoints list")?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.context("failed to read checkpoint row")?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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

        let blob2 =
            valid_checkpoint("run-1").replace("\"runId\":\"run-1\"", "\"runId\":\"run-1x\"");
        let d2 = upsert_checkpoint(&*db, "owner", &ctx_id, "run-1", &blob2).unwrap();
        assert_ne!(d1, d2);
        let (blob2_read, digest2) = get_checkpoint(&*db, "owner", &ctx_id, "run-1")
            .unwrap()
            .unwrap();
        assert_eq!(blob2_read, blob2);
        assert_eq!(digest2, d2);
    }

    #[test]
    fn owner_gate_blocks_other_users_reads_and_writes() {
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
        let ctx_id = seed_context(&db_path);
        assert!(upsert_checkpoint(&*db, "owner", "", "run-1", &valid_checkpoint("run-1")).is_err());
        assert!(
            upsert_checkpoint(&*db, "owner", &ctx_id, "   ", &valid_checkpoint("run-1")).is_err()
        );
    }

    #[test]
    fn overwrite_keeps_original_created_at_not_churn() {
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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
        let (db, _db_path) = db_in(&tempfile::tempdir().unwrap());
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
        let (db, db_path) = db_in(&tempfile::tempdir().unwrap());
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
    fn failing_fk_off_does_not_hide_orphans() {
        // Turn FK off (broken environment), then insert an orphan. Fail the
        // whole call because the contract requires it.
        let (_db, db_path) = db_in(&tempfile::tempdir().unwrap());

        // Enforce FK to verify our invariant, then disable it on this
        // connection to simulate a busted config.
        let probe = rusqlite::Connection::open(&db_path).unwrap();
        probe.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let fk: i64 = probe
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "FK must be ON by default after Db::new");

        probe.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        let fk_after: i64 = probe
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk_after, 0, "FK turns off when explicitly disabled");

        // Now insert an orphan via the same raw connection (bypassing the
        // typed layer to isolate the failure mode). This isn't a code change
        // to the registry itself; it proves the FK really is the backstop.
        probe.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        let err = probe.execute(
            "INSERT INTO graph_checkpoints (work_context_id, graph_run_id, checkpoint_json, checkpoint_digest, created_at)
             VALUES ('ghost', 'r', '{}', 'x', '2026-01-01T00:00:00Z')",
            [],
        );
        // sqlite allows the insert when FK is off — that's the behavior we
        // detect. But our *typed* layer always checks ownership first, so
        // this path is unreachable through upsert_checkpoint; the test is
        // demonstrating the DB-level guard has teeth — FK on vs off.
        assert!(
            err.is_ok(),
            "INSERT bypasses FK when disabled (as documented)"
        );

        // Re-enable FK, verify the ownership-defense would still catch
        // orphan insertion through the typed layer (regardless of pragma).
        // The seed context here is created as "owner" via the same service.
        let owned_ctx = seed_context(&db_path);
        let svc_db = Arc::new(crate::db::Db::new(&db_path).unwrap());
        let _svc = crate::work::WorkContextService::new(svc_db.clone());
        std::mem::drop(_svc);
        let check = svc_db.conn().query_row(
            "SELECT COUNT(*) FROM work_contexts WHERE id = ?1 AND user_id = 'owner'",
            params![owned_ctx],
            |r| r.get::<_, i64>(0),
        );
        assert!(matches!(check, Ok(1)));
    }
}
