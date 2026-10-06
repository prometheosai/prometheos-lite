//! #132 Slice 2: portable observation endpoints over the Slice 1B SOMA
//! WorkEvent projections.
//!
//! Locks the approved-plan contract for the three GET-only endpoints:
//! byte-deterministic canonical-envelope bodies, `X-Next-Cursor`
//! reconnectable paging with no gaps or duplication, `ETag` = projection
//! digest, ownership scoping identical to every other router read, the
//! typed run-key wire resources, and the full fail-closed error mapping
//! (400 validation -> 403/404 ownership -> 500 gate failures with the
//! gate's own text -> 422 unprojectable records -> 404 unknown run key).
//!
//! Tests drive the real `AppState` + real `Db` over a tempdir `db_path`
//! (the established `api_event_cursor.rs` pattern): contexts are created
//! through the API (provenance-complete journals for free); out-of-band
//! journal manipulations use the established trigger-suspension
//! technique against the same durable file.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prometheos_lite::db::Db;
use prometheos_lite::db::repository::work_context_events::record_event_conn;
use prometheos_lite::work::event::WorkContextEvent;
use prometheos_lite::work::provenance::JournalContext;
use prometheos_lite::work::soma_projection::WorkEventStreamPage;
use prometheos_lite::work::types::{ApprovalPolicy, AutonomyLevel};
use prometheos_lite::workflow::projection::VersionedProjectionEnvelope;
use std::sync::Arc;
use tower::ServiceExt;

fn test_app_state() -> (
    Arc<prometheos_lite::api::AppState>,
    String,
    tempfile::TempDir,
) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("api_work_event_projection_test.db")
        .to_str()
        .expect("db path")
        .to_string();
    let runtime = Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new());
    let embedding: Arc<dyn prometheos_lite::flow::EmbeddingProvider> =
        Arc::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        ));
    let memory_service = Arc::new(prometheos_lite::flow::memory::MemoryService::new(
        prometheos_lite::flow::memory::MemoryDb::in_memory().expect("in-memory memory db"),
        Box::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        )),
    ));
    let state = Arc::new(
        prometheos_lite::api::AppState::new(db_path.clone(), runtime, embedding, memory_service)
            .expect("app state"),
    );
    (state, db_path, db_dir)
}

fn test_router(state: &Arc<prometheos_lite::api::AppState>) -> axum::Router {
    prometheos_lite::api::router::create_router(state.clone())
}

fn authority() -> prometheos_lite::work::provenance::AuthorityRecord {
    JournalContext::work_authority(AutonomyLevel::Review, ApprovalPolicy::Auto)
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body must be collectable")
        .to_vec()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = body_bytes(resp).await;
    serde_json::from_slice(&bytes).expect("body must be valid json")
}

