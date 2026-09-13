//! E6/I03 Slice C: governed WorkContext cancellation endpoint (#132).
//!
//! Pins `POST /work-contexts/:id/cancel` at the HTTP frontier: ownership
//! checks, idempotent re-cancellation, terminal-state fail-closed behavior,
//! durable event recording, restart survivability, and the terminal-state
//! refusal of run/continue/harness endpoints. The decision half of the
//! slice is intentionally deferred — see the slice-prerequisite notes in
//! the PR body.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prometheos_lite::api::AppState;
use prometheos_lite::work::WorkContextService;
use prometheos_lite::work::types::WorkStatus;
use tower::ServiceExt;

fn test_app_state() -> (std::sync::Arc<AppState>, String, tempfile::TempDir) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("api_cancel_test.db")
        .to_str()
        .expect("db path")
        .to_string();
    let runtime = std::sync::Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new());
    let local_embed = || {
        prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        )
    };
    let embedding: std::sync::Arc<dyn prometheos_lite::flow::EmbeddingProvider> =
        std::sync::Arc::new(local_embed());
    let memory_service = std::sync::Arc::new(prometheos_lite::flow::memory::MemoryService::new(
        prometheos_lite::flow::memory::MemoryDb::in_memory().expect("in-memory memory db"),
        Box::new(local_embed()),
    ));
    let state = std::sync::Arc::new(
        AppState::new(db_path.clone(), runtime, embedding, memory_service).expect("app state"),
    );
    (state, db_path, db_dir)
}

fn rebuild_app_state(db_path: &str) -> std::sync::Arc<AppState> {
    let (state, _, _) = test_app_state_at(db_path.to_string());
    state
}

fn test_app_state_at(db_path: String) -> (std::sync::Arc<AppState>, String, tempfile::TempDir) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
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
        AppState::new(db_path, runtime, embedding, memory_service).expect("app state"),
    );
    (state, String::new(), db_dir)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 256 * 1024)
        .await
        .expect("body collectable");
    serde_json::from_slice(&bytes).expect("valid json")
}

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
                        "goal": "cancel test"
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

async fn cancel(
    app: &axum::Router,
    user: &str,
    id: &str,
    reason: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/cancel?user_id={user}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "reason": reason }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    (status, body)
}

#[tokio::test]
async fn cancel_flips_status_and_writes_durable_event() {
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "owner", "cancel-basic").await;

    let (status, body) = cancel(&app, "owner", &id, "operator requested stop").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str(), Some("Cancelled"));

    // Event persisted in the durable event stream (via the cursorable
    // events endpoint, same-store projection).
    let (state2, _, _) = test_app_state();
    let _ = (state2,); // fully independent state for clarity
    let events_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/work-contexts/{id}/events?user_id=owner&after=0&limit=500"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let events_json = body_json(events_resp).await;
    let events = events_json["events"].as_array().unwrap();
    let cancel_event = events
        .iter()
        .find(|e| e["event_type"] == "context_cancelled")
        .expect("context_cancelled event recorded");
    assert_eq!(
        cancel_event["data"]["reason"].as_str(),
        Some("operator requested stop")
    );
    assert_eq!(cancel_event["data"]["to"].as_str(), Some("Cancelled"));

    // Rebuild read-model from the same on-disk db and confirm durability.
    drop(app);
    let state3 = rebuild_app_state(&db_path);
    let app3 = prometheos_lite::api::router::create_router(state3);
    let resp = app3
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/work-contexts/{id}?user_id=owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body3 = body_json(resp).await;
    assert_eq!(body3["status"].as_str(), Some("Cancelled"));
}

#[tokio::test]
async fn cancel_is_idempotent() {
    let (state, _db, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "owner", "cancel-idem").await;

    let (s1, _) = cancel(&app, "owner", &id, "first").await;
    assert_eq!(s1, StatusCode::OK);

    // Second cancel: still success, NO duplicate context_cancelled event.
    let (s2, _) = cancel(&app, "owner", &id, "second").await;
    assert_eq!(s2, StatusCode::OK);

    let events_resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/work-contexts/{id}/events?user_id=owner&after=0&limit=500"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let events = body_json(events_resp).await;
    let cancels = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event_type"] == "context_cancelled")
        .count();
    assert_eq!(cancels, 1, "idempotent cancel must not duplicate the event");
}

