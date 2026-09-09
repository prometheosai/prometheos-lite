//! E6/I03 Slice B: cursorable durable event stream for work contexts.
//!
//! Locks the #132 acceptance bullet "Event consumers can reconnect and
//! resume from a stable cursor without gaps or duplication" via the new
//! read-only endpoint `GET /work-contexts/:id/events?user_id=..&after=..
//! &limit=..`.
//!
//! The cursor is the event row's durable `seq` (`INTEGER PRIMARY KEY
//! AUTOINCREMENT`): insertion-ordered, strictly monotonic, never reused
//! after deletions, and stable across VACUUM (it lives in the same SQLite
//! file via `sqlite_sequence`), so it survives AppState drops, process
//! restarts, and maintenance. Corrupt rows fail closed (typed errors, no
//! fabricated events, no panic).
//!
//! Tests in this file drive the real `AppState` + real `Db` over a
//! tempdir `db_path`, the same pattern as `api_read_model_rebuild.rs`.
//! No mocks.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn test_app_state() -> (
    std::sync::Arc<prometheos_lite::api::AppState>,
    String,
    tempfile::TempDir,
) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("api_event_cursor_test.db")
        .to_str()
        .expect("db path")
        .to_string();
    let runtime = std::sync::Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new());
    let embedding: std::sync::Arc<dyn prometheos_lite::flow::EmbeddingProvider> =
        std::sync::Arc::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        ));
    let memory_service = std::sync::Arc::new(prometheos_lite::flow::memory::MemoryService::new(
        prometheos_lite::flow::memory::MemoryDb::in_memory().expect("in-memory memory db"),
        Box::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        )),
    ));
    let state = std::sync::Arc::new(
        prometheos_lite::api::AppState::new(db_path.clone(), runtime, embedding, memory_service)
            .expect("app state"),
    );
    (state, db_path, db_dir)
}

fn rebuild_app_state(db_path: &str) -> std::sync::Arc<prometheos_lite::api::AppState> {
    let runtime = std::sync::Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new());
    let embedding: std::sync::Arc<dyn prometheos_lite::flow::EmbeddingProvider> =
        std::sync::Arc::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        ));
    let memory_service = std::sync::Arc::new(prometheos_lite::flow::memory::MemoryService::new(
        prometheos_lite::flow::memory::MemoryDb::in_memory().expect("in-memory memory db"),
        Box::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        )),
    ));
    std::sync::Arc::new(
        prometheos_lite::api::AppState::new(
            db_path.to_string(),
            runtime,
            embedding,
            memory_service,
        )
        .expect("rebuild app state"),
    )
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 256 * 1024)
        .await
        .expect("body must be collectable");
    serde_json::from_slice(&bytes).expect("body must be valid json")
}

/// Create a work context through the API and return its id.
async fn create_context(app: &axum::Router, user: &str, title: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/work-contexts")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_id": user,
                        "title": title,
                        "domain": "operations",
                        "goal": "cursor test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["id"].as_str().unwrap().to_string()
}