async fn get(app: &axum::Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get_with_header(
    app: &axum::Router,
    uri: &str,
    header: &str,
    value: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header(header, value)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn post(app: &axum::Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Create a work context through the API (a provenance-complete
/// `context_created` event) and drive one `status_changed` event.
async fn driven_context(app: &axum::Router, user: &str) -> String {
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
                        "title": "Slice 2 observation",
                        "domain": "operations",
                        "goal": "observation test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{id}/status?user_id={user}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "status": "in_progress" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    id
}

/// The established out-of-band technique against the durable file.
fn with_triggers_suspended<T>(db: &Db, f: impl FnOnce() -> T) -> T {
    db.conn()
        .execute("DROP TRIGGER IF EXISTS work_context_events_append_only", [])
        .unwrap();
    db.conn()
        .execute(
            "DROP TRIGGER IF EXISTS work_context_events_provenance_required",
            [],
        )
        .unwrap();
    let out = f();
    db.conn()
        .execute(
            "CREATE TRIGGER work_context_events_append_only
             BEFORE UPDATE ON work_context_events
             BEGIN
                 SELECT RAISE(ABORT, 'journal rows are append-only');
             END",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "CREATE TRIGGER IF NOT EXISTS work_context_events_provenance_required
             BEFORE INSERT ON work_context_events
             WHEN NEW.provenance_json IS NULL
               OR NEW.source_digest IS NULL
               OR NEW.run_id IS NULL
               OR NEW.correlation_id IS NULL
             BEGIN
                 SELECT RAISE(ABORT, 'journal insert requires complete provenance');
             END",
            [],
        )
        .unwrap();
    out
}

/// Record one journal event directly against the durable file (the API
/// created the context row, so the foreign key holds).
fn record_direct(
    db: &Db,
    context_id: &str,
    id: &str,
    event_type: &str,
    envelope: &prometheos_lite::work::provenance::ProvenanceEnvelope,
    created_at: &str,
) {
    let event = WorkContextEvent {
        id: id.to_string(),
        work_context_id: context_id.to_string(),
        event_type: event_type.to_string(),
        data: serde_json::json!({ "from": "Draft", "to": "InProgress" }),
        created_at: chrono::DateTime::parse_from_rfc3339(created_at)
            .unwrap()
            .with_timezone(&chrono::Utc),
    };
    record_event_conn(db.conn(), &event, envelope).unwrap();
}

fn fixed_event(id: &str, context_id: &str, event_type: &str, created_at: &str) -> WorkContextEvent {
    WorkContextEvent {
        id: id.to_string(),
        work_context_id: context_id.to_string(),
        event_type: event_type.to_string(),
        data: serde_json::json!({ "note": "slice-2" }),
        created_at: chrono::DateTime::parse_from_rfc3339(created_at)
            .unwrap()
            .with_timezone(&chrono::Utc),
    }
}

// --- Ownership ------------------------------------------------------------

#[tokio::test]
async fn unknown_context_is_404() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let resp = get(
        &app,
        "/work-contexts/00000000-0000-0000-0000-000000000000/work-events?user_id=u1",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wrong_user_is_403() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    for uri in [
        format!("/work-contexts/{id}/work-events?user_id=intruder"),
        format!("/work-contexts/{id}/work-event-runs?user_id=intruder"),
        format!("/work-contexts/{id}/work-event-runs/request/req-1?user_id=intruder"),
    ] {
        let resp = get(&app, &uri).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{uri}");
    }
}

#[tokio::test]
async fn missing_user_id_is_400() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    for uri in [
        format!("/work-contexts/{id}/work-events"),
        format!("/work-contexts/{id}/work-event-runs"),
        format!("/work-contexts/{id}/work-event-runs/request/req-1"),
    ] {
        let resp = get(&app, &uri).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}

// --- Cursor stability -------------------------------------------------------

#[tokio::test]
async fn http_paging_has_no_gaps_or_duplication() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await; // 2 events

    let mut seen: Vec<String> = Vec::new();
    let mut uri = format!("/work-contexts/{id}/work-events?user_id=owner&limit=1");
    let mut pages = 0;
    loop {
        let resp = get(&app, &uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // The continuation cursor is ALWAYS present (review P1b);
        // paging terminates on X-More-Available, not on cursor absence.
        let next_cursor = resp
            .headers()
            .get("x-next-cursor")
            .and_then(|v| v.to_str().ok())
            .expect("the continuation cursor is always present")
            .to_string();
        let more = resp
            .headers()
            .get("x-more-available")
            .and_then(|v| v.to_str().ok())
            .expect("x-more-available is always present")
            .to_string();
        let bytes = body_bytes(resp).await;
        let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
            serde_json::from_slice(&bytes).expect("canonical bytes parse as the envelope");
        for event in &envelope.payload.events {
            seen.push(event.id.clone());
        }
        pages += 1;
        if more != "true" {
            break;
        }
        uri = format!("/work-contexts/{id}/work-events?user_id=owner&limit=1&after={next_cursor}");
        assert!(pages < 10, "paging must terminate");
    }
    assert_eq!(pages, 2, "2 events with limit 1 -> exactly 2 pages");
    assert_eq!(seen.len(), 2);
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 2, "no gaps, no duplication across HTTP pages");
}

#[tokio::test]
async fn negative_after_is_400() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner&after=-1"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn limit_is_clamped_not_rejected() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    // limit=0 clamps to 1 (never a rejection, never an unbounded page).
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner&limit=0"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(envelope.payload.events.len(), 1);
}

// --- Byte determinism -------------------------------------------------------

#[tokio::test]
async fn response_body_is_the_canonical_envelope_bytes() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let wire = body_bytes(resp).await;

    // Independently compute the SAME projection straight from the
    // durable file: the wire bytes must be byte-identical.
    let db = Db::new(&db_path).unwrap();
    let page = prometheos_lite::work::soma_projection::project_page(&db, &id, 0, 500).unwrap();
    assert_eq!(wire, page.envelope.canonical_bytes().unwrap());
}

#[tokio::test]
async fn etag_identifies_the_complete_response_representation() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let uri = format!("/work-contexts/{id}/work-events?user_id=owner");

    let first = get(&app, &uri).await;
    let first_etag = first
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .to_string();
    let first_cursor = first
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .expect("cursor present")
        .to_string();
    let first_body = body_bytes(first).await;

    // The ETag is the COMPLETE-REPRESENTATION validator: the canonical
    // digest of {body digest, nextCursor, moreAvailable} — bound to the
    // paging metadata, not the payload-only projectionDigest and not
    // the body alone (see the cache/paging regression below).
    let binding = serde_json::json!({
        "body": prometheos_lite::workflow::soma::canonical::sha256_hex(&first_body),
        "nextCursor": first_cursor.parse::<i64>().unwrap(),
        "moreAvailable": false,
    });
    assert_eq!(
        first_etag,
        format!(
            "\"{}\"",
            prometheos_lite::workflow::soma::canonical::try_canonical_digest(&binding).unwrap()
        ),
        "the stream ETag binds body + paging metadata"
    );

    // Review P1a regression: a LEXICAL source-row change (same parsed
    // value — the read gate passes, the semantic payload is IDENTICAL,
    // so the projectionDigest is unchanged) changes sourceDigest and
    // therefore the response bytes — and the ETag MUST change with
    // them. The old payload-only ETag would have stayed identical.
    let db = Db::new(&db_path).unwrap();
    with_triggers_suspended(&db, || {
        db.conn()
            .execute(
                "UPDATE work_context_events
                 SET data = '{ \"from\" : \"Draft\" , \"to\" : \"InProgress\" }'
                 WHERE work_context_id = ?1 AND event_type = 'status_changed'",
                rusqlite::params![id],
            )
            .unwrap();
    });
    drop(db);

    let second = get(&app, &uri).await;
    let second_etag = second
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .to_string();
    let second_body = body_bytes(second).await;

    let first_env: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&first_body).unwrap();
    let second_env: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&second_body).unwrap();
    // The semantic payload is unchanged (same projectionDigest)…
    assert_eq!(
        first_env.projection_digest, second_env.projection_digest,
        "the re-lex preserves the semantic payload"
    );
    // …but sourceDigest, the body bytes, and therefore the ETag differ.
    assert_ne!(first_env.source_digest, second_env.source_digest);
    assert_ne!(first_body, second_body);
    assert_ne!(
        first_etag, second_etag,
        "different canonical bodies must always produce different ETags"
    );
}

