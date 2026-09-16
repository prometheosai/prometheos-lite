//! E6/I03 decide endpoint (#132):
//! POST /work-contexts/:id/graph-runs/:run_id/decisions
//!
//! Covers:
//! - happy path: changes are persisted back to the registry with a new digest
//! - ownership + wrong-user ~ 403
//! - unknown run -> 404
//! - cancellation of the work context -> 409 (no decisions ever commit)
//! - already-terminated run -> 409
//! - decision rejected: basis not journaled -> 409
//! - basis journaled, decision published in the new checkpoint

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::graph_checkpoints::{get_checkpoint, upsert_checkpoint};
use prometheos_lite::workflow::graph_state::{
    GraphManifestV1, GraphRunStateV1, NodeAttemptRecordV1, OutcomeCategory,
};

const PORTABLE_DIGEST: &str = "2aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Build the minimal sealed manifest: a -> b -> done.
fn manifest() -> GraphManifestV1 {
    use prometheos_lite::workflow::graph_state::{EdgeKind, GraphEdgeV1, GraphNodeV1};

    GraphManifestV1 {
        schema_version: "1.0.0".into(),
        graph_id: "g1".into(),
        version: "1.0.0".into(),
        nodes: vec![
            GraphNodeV1 {
                node_id: "a".into(),
                capability: "n.a".into(),
                purpose: None,
                resources: Vec::new(),
                join: None,
            },
            GraphNodeV1 {
                node_id: "b".into(),
                capability: "n.b".into(),
                purpose: None,
                resources: Vec::new(),
                join: None,
            },
            GraphNodeV1 {
                node_id: "done".into(),
                capability: "n.done".into(),
                purpose: None,
                resources: Vec::new(),
                join: None,
            },
        ],
        edges: vec![
            GraphEdgeV1 {
                from: "a".into(),
                to: "b".into(),
                kind: EdgeKind::Sequence,
                condition_label: None,
            },
            GraphEdgeV1 {
                from: "b".into(),
                to: "done".into(),
                kind: EdgeKind::Sequence,
                condition_label: None,
            },
        ],
        entry_points: vec!["a".into()],
        terminal_exits: vec!["done".into()],
        shared_state_keys: Vec::new(),
        policy_digest: None,
        content_digest: Some(String::new()), // placeholder, sealed below
    }
    .sealed()
}

/// Make a run with `a` journaled as completed and frontier ["b"].
fn checkpoint_blob(run_id: &str) -> (GraphManifestV1, String) {
    let manifest = manifest();
    let mut state = GraphRunStateV1::open(
        run_id,
        &manifest,
        "revision-1",
        "pws",
        PORTABLE_DIGEST,
        "2026-01-01T00:00:00Z",
    )
    .expect("open run");

    state
        .apply_node_completion(
            "a",
            NodeAttemptRecordV1 {
                attempt: 1,
                started_at: "2026-01-01T00:00:00Z".into(),
                completed_at: Some("2026-01-01T00:00:01Z".into()),
                outcome: OutcomeCategory::Completed,
                result_digest: "d".repeat(64),
            },
            "2026-01-01T00:00:01Z",
        )
        .expect("journaled completion");

    (manifest, state.export_checkpoint().expect("checkpoint"))
}

fn test_app_state() -> (
    std::sync::Arc<prometheos_lite::api::AppState>,
    String,
    tempfile::TempDir,
) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("decide_test.db")
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
                        "goal": "decide",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 128 * 1024)
        .await
        .unwrap();
    let j: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    j["id"].as_str().unwrap().to_string()
}

async fn cancel_context_http(app: &axum::Router, user: &str, ctx: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work-contexts/{}/cancel?user_id={}", ctx, user))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({"reason": "stop"})).expect("json"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

async fn decide(
    app: &axum::Router,
    user: &str,
    ctx: &str,
    run_id: &str,
    manifest: &GraphManifestV1,
    decision: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "manifest": serde_json::to_value(manifest).unwrap(),
        "portableStateDigest": PORTABLE_DIGEST,
        "decision": decision,
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/work-contexts/{}/graph-runs/{}/decisions?user_id={}",
                    ctx, run_id, user
                ))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 128 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({"bad_json": true}));
    (status, json)
}

async fn setup_seeded_run(
    db_path: &str,
    user: &str,
    ctx: &str,
    run_id: &str,
) -> (GraphManifestV1, String) {
    let db = Db::new(db_path).expect("db");
    let (m, blob) = checkpoint_blob(run_id);
    upsert_checkpoint(&db, user, ctx, run_id, &blob).expect("seed checkpoint");
    (m, blob)
}

fn decision_payload(from: &str, to: &str, basis: &str) -> serde_json::Value {
    serde_json::json!({
        "recordedAt": "2026-01-01T00:00:02Z",
        "fromNode": from,
        "toNode": to,
        "basisResultDigest": basis,
        // conditionLabel: omitted
    })
}