/// Update the context status through the API (each call appends one
/// `status_changed` event).
async fn set_status(app: &axum::Router, user: &str, id: &str, status: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{}/status?user_id={}", id, user))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "status": status }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// One page of events. Returns (status, Option<json body>).
async fn events_page(
    app: &axum::Router,
    user: &str,
    id: &str,
    after: Option<i64>,
    limit: Option<usize>,
) -> (StatusCode, serde_json::Value) {
    let mut uri = format!("/work-contexts/{}/events?user_id={}", id, user);
    if let Some(a) = after {
        uri.push_str(&format!("&after={}", a));
    }
    if let Some(l) = limit {
        uri.push_str(&format!("&limit={}", l));
    }
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    (status, body)
}

#[tokio::test]
async fn events_endpoint_returns_created_event_with_cursor() {
    let (state, _db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-1", "cursor-basic").await;

    let (status, page) = events_page(&app, "ev-user-1", &id, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let events = page["events"].as_array().expect("events array");
    assert_eq!(events.len(), 1, "create emits exactly one event");
    assert_eq!(events[0]["event_type"].as_str(), Some("context_created"));
    let cursor = events[0]["cursor"].as_i64().expect("cursor is an i64");
    assert!(cursor > 0, "seq cursor must be positive");
    assert_eq!(page["next_cursor"].as_i64().unwrap(), cursor);
}

#[tokio::test]
async fn cursor_resume_has_no_gaps_and_no_duplication() {
    let (state, _db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-2", "cursor-resume").await;

    // Full page (cursor 0): expect [context_created].
    let (_, first) = events_page(&app, "ev-user-2", &id, Some(0), None).await;
    let mut seen_ids: Vec<String> = first["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    let mut cursor = first["next_cursor"].as_i64().unwrap();
    assert_eq!(seen_ids.len(), 1);

    // Three more events (in_progress, blocked, in_progress).
    set_status(&app, "ev-user-2", &id, "in_progress").await;
    set_status(&app, "ev-user-2", &id, "blocked").await;
    set_status(&app, "ev-user-2", &id, "in_progress").await;

    // Resume from the saved cursor — must see exactly the 3 new
    // events, none repeated.
    let (_, second) = events_page(&app, "ev-user-2", &id, Some(cursor), None).await;
    let new = second["events"].as_array().unwrap();
    assert_eq!(new.len(), 3, "resume must yield exactly the new events");
    assert!(
        new.iter().all(|e| e["event_type"] == "status_changed"),
        "expected status_changed events: {new:?}"
    );
    for e in new {
        let eid = e["id"].as_str().unwrap().to_string();
        assert!(
            !seen_ids.contains(&eid),
            "cursor resume must not duplicate event {eid}"
        );
        seen_ids.push(eid);
    }
    cursor = second["next_cursor"].as_i64().unwrap();

    // No further events: resume again returns an empty page and the
    // same cursor (stable tail).
    let (_, third) = events_page(&app, "ev-user-2", &id, Some(cursor), None).await;
    assert_eq!(third["events"].as_array().unwrap().len(), 0);
    assert_eq!(third["next_cursor"].as_i64().unwrap(), cursor);

    // And the concatenated cursor walk covers every event exactly once,
    // in insertion order: compare against an unpaged full read via the
    // service-level ordering guarantee (page with limit 500 covers all).
    let (_, all) = events_page(&app, "ev-user-2", &id, Some(0), Some(500)).await;
    let all_ids: Vec<String> = all["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(all_ids, seen_ids, "cursor walk == full read, same order");
    // Cursors strictly increasing across the full stream.
    let cursors: Vec<i64> = all["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["cursor"].as_i64().unwrap())
        .collect();
    assert!(
        cursors.windows(2).all(|w| w[0] < w[1]),
        "cursors must be strictly increasing: {cursors:?}"
    );
}

#[tokio::test]
async fn cursor_survives_appstate_rebuild() {
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-3", "cursor-rebuild").await;
    set_status(&app, "ev-user-3", &id, "in_progress").await;

    let (_, page) = events_page(&app, "ev-user-3", &id, Some(0), Some(1)).await;
    assert_eq!(page["events"].as_array().unwrap().len(), 1);
    let cursor = page["next_cursor"].as_i64().unwrap();

    // Drop the entire read model; rebuild from the same db_path.
    drop(app);
    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2);

    let (_, resumed) = events_page(&app2, "ev-user-3", &id, Some(cursor), None).await;
    let events = resumed["events"].as_array().unwrap();
    assert_eq!(
        events.len(),
        1,
        "after rebuild, resuming from cursor yields the remaining event"
    );
    assert_eq!(events[0]["event_type"].as_str(), Some("status_changed"));
}

#[tokio::test]
async fn pagination_with_limit_yields_complete_unique_stream() {
    let (state, _db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-4", "cursor-page").await;
    for status in ["in_progress", "blocked", "in_progress", "blocked"] {
        set_status(&app, "ev-user-4", &id, status).await;
    }

    // Page with limit=2 until empty.
    let mut cursor = 0i64;
    let mut collected: Vec<String> = Vec::new();
    for _ in 0..10 {
        let (_, page) = events_page(&app, "ev-user-4", &id, Some(cursor), Some(2)).await;
        let events = page["events"].as_array().unwrap();
        let next = page["next_cursor"].as_i64().unwrap();
        for e in events {
            collected.push(e["id"].as_str().unwrap().to_string());
            assert!(e["cursor"].as_i64().unwrap() > cursor);
        }
        if events.is_empty() {
            break;
        }
        assert!(next > cursor, "cursor must advance on a non-empty page");
        cursor = next;
    }
    assert_eq!(collected.len(), 5, "1 create + 4 status events");
    let unique: std::collections::HashSet<_> = collected.iter().collect();
    assert_eq!(unique.len(), collected.len(), "no duplicated events");
}

#[tokio::test]
async fn events_endpoint_enforces_ownership_and_validates_cursor() {
    let (state, _db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-owner", "cursor-auth").await;

    // Missing user_id -> 400.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/work-contexts/{}/events", id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Wrong user -> 403 (same rule as every other read on this router).
    let (status, _) = events_page(&app, "ev-intruder", &id, Some(0), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unknown context -> 404.
    let (status, _) = events_page(&app, "ev-owner", "no-such-context", Some(0), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Negative cursor -> 400.
    let (status, _) = events_page(&app, "ev-owner", &id, Some(-1), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Non-integer cursor -> the Query extractor rejects with 400.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/work-contexts/{}/events?user_id=ev-owner&after=abc",
                    id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cursor_read_is_repeatable_and_events_from_cli_path_visible() {
    // The events written through the CLI-side service path
    // (WorkContextService) are the same rows the API endpoint serves:
    // one durable store, one projection.
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-6", "cursor-shared").await;

    // Write an extra event via the CLI-side service (no API involved).
    {
        let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db"));
        let svc = prometheos_lite::work::WorkContextService::new(db);
        let mut ctx = svc.get_context(&id).unwrap().expect("context");
        svc.update_status(&mut ctx, prometheos_lite::work::types::WorkStatus::Blocked)
            .expect("update status via service");
    }

    // Repeatable read: the same query twice returns identical bytes.
    let (_, first) = events_page(&app, "ev-user-6", &id, Some(0), None).await;
    let (_, second) = events_page(&app, "ev-user-6", &id, Some(0), None).await;
    assert_eq!(first, second, "cursor reads are stable snapshots");

    // The CLI-side write is visible through the API projection.
    let types: Vec<&str> = first["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["context_created", "status_changed"]);
}

#[tokio::test]
async fn cursor_survives_vacuum_without_renumbering() {
    // P1 regression: implicit rowids may be renumbered by VACUUM; the
    // explicit AUTOINCREMENT `seq` cursor must be stable across it.
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-vac", "cursor-vacuum").await;
    set_status(&app, "ev-user-vac", &id, "in_progress").await;

    let (_, before) = events_page(&app, "ev-user-vac", &id, Some(0), Some(500)).await;
    let cursors_before: Vec<i64> = before["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["cursor"].as_i64().unwrap())
        .collect();
    assert_eq!(cursors_before.len(), 2);

    // VACUUM the underlying file while no request holds the connection
    // (AppState creates per-request connections; the router holds none).
    drop(app);
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db for vacuum");
        conn.execute_batch("VACUUM;").expect("vacuum must succeed");
    }

    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2);
    let (_, after) = events_page(&app2, "ev-user-vac", &id, Some(0), Some(500)).await;
    let cursors_after: Vec<i64> = after["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["cursor"].as_i64().unwrap())
        .collect();
    assert_eq!(
        cursors_before, cursors_after,
        "seq cursors must be identical across VACUUM"
    );

    // Resume from the first cursor still yields exactly the second event.
    let (_, resumed) = events_page(&app2, "ev-user-vac", &id, Some(cursors_after[0]), None).await;
    let events = resumed["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["cursor"].as_i64().unwrap(), cursors_after[1]);
}

#[tokio::test]
async fn cursor_sequence_is_not_reused_after_delete() {
    // P1 regression: deleted implicit-rowid maxima may be reused; the
    // AUTOINCREMENT `seq` must keep advancing so resume never skips or
    // duplicates a live event.
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-del", "cursor-delete").await;
    set_status(&app, "ev-user-del", &id, "in_progress").await;
    set_status(&app, "ev-user-del", &id, "blocked").await;

    let (_, full) = events_page(&app, "ev-user-del", &id, Some(0), Some(500)).await;
    let max_seq = full["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["cursor"].as_i64().unwrap())
        .max()
        .unwrap();

    // Delete the newest event row directly, then append a fresh event.
    drop(app);
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db for delete");
        conn.execute(
            "DELETE FROM work_context_events WHERE work_context_id = ?1 AND seq = ?2",
            rusqlite::params![id, max_seq],
        )
        .expect("delete must succeed");
    }
    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2.clone());
    set_status(&app2, "ev-user-del", &id, "in_progress").await;

    let (_, resumed) = events_page(&app2, "ev-user-del", &id, Some(0), Some(500)).await;
    let cursors: Vec<i64> = resumed["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["cursor"].as_i64().unwrap())
        .collect();
    assert!(
        cursors.windows(2).all(|w| w[0] < w[1]),
        "cursors must stay strictly increasing after delete+insert: {cursors:?}"
    );
    let new_max = *cursors.iter().max().unwrap();
    assert!(
        new_max > max_seq,
        "new seq ({new_max}) must exceed deleted max ({max_seq}), never reuse"
    );
}

#[tokio::test]
async fn corrupt_event_data_fails_closed_without_fabrication() {
    // P1 regression: `unwrap_or_default()` silently replaced corrupt JSON
    // with `null`. The read must now error fail-closed instead.
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-corrupt", "cursor-corrupt").await;
    drop(app);

    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db to corrupt");
        conn.execute(
            "UPDATE work_context_events SET data = 'not-json{{{' WHERE work_context_id = ?1",
            rusqlite::params![id],
        )
        .expect("corruption write must succeed");
    }

    // Service-level read fails closed (no fabricated `null` payload).
    {
        let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db"));
        let svc = prometheos_lite::work::WorkContextService::new(db);
        let err = svc
            .list_events_after(&id, Some(0), 50)
            .expect_err("corrupt data must surface as an error");
        assert!(
            !err.to_string().is_empty(),
            "error must carry a diagnostic message"
        );
    }

    // API projection fails closed with 500, not a fabricated event.
    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2);
    let (status, _) = events_page(&app2, "ev-user-corrupt", &id, Some(0), None).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "corrupt row must not return 200 with fabricated data"
    );
}

#[tokio::test]
async fn malformed_timestamp_fails_closed_without_panic() {
    // P1 regression: `DateTime::parse_from_rfc3339(...).unwrap()` could
    // panic the request path. Malformed timestamps must error cleanly.
    let (state, db_path, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "ev-user-ts", "cursor-timestamp").await;
    drop(app);

    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db to corrupt");
        conn.execute(
            "UPDATE work_context_events SET created_at = 'not-a-timestamp' WHERE work_context_id = ?1",
            rusqlite::params![id],
        )
        .expect("corruption write must succeed");
    }

    {
        let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db"));
        let svc = prometheos_lite::work::WorkContextService::new(db);
        svc.list_events_after(&id, Some(0), 50)
            .expect_err("malformed timestamp must surface as an error, not panic");
    }

    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2);
    let (status, _) = events_page(&app2, "ev-user-ts", &id, Some(0), None).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "malformed timestamp must not panic or return fabricated data"
    );
}

#[test]
fn legacy_event_table_migrates_to_durable_seq_in_insertion_order() {
    // P1 regression: databases created before the fix have no `seq`
    // column. Opening them via `Db` must backfill an AUTOINCREMENT `seq`
    // in original insertion (rowid) order, and the result must survive
    // VACUUM unchanged.
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("legacy_events_test.db")
        .to_str()
        .expect("db path")
        .to_string();

    // Build the legacy schema by hand (pre-fix shape: TEXT PRIMARY KEY,
    // no `seq`), with three events inserted in a known order.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            "CREATE TABLE work_context_events (
                id TEXT PRIMARY KEY,
                work_context_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .expect("legacy table");
        for (i, (eid, etype)) in [
            ("evt-legacy-1", "context_created"),
            ("evt-legacy-2", "status_changed"),
            ("evt-legacy-3", "status_changed"),
        ]
        .iter()
        .enumerate()
        {
            conn.execute(
                "INSERT INTO work_context_events
                    (id, work_context_id, event_type, data, created_at)
                 VALUES (?1, 'ctx-legacy', ?2, '{\"n\":0}', ?3)",
                rusqlite::params![eid, etype, format!("2026-09-0{}T00:00:00+00:00", i + 1)],
            )
            .expect("legacy row");
        }
    }

    // Opening via `Db` runs the copy-and-rename migration.
    let cursors: Vec<(i64, String)> = {
        let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("migrated db"));
        let svc = prometheos_lite::work::WorkContextService::new(db);
        svc.list_events_after("ctx-legacy", Some(0), 50)
            .expect("migrated read must succeed")
            .into_iter()
            .map(|(seq, ev)| (seq, ev.id))
            .collect()
    };
    assert_eq!(
        cursors
            .iter()
            .map(|(_, id)| id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt-legacy-1", "evt-legacy-2", "evt-legacy-3"],
        "migration must preserve insertion order"
    );
    let seqs: Vec<i64> = cursors.iter().map(|(s, _)| *s).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]) && seqs[0] > 0,
        "migrated seqs must be positive and strictly increasing: {seqs:?}"
    );

    // Stable across VACUUM.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db for vacuum");
        conn.execute_batch("VACUUM;").expect("vacuum must succeed");
    }
    let after: Vec<(i64, String)> = {
        let db =
            std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db after vacuum"));
        let svc = prometheos_lite::work::WorkContextService::new(db);
        svc.list_events_after("ctx-legacy", Some(0), 50)
            .expect("post-vacuum read must succeed")
            .into_iter()
            .map(|(seq, ev)| (seq, ev.id))
            .collect()
    };
    assert_eq!(cursors, after, "seqs must be identical across VACUUM");
}