#[tokio::test]
async fn if_none_match_revalidates() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let uri = format!("/work-contexts/{id}/work-events?user_id=owner");

    let resp = get(&app, &uri).await;
    let etag = resp
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .to_string();
    drop(resp);

    // A matching If-None-Match yields 304 Not Modified (empty body,
    // ETag set).
    let resp = get_with_header(&app, &uri, "if-none-match", &etag).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::ETAG)
            .and_then(|v| v.to_str().ok()),
        Some(etag.as_str())
    );
    assert!(body_bytes(resp).await.is_empty());

    // A stale If-None-Match yields the full 200 representation.
    let stale = format!("\"{}\"", "0".repeat(64));
    let resp = get_with_header(&app, &uri, "if-none-match", &stale).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!body_bytes(resp).await.is_empty());

    // `If-None-Match: *` also revalidates to 304 for an existing
    // representation.
    let resp = get_with_header(&app, &uri, "if-none-match", "*").await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn if_none_match_parses_weak_and_multi_tags() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let uri = format!("/work-contexts/{id}/work-events?user_id=owner");

    let resp = get(&app, &uri).await;
    let etag = resp
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .to_string();
    drop(resp);

    // A WEAK tag matches (RFC 9110 list syntax, W/ prefix stripped).
    let weak = format!("W/{etag}");
    let resp = get_with_header(&app, &uri, "if-none-match", &weak).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);

    // A comma-separated list: any matching entry revalidates.
    let stale = format!("\"{}\"", "0".repeat(64));
    let list = format!("{stale}, {etag}");
    let resp = get_with_header(&app, &uri, "if-none-match", &list).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);

    // A list with NO matching entry yields the full representation.
    let list = format!("{stale}, W/{stale}");
    let resp = get_with_header(&app, &uri, "if-none-match", &list).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn revalidation_never_preserves_stale_exhaustion() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await; // 2 events (seq 1..2)

    // A FULL page that exhausts the stream at cursor 2: body B,
    // more=false, ETag T. (after=1, limit=1 -> exactly the last event.)
    let uri = format!("/work-contexts/{id}/work-events?user_id=owner&after=1&limit=1");
    let resp = get(&app, &uri).await;
    let etag = resp
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .to_string();
    assert_eq!(
        resp.headers()
            .get("x-more-available")
            .and_then(|v| v.to_str().ok())
            .unwrap(),
        "false"
    );
    let exhausted_body = body_bytes(resp).await;

    // A NEW event arrives BEYOND the page. The page's BODY is
    // byte-identical (after=1, limit=1 still returns the same single
    // event) — only the paging metadata flips more=false -> true.
    let db = Db::new(&db_path).unwrap();
    let journal =
        JournalContext::internal_system(format!("req-{}", uuid::Uuid::new_v4()), authority());
    record_direct(
        &db,
        &id,
        "ev-late",
        "status_changed",
        &journal.event_envelope(None),
        "2026-10-04T12:00:00+00:00",
    );
    drop(db);

    // Revalidating the SAME URI with the cached ETag: the ETag is
    // bound to the paging metadata, so it CHANGED — the response MUST
    // be a full 200 carrying more=true, never a 304 that would let
    // the client keep the stale more=false.
    let resp = get_with_header(&app, &uri, "if-none-match", &etag).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a paging-metadata change must invalidate the cached representation"
    );
    assert_eq!(
        resp.headers()
            .get("x-more-available")
            .and_then(|v| v.to_str().ok())
            .expect("header present"),
        "true",
        "the client must discover the newly available page"
    );
    let new_etag = resp
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    let new_cursor = resp
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    // Same body bytes — the difference is purely the metadata.
    assert_eq!(exhausted_body, body_bytes(resp).await);
    assert_ne!(etag, new_etag);

    // The client drains the newly available page.
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner&after={new_cursor}&limit=1"),
    )
    .await;
    let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(envelope.payload.events.len(), 1);
    assert_eq!(envelope.payload.events[0].id, "ev-late");

    // Revalidating AGAIN with the CURRENT etag yields a 304 — and the
    // 304 carries BOTH paging headers so no intermediary can strip
    // the reconnect metadata.
    let resp = get_with_header(&app, &uri, "if-none-match", &new_etag).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert!(
        resp.headers()
            .get("x-next-cursor")
            .and_then(|v| v.to_str().ok())
            .is_some()
    );
    assert!(
        resp.headers()
            .get("x-more-available")
            .and_then(|v| v.to_str().ok())
            .is_some()
    );
}

