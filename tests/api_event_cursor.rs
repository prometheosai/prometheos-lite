//! E6/I03 Slice B: cursorable durable event stream for work contexts.
//!
//! Locks the #132 acceptance bullet "Event consumers can reconnect and
//! resume from a stable cursor without gaps or duplication" via the new
//! read-only endpoint `GET /work-contexts/:id/events?user_id=..&after=..
//! &limit=..`.
//!
//! The cursor is the event row's SQLite rowid: insertion-ordered,
//! strictly monotonic, durable (it lives in the same SQLite file), so
//! it survives AppState drops and process restarts. No VACUUM exists
//! anywhere in the codebase (see the invariant note in
//! `src/db/repository/work_context_events.rs`).
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
    assert!(cursor > 0, "rowid cursor must be positive");
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
