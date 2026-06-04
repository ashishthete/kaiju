//! Integration tests for the daemon HTTP API contract.
//!
//! These exercise the real Axum router and shared state via `oneshot`, with no
//! network socket. Only endpoints that do not shell out to tmux are covered
//! here, so the suite runs in any environment (no terminal required).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kaiju_daemon::server::{build_app, AppState};
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
    Request::builder().uri(uri).body(Body::empty()).unwrap()
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

// -- Auth --

fn authed_state() -> AppState {
    let mut state = AppState::new();
    state.auth_token = Some("secret".to_string());
    state
}

#[tokio::test]
async fn protected_route_rejects_missing_token() {
    let resp = build_app(authed_state())
        .oneshot(get_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_valid_token() {
    let req = Request::builder()
        .uri("/agents")
        .header("authorization", "Bearer secret")
        .body(Body::empty())
        .unwrap();
    let resp = build_app(authed_state()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_and_dashboard_are_public_under_auth() {
    let h = build_app(authed_state())
        .oneshot(get_request("/health"))
        .await
        .unwrap();
    assert_eq!(h.status(), StatusCode::OK);

    let root = build_app(authed_state())
        .oneshot(get_request("/"))
        .await
        .unwrap();
    assert_eq!(root.status(), StatusCode::OK);
}

// -- Task queue (scheduler loop is not running here, so tasks stay queued) --

#[tokio::test]
async fn create_task_returns_created_and_queued() {
    let req = json_request(
        "POST",
        "/tasks",
        serde_json::json!({
            "agent_type": "claude",
            "workspace": "/tmp",
            "prompt": "do it"
        }),
    );
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "queued");
    assert_eq!(json["agent_type"], "claude");
}

#[tokio::test]
async fn create_task_unsupported_type_is_rejected() {
    let req = json_request(
        "POST",
        "/tasks",
        serde_json::json!({ "agent_type": "aider", "workspace": "/tmp" }),
    );
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn task_lifecycle_create_list_get_cancel() {
    let state = AppState::new();

    let create = json_request(
        "POST",
        "/tasks",
        serde_json::json!({ "agent_type": "codex", "workspace": "/tmp", "prompt": "x" }),
    );
    let resp = build_app(state.clone()).oneshot(create).await.unwrap();
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // List shows the one task.
    let resp = build_app(state.clone())
        .oneshot(get_request("/tasks"))
        .await
        .unwrap();
    assert_eq!(body_json(resp).await.as_array().unwrap().len(), 1);

    // Cancel it -> canceled.
    let cancel = Request::builder()
        .method("POST")
        .uri(format!("/tasks/{id}/cancel"))
        .body(Body::empty())
        .unwrap();
    let resp = build_app(state.clone()).oneshot(cancel).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "canceled");

    // Get reflects the canceled status.
    let resp = build_app(state)
        .oneshot(get_request(&format!("/tasks/{id}")))
        .await
        .unwrap();
    assert_eq!(body_json(resp).await["status"], "canceled");
}

#[tokio::test]
async fn get_missing_task_returns_404() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(get_request("/tasks/nope")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_missing_task_returns_404() {
    let req = Request::builder()
        .method("POST")
        .uri("/tasks/nope/cancel")
        .body(Body::empty())
        .unwrap();
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dashboard_served_at_root() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(get_request("/")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("Kaiju"));
    assert!(html.contains("/agents"));
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
async fn create_with_isolate_flag_is_accepted() {
    // Without auto_start no git work happens; the flag is just recorded.
    let req = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "claude",
            "workspace": "/tmp/project",
            "auto_start": false,
            "isolate": true
        }),
    );
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
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
async fn diff_missing_agent_returns_404() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(get_request("/agents/does-not-exist/diff"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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
async fn send_input_to_missing_agent_returns_404() {
    let req = json_request(
        "POST",
        "/agents/nope/input",
        serde_json::json!({ "text": "hello" }),
    );
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn send_input_to_stopped_agent_returns_409() {
    let state = AppState::new();

    // Create an agent.
    let create = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "claude",
            "workspace": "/tmp",
            "auto_start": false
        }),
    );
    let resp = build_app(state.clone()).oneshot(create).await.unwrap();
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Stop it. With no tmux session present, stop just marks it Stopped.
    let stop = Request::builder()
        .method("POST")
        .uri(format!("/agents/{id}/stop"))
        .body(Body::empty())
        .unwrap();
    let resp = build_app(state.clone()).oneshot(stop).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Sending input to a terminal agent is a conflict.
    let input = json_request(
        "POST",
        &format!("/agents/{id}/input"),
        serde_json::json!({ "text": "hello" }),
    );
    let resp = build_app(state).oneshot(input).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
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

#[tokio::test]
async fn xterm_asset_is_served_publicly() {
    let app = kaiju_daemon::server::build_app(kaiju_daemon::server::AppState::new());
    let res = app.oneshot(get_request("/assets/xterm.js")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("javascript"), "got content-type {ct}");
}