#[tokio::test]
async fn two_observers_see_identical_bytes() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let first = body_bytes(
        get(
            &app,
            &format!("/work-contexts/{id}/work-events?user_id=owner"),
        )
        .await,
    )
    .await;
    let second = body_bytes(
        get(
            &app,
            &format!("/work-contexts/{id}/work-events?user_id=owner"),
        )
        .await,
    )
    .await;
    assert_eq!(
        first, second,
        "multiple authorized clients observe identical bytes; no session state"
    );
}

// --- Run identity -----------------------------------------------------------

#[tokio::test]
async fn typed_run_kinds_are_distinct_resources() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    // Record a work run "shared-1" AND a graph run "shared-1" directly
    // against the durable file: equal strings, different identity types.
    let db = Db::new(&db_path).unwrap();
    let work_journal = JournalContext::for_work_run(
        "owner",
        "req-a".to_string(),
        "shared-1".to_string(),
        authority(),
    );
    let graph_base = JournalContext::for_request("owner", "req-b".to_string(), authority());
    let graph_envelope = graph_base.graph_run_envelope("shared-1".to_string(), None);
    record_direct(
        &db,
        &id,
        "ev-work",
        "status_changed",
        &work_journal.event_envelope(None),
        "2026-10-04T12:00:00+00:00",
    );
    record_direct(
        &db,
        &id,
        "ev-graph",
        "status_changed",
        &graph_envelope,
        "2026-10-04T12:01:00+00:00",
    );
    drop(db);

    // The run-keys enumeration contains BOTH typed keys as DISTINCT
    // entries (the API-written events legitimately add their own request
    // runs) — equal strings under different kinds never merge.
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-event-runs?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let keys = body_json(resp).await;
    let entries: Vec<(String, String)> = keys
        .as_array()
        .expect("the enumeration is an array")
        .iter()
        .map(|entry| {
            (
                entry["kind"].as_str().expect("kind").to_string(),
                entry["id"].as_str().expect("id").to_string(),
            )
        })
        .collect();
    assert!(
        entries.contains(&("work-run".to_string(), "shared-1".to_string())),
        "the work run is listed: {entries:?}"
    );
    assert!(
        entries.contains(&("graph-run".to_string(), "shared-1".to_string())),
        "the graph run is listed: {entries:?}"
    );
    // First-appearance order: the work run was recorded before the graph run.
    let work_pos = entries
        .iter()
        .position(|(kind, id)| kind == "work-run" && id == "shared-1")
        .unwrap();
    let graph_pos = entries
        .iter()
        .position(|(kind, id)| kind == "graph-run" && id == "shared-1")
        .unwrap();
    assert!(
        work_pos < graph_pos,
        "first-appearance seq order: {entries:?}"
    );

    // Distinct resources: each batch contains exactly its own event.
    for (kind, expected) in [("work-run", "ev-work"), ("graph-run", "ev-graph")] {
        let resp = get(
            &app,
            &format!("/work-contexts/{id}/work-event-runs/{kind}/shared-1?user_id=owner"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "{kind}");
        let bytes = body_bytes(resp).await;
        let envelope: VersionedProjectionEnvelope<
            prometheos_lite::workflow::soma::event::WorkEventBatch,
        > = serde_json::from_slice(&bytes).expect("batch envelope parses");
        assert_eq!(envelope.payload.events.len(), 1, "{kind}");
        assert_eq!(envelope.payload.events[0].id, expected, "{kind}");
    }
}

