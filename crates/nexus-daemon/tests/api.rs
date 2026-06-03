//! Integration tests for the daemon HTTP API contract.
//!
//! These exercise the real Axum router and shared state via `oneshot`, with no
//! network socket. Only endpoints that do not shell out to tmux are covered
//! here, so the suite runs in any environment (no terminal required).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nexus_daemon::server::{build_app, AppState};
use serde_json::Value;
use tower::ServiceExt; // for `oneshot`

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_returns_ok() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(get_request("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn create_agent_without_autostart_returns_created() {
    let state = AppState::new();
    let req = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "claude",
            "workspace": "/tmp/project",
            "auto_start": false
        }),
    );

    let resp = build_app(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let json = body_json(resp).await;
    assert_eq!(json["agent_type"], "claude");
    assert_eq!(json["status"], "starting");
}

#[tokio::test]
async fn unsupported_agent_type_is_rejected() {
    let state = AppState::new();
    let req = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "aider",
            "workspace": "/tmp",
            "auto_start": false
        }),
    );

    let resp = build_app(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_then_list_and_get() {
    let state = AppState::new();

    // Create.
    let create = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "codex",
            "workspace": "/tmp",
            "auto_start": false
        }),
    );
    let resp = build_app(state.clone()).oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    let id = created["id"].as_str().unwrap().to_string();

    // List should contain exactly one agent.
    let resp = build_app(state.clone())
        .oneshot(get_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Get by id returns the same agent.
    let resp = build_app(state)
        .oneshot(get_request(&format!("/agents/{id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fetched = body_json(resp).await;
    assert_eq!(fetched["id"], id);
}

#[tokio::test]
async fn get_missing_agent_returns_404() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(get_request("/agents/does-not-exist"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_missing_agent_returns_404() {
    let req = Request::builder()
        .method("DELETE")
        .uri("/agents/nope")
        .body(Body::empty())
        .unwrap();
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