#[tokio::test]
async fn cancel_requires_auth_and_reason() {
    let (state, _db, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let owner = "owner";
    let id = create_context(&app, owner, "cancel-auth").await;

    // Wrong user → 403.
    let (st, _) = cancel(&app, "intruder", &id, "nope").await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Unknown context → 404.
    let (st, _) = cancel(&app, owner, "no-such-context", "nope").await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Missing user_id → 400.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/cancel"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"reason":"x"}"#.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Empty reason → 400.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/cancel?user_id={owner}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"reason":"   "}"#.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cancel_refuses_terminal_states() {
    let (state, _db, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let owner = "owner";
    let id = create_context(&app, owner, "cancel-terminal").await;

    // Cancel works once (200, idempotent).
    let (s1, _) = cancel(&app, owner, &id, "stop").await;
    assert_eq!(s1, StatusCode::OK);
    let (s2, _) = cancel(&app, owner, &id, "again").await;
    assert_eq!(s2, StatusCode::OK);

    // Try to drive the now-cancelled context forward via /status.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/status?user_id={owner}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"in_progress"}"#.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn cancelled_context_refuses_run_endpoints() {
    let (state, _db, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let owner = "owner";
    let id = create_context(&app, owner, "cancel-gates").await;

    let (st, _) = cancel(&app, owner, &id, "stop").await;
    assert_eq!(st, StatusCode::OK);

    // continue → refused.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/continue?user_id={owner}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // run-until-complete → refused.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/work-contexts/{id}/run-until-complete?user_id={owner}"
                ))
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // harness run → refused.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/harness/run?user_id={owner}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"repo_root": ".", "mode": "Review"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn cancel_survives_appstate_rebuild() {
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "owner", "cancel-restart").await;
    let (st, _) = cancel(&app, "owner", &id, "restart carve").await;
    assert_eq!(st, StatusCode::OK);

    drop(app);

    let state2 = rebuild_app_state(&db_path);
    let app2 = prometheos_lite::api::router::create_router(state2);

    // Both /work-contexts/:id and the event stream reflect the cancel.
    let resp = app2
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/work-contexts/{id}?user_id=owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["status"].as_str(), Some("Cancelled"));

    let resp = app2
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/work-contexts/{id}/events?user_id=owner&after=0&limit=500"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let events = body_json(resp).await;
    let has_cancel = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event_type"] == "context_cancelled");
    assert!(has_cancel, "context_cancelled event survives restart");
}