#[tokio::test]
async fn unknown_kind_is_400_listing_kinds() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-event-runs/virtual-run/x?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    let msg = body["error"].as_str().unwrap();
    assert!(
        msg.contains("work-run") && msg.contains("graph-run") && msg.contains("request"),
        "{msg}"
    );
}

#[tokio::test]
async fn unknown_run_id_is_404() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-event-runs/work-run/wr-nope?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert!(
        body["error"].as_str().unwrap().contains("work-run:wr-nope"),
        "{}",
        body
    );
}

// --- Fail-closed over HTTP --------------------------------------------------

#[tokio::test]
async fn tampered_row_surfaces_the_gate_text_as_500() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let db = Db::new(&db_path).unwrap();
    with_triggers_suspended(&db, || {
        db.conn()
            .execute(
                "UPDATE work_context_events
                 SET data = '{\"tampered\": true}'
                 WHERE work_context_id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
    });

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let msg = body_json(resp).await["error"].as_str().unwrap().to_string();
    assert!(
        msg.contains("source-digest verification") || msg.contains("tamper"),
        "the read gate's own detection text must surface: {msg}"
    );
}

#[tokio::test]
async fn legacy_row_is_422_naming_the_event() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let db = Db::new(&db_path).unwrap();
    let legacy_id: String = db
        .conn()
        .query_row(
            "SELECT id FROM work_context_events WHERE work_context_id = ?1 ORDER BY seq LIMIT 1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .unwrap();
    with_triggers_suspended(&db, || {
        db.conn()
            .execute(
                "UPDATE work_context_events
                 SET provenance_json = NULL, source_digest = NULL, run_id = NULL,
                     principal_id = NULL, correlation_id = NULL
                 WHERE work_context_id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
    });

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body_json(resp).await["error"].as_str().unwrap().to_string();
    assert!(msg.contains(&legacy_id) && msg.contains("legacy"), "{msg}");
}

#[tokio::test]
async fn unmapped_event_type_is_422_naming_the_type() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let db = Db::new(&db_path).unwrap();
    let journal =
        JournalContext::internal_system(format!("req-{}", uuid::Uuid::new_v4()), authority());
    record_direct(
        &db,
        &id,
        "ev-future",
        "future_widget",
        &journal.event_envelope(None),
        "2026-10-04T12:00:00+00:00",
    );

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body_json(resp).await["error"].as_str().unwrap().to_string();
    assert!(msg.contains("future_widget"), "{msg}");
}

#[tokio::test]
async fn stored_system_producer_is_422() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    let db = Db::new(&db_path).unwrap();
    let journal =
        JournalContext::internal_system(format!("req-{}", uuid::Uuid::new_v4()), authority());
    let mut envelope = journal.event_envelope(None);
    envelope.producer.kind = prometheos_lite::work::provenance::ProducerKind::System;
    record_direct(
        &db,
        &id,
        "ev-sys",
        "status_changed",
        &envelope,
        "2026-10-04T12:00:00+00:00",
    );

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body_json(resp).await["error"].as_str().unwrap().to_string();
    assert!(msg.contains("system"), "{msg}");
}