#[tokio::test]
async fn happy_path_decision_updates_checkpoint() {
    let (state, db_path, _dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let ctx = create_context(&app, "owner", "t1").await;

    let (manifest, blob) = setup_seeded_run(&db_path, "owner", &ctx, "run-1").await;
    let before_digest = prometheos_lite::workflow::soma::canonical::sha256_hex(blob.as_bytes());

    let (status, body) = decide(
        &app,
        "owner",
        &ctx,
        "run-1",
        &manifest,
        decision_payload("a", "b", &"d".repeat(64)),
    )
    .await;
    assert_eq!(status, 200);

    assert_eq!(body["recorded"], true);
    assert_eq!(body["runId"].as_str(), Some("run-1"));

    // Known: checkpoint digest is re-pinned after decision + export.
    let new_digest = body["newDigest"].as_str().expect("registry re-pins");
    assert_ne!(new_digest, before_digest);

    let db = Db::new(&db_path).expect("db");
    let (new_blob, digest) = get_checkpoint(&db, "owner", &ctx, "run-1")
        .unwrap()
        .expect("row exists");
    assert_eq!(digest, new_digest, "registry in sync with response");
    assert_ne!(
        new_blob, blob,
        "checkpoint content must reflect the new decision"
    );

    let sre: prometheos_lite::workflow::graph_state::GraphRunStateV1 =
        prometheos_lite::workflow::graph_state::GraphRunStateV1::import_checkpoint(
            &new_blob,
            &manifest,
            PORTABLE_DIGEST,
        )
        .expect("re-import after decision");
    assert_eq!(sre.decisions.len(), 1);
    assert_eq!(sre.frontier, vec!["b".to_string()]);
    assert_eq!(sre.decisions[0].from_node, "a");
    assert_eq!(sre.decisions[0].to_node, "b");
}

#[tokio::test]
async fn unknown_run_is_404_and_wrong_user_is_403() {
    let (state, db_path, _dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let ctx = create_context(&app, "owner", "t2").await;
    let (manifest, _blob) = setup_seeded_run(&db_path, "owner", &ctx, "run-1").await;

    // Run not registered.
    let (status, _) = decide(
        &app,
        "owner",
        &ctx,
        "nonexistent",
        &manifest,
        decision_payload("a", "b", &"d".repeat(64)),
    )
    .await;
    assert_eq!(status, 404);

    // Wrong user.
    let (status, _) = decide(
        &app,
        "intruder",
        &ctx,
        "run-1",
        &manifest,
        decision_payload("a", "b", &"d".repeat(64)),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn cancel_of_work_context_blocks_decide() {
    let (state, db_path, _dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let ctx = create_context(&app, "owner", "t3").await;
    let (manifest, _blob) = setup_seeded_run(&db_path, "owner", &ctx, "run-1").await;

    cancel_context_http(&app, "owner", &ctx).await;

    let (status, _) = decide(
        &app,
        "owner",
        &ctx,
        "run-1",
        &manifest,
        decision_payload("a", "b", &"d".repeat(64)),
    )
    .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn unjournaled_basis_results_in_conflict() {
    let (state, db_path, _dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let ctx = create_context(&app, "owner", "t4").await;
    let (manifest, _blob) = setup_seeded_run(&db_path, "owner", &ctx, "run-1").await;

    let (status, _) = decide(
        &app,
        "owner",
        &ctx,
        "run-1",
        &manifest,
        decision_payload("a", "b", "boom-not-journaled"),
    )
    .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn terminated_run_refuses_decision() {
    let (state, db_path, _dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);
    let ctx = create_context(&app, "owner", "t5").await;
    let (manifest, blob) = setup_seeded_run(&db_path, "owner", &ctx, "run-1").await;

    // Terminate by synthetic-marking the checkpoint with a cancelled run.
    let mut state: prometheos_lite::workflow::graph_state::GraphRunStateV1 =
        prometheos_lite::workflow::graph_state::GraphRunStateV1::import_checkpoint(
            &blob,
            &manifest,
            PORTABLE_DIGEST,
        )
        .expect("import");
    state.termination = Some(prometheos_lite::workflow::graph_state::RunTerminationV1 {
        kind: "cancelled".into(),
        reason: "user stop".into(),
        recorded_at: "2026-01-01T00:00:05Z".into(),
    });
    state.content_digest = Some(state.compute_digest());
    let terminated_blob = state.export_checkpoint().expect("export");
    let db = Db::new(&db_path).expect("db");
    upsert_checkpoint(&db, "owner", &ctx, "run-1", &terminated_blob).expect("seed");

    let (status, _) = decide(
        &app,
        "owner",
        &ctx,
        "run-1",
        &manifest,
        decision_payload("a", "b", &"d".repeat(64)),
    )
    .await;
    assert_eq!(status, 409);
}