#[tokio::test]
async fn cancel_works_at_service_layer_as_well() {
    // Slice C parity: the same durable mechanism is reachable from the
    // service tier used by the CLI, not just the HTTP route.
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    drop(app);

    let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db"));
    let svc = WorkContextService::new(db);
    let mut ctx = svc
        .create_context(
            "owner".into(),
            "service-level cancel".to_string(),
            prometheos_lite::work::types::WorkDomain::Operations,
            "g".to_string(),
        )
        .unwrap();

    svc.cancel_context(&mut ctx, "cli cancel").expect("cancel");
    assert_eq!(ctx.status, WorkStatus::Cancelled);

    // Idempotent on the service tier as well.
    svc.cancel_context(&mut ctx, "again").expect("re-cancel");

    // Events: exactly one context_cancelled row.
    let events = svc.list_events_after(&ctx.id, None, 500).unwrap();
    let n = events
        .iter()
        .filter(|(_, e)| e.event_type == "context_cancelled")
        .count();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// Round-2 P1 repairs: atomicity + concurrency + shared-boundary enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_is_atomic_against_event_insert_failure() {
    // Force the event write inside the transaction to fail and prove the
    // status column does NOT flip. Minimal-intrusion injection: install a
    // trigger that aborts just this event type, run cancel, verify status
    // unchanged, then drop the trigger.
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let id = create_context(&app, "owner", "cancel-atomic").await;

    let conn = rusqlite::Connection::open(&db_path).expect("db");
    conn.execute_batch(
        "CREATE TRIGGER force_cancel_event_fail
         BEFORE INSERT ON work_context_events
         WHEN NEW.event_type = 'context_cancelled'
         BEGIN SELECT RAISE(ABORT, 'forced-failure'); END;",
    )
    .expect("install injection trigger");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/cancel?user_id=owner"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"reason": "forced-failure"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    conn.execute_batch("DROP TRIGGER force_cancel_event_fail;")
        .expect("remove trigger");
    let status: String = conn
        .query_row(
            "SELECT status FROM work_contexts WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("row still present");
    assert_eq!(status, "\"Draft\"", "status rollback: not Cancelled");
}

#[tokio::test]
async fn concurrent_cancels_produce_single_event() {
    // Two parallel HTTP cancels against the same context: exactly one must
    // transition (win the conditional UPDATE + insert) and emit the event;
    // the loser either fails closed (terminal state error) or idempotently
    // succeeds, but MUST NOT produce a second context_cancelled event row.
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let id = create_context(&app, "owner", "cancel-race").await;

    let uri = format!("/work-contexts/{id}/cancel?user_id=owner");
    let body = serde_json::json!({"reason": "race"}).to_string();

    let mk = |app: &axum::Router, tag: &str| {
        let app = app.clone();
        let uri = uri.clone();
        let body = body.clone();
        let tag = tag.to_string();
        tokio::spawn(async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            (tag, resp.status())
        })
    };

    let (a, b) = tokio::join!(mk(&app, "A"), mk(&app, "B"));
    let (_, sa) = a.unwrap();
    let (_, sb) = b.unwrap();
    // Both results are admissible (200 winner, 200 idempotent, or 409 loss)
    // but only ONE db row must exist at the end.
    assert!(matches!(
        sa,
        StatusCode::OK | StatusCode::CONFLICT | StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert!(matches!(
        sb,
        StatusCode::OK | StatusCode::CONFLICT | StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert!(
        sa != StatusCode::INTERNAL_SERVER_ERROR || sb != StatusCode::INTERNAL_SERVER_ERROR,
        "at least one cancel must succeed"
    );

    let conn = rusqlite::Connection::open(&db_path).expect("db");
    let events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM work_context_events
             WHERE work_context_id = ?1 AND event_type = 'context_cancelled'",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        events, 1,
        "exactly one context_cancelled event regardless of ordering"
    );
}

#[tokio::test]
async fn service_layer_cancel_gate_bypassing_http_also_refuses_terminal_states() {
    // The execution boundary must not advance a cancelled context even when
    // the HTTP handler is bypassed (service/orchestrator path).
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let id = create_context(&app, "owner", "cancel-service-boundary").await;
    let (st, _) = cancel(&app, "owner", &id, "stop").await;
    assert_eq!(st, StatusCode::OK);

    // WorkExecutionService path (used by CLI and orchestrator):
    let db = std::sync::Arc::new(prometheos_lite::db::Db::new(&db_path).expect("db"));
    let svc = std::sync::Arc::new(WorkContextService::new(db.clone()));
    let execution = prometheos_lite::work::WorkExecutionService::new(
        svc,
        std::sync::Arc::new(
            prometheos_lite::flow::execution_service::FlowExecutionService::new(
                std::sync::Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new()),
            )
            .expect("flow exec"),
        ),
    );
    let err = execution
        .continue_context(&id)
        .await
        .expect_err("cancelled context must refuse continuation at the service boundary");
    assert!(
        err.to_string().contains("cancelled"),
        "error must cite cancellation: {err}"
    );
}

#[tokio::test]
async fn refused_states_report_full_set() {
    // Completed / Failed / Archived all refused. We can't drive contexts to
    // those states from the API alone (Completed requires harness evidence,
    // Archived requires manual status), so exercise the service tool directly.
    let (state, db_path, _tmp) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let id = create_context(&app, "owner", "cancel-terminal-full").await;

    let db_conn = rusqlite::Connection::open(&db_path).expect("db");
    for terminal in ["\"Completed\"", "\"Failed\"", "\"Archived\""] {
        db_conn
            .execute(
                "UPDATE work_contexts SET status = ?1 WHERE id = ?2",
                rusqlite::params![terminal, id],
            )
            .expect("seed terminal status");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/work-contexts/{id}/cancel?user_id=owner"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reason":"too late"}"#.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "terminal state {terminal} must refuse cancel"
        );
    }
}