#[tokio::test]
async fn audit_failure_is_500() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;

    // A causal cycle among request-run events: the batch audit gate
    // refuses — surfaced as 500 with the SOMA-EVT-0002 code.
    let db = Db::new(&db_path).unwrap();
    let journal = JournalContext::internal_system("req-cycle".to_string(), authority());
    for (event_id, parent) in [("ev-a", Some("ev-b")), ("ev-b", Some("ev-a"))] {
        let event = fixed_event(event_id, &id, "status_changed", "2026-10-04T12:00:00+00:00");
        record_event_conn(
            db.conn(),
            &event,
            &journal.event_envelope(parent.map(str::to_string)),
        )
        .unwrap();
    }

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-event-runs/request/req-cycle?user_id=owner"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let msg = body_json(resp).await["error"].as_str().unwrap().to_string();
    assert!(msg.contains("SOMA-EVT-0002"), "{msg}");
}

// --- Boundary ---------------------------------------------------------------

#[tokio::test]
async fn final_non_empty_page_returns_its_last_seq_cursor() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await; // 2 events

    // A page that exhausts the existing events (review P1b): the
    // continuation cursor is STILL present — the last returned seq —
    // so the client keeps the reconnect position for events that
    // arrive later. X-More-Available carries the exhaustion fact.
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    let next_cursor = resp
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .expect("the final non-empty page still carries the cursor")
        .to_string();
    let more = resp
        .headers()
        .get("x-more-available")
        .and_then(|v| v.to_str().ok())
        .expect("x-more-available present")
        .to_string();
    let bytes = body_bytes(resp).await;
    let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(more, "false");
    // The cursor is the LAST RETURNED event's durable seq.
    let last_seq = envelope.payload.events.last().unwrap().sequence as i64;
    assert_eq!(next_cursor, last_seq.to_string());
}

#[tokio::test]
async fn empty_page_preserves_the_supplied_cursor() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await; // 2 events at seq 1..2

    // Polling beyond the current end: an empty page whose continuation
    // cursor is the REQUEST's cursor — the reconnect position survives
    // an exhausted poll (review P1b).
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner&after=500"),
    )
    .await;
    let next_cursor = resp
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .expect("the empty page still carries the cursor")
        .to_string();
    let more = resp
        .headers()
        .get("x-more-available")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(next_cursor, "500");
    assert_eq!(more, "false");
    assert!(envelope.payload.events.is_empty());
}

#[tokio::test]
async fn polling_after_a_new_event_returns_only_that_event() {
    let (state, db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await; // 2 events

    // Exhaust the stream and keep the preserved cursor.
    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner"),
    )
    .await;
    let cursor = resp
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .expect("cursor present on the final page")
        .to_string();
    drop(resp);

    // A NEW event arrives later (a direct journal write — any writer
    // would do): polling from the preserved cursor returns EXACTLY the
    // new event and nothing else.
    let db = Db::new(&db_path).unwrap();
    let journal =
        JournalContext::internal_system(format!("req-{}", uuid::Uuid::new_v4()), authority());
    record_direct(
        &db,
        &id,
        "ev-late",
        "status_changed",
        &journal.event_envelope(None),
        "2026-10-04T12:00:00+00:00",
    );
    drop(db);

    let resp = get(
        &app,
        &format!("/work-contexts/{id}/work-events?user_id=owner&after={cursor}"),
    )
    .await;
    let more = resp
        .headers()
        .get("x-more-available")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    let new_cursor = resp
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    let envelope: VersionedProjectionEnvelope<WorkEventStreamPage> =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(envelope.payload.events.len(), 1, "exactly the new event");
    assert_eq!(envelope.payload.events[0].id, "ev-late");
    assert_eq!(more, "false");
    assert_eq!(new_cursor, envelope.payload.events[0].sequence.to_string());
}

#[tokio::test]
async fn all_new_routes_are_get_only() {
    let (state, _db_path, _dir) = test_app_state();
    let app = test_router(&state);
    let id = driven_context(&app, "owner").await;
    for uri in [
        format!("/work-contexts/{id}/work-events?user_id=owner"),
        format!("/work-contexts/{id}/work-event-runs?user_id=owner"),
        format!("/work-contexts/{id}/work-event-runs/request/req-1?user_id=owner"),
    ] {
        let resp = post(&app, &uri).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "{uri}");
    }
}
