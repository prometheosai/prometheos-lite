use axum::body::Body;
use axum::http::Method;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt;

fn test_app_state() -> (
    std::sync::Arc<prometheos_lite::api::AppState>,
    tempfile::TempDir,
) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("api_compat_test.db")
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
        prometheos_lite::api::AppState::new(db_path, runtime, embedding, memory_service)
            .expect("app state"),
    );

    (state, db_dir)
}

#[tokio::test]
async fn project_crud_create_and_list() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/projects")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"name":"smoke-test-project"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        create_resp.status(),
        201,
        "POST /projects should return 201"
    );

    let list_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200, "GET /projects should return 200");

    let body_bytes = axum::body::to_bytes(list_resp.into_body(), 1024 * 16)
        .await
        .unwrap();
    let projects: Vec<Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(!projects.is_empty(), "should have at least one project");
    assert_eq!(projects[0]["name"], "smoke-test-project");
    assert!(!projects[0]["id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn project_crud_create_and_get_by_id() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/projects")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"name":"get-by-id-test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);

    let body_bytes = axum::body::to_bytes(create_resp.into_body(), 1024 * 16)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&body_bytes).unwrap();
    let project_id = created["id"].as_str().unwrap().to_string();

    let get_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        get_resp.status(),
        200,
        "GET /projects/:id should return 200"
    );

    let body_bytes = axum::body::to_bytes(get_resp.into_body(), 1024 * 16)
        .await
        .unwrap();
    let project: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(project["id"], project_id);
    assert_eq!(project["name"], "get-by-id-test");
}

#[tokio::test]
async fn project_crud_get_nonexistent_returns_404() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let get_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects/nonexistent-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        get_resp.status(),
        404,
        "GET /projects/:id for nonexistent should return 404"
    );
}

#[tokio::test]
async fn project_crud_multiple_projects() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    for name in &["project-a", "project-b", "project-c"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/projects")
                    .header("Content-Type", "application/json")
                    .body(Body::from(format!(r#"{{"name":"{}"}}"#, name)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
    }

    let list_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);

    let body_bytes = axum::body::to_bytes(list_resp.into_body(), 1024 * 16)
        .await
        .unwrap();
    let projects: Vec<Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(projects.len(), 3, "should have three projects");

    let names: Vec<&str> = projects
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"project-a"));
    assert!(names.contains(&"project-b"));
    assert!(names.contains(&"project-c"));
}
