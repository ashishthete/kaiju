//! Integration tests for the daemon HTTP API contract.
//!
//! These exercise the real Axum router and shared state via `oneshot`, with no
//! network socket. Only endpoints that do not shell out to tmux are covered
//! here, so the suite runs in any environment (no terminal required).

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kaiju_daemon::server::{build_app, AppState};
use serde_json::Value;
use std::net::SocketAddr;
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

/// Point `KAIJU_DEVICES` at a unique temp file so device-save side effects in
/// handler tests never touch the real `~/.kaiju/devices.json`. `cargo test`
/// shares one process, so the filename must be unique per test.
fn isolate_devices_path(test_name: &str) {
    let path = std::env::temp_dir().join(format!("kaiju-test-devices-{test_name}.json"));
    let _ = std::fs::remove_file(&path);
    std::env::set_var("KAIJU_DEVICES", &path);
}

/// A request carrying a simulated remote (non-loopback) peer, so the auth
/// middleware exercises token enforcement rather than loopback trust.
fn remote_request(uri: &str) -> Request<Body> {
    let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "203.0.113.7:54321".parse::<SocketAddr>().unwrap(),
    ));
    req
}

/// A remote request that also presents a bearer token.
fn remote_request_with_token(uri: &str, token: &str) -> Request<Body> {
    let mut req = remote_request(uri);
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().unwrap(),
    );
    req
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
        .oneshot(remote_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_valid_token() {
    let resp = build_app(authed_state())
        .oneshot(remote_request_with_token("/agents", "secret"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn loopback_peer_is_trusted_without_token() {
    // No ConnectInfo (in-process) = loopback = always allowed, even with a
    // token configured. This is the host-machine trust anchor.
    let resp = build_app(authed_state())
        .oneshot(get_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn remote_peer_rejected_when_no_token_configured() {
    // The LAN-exposed-without-a-shared-token case: a remote, unpaired device
    // is still rejected (it must pair first).
    let resp = build_app(AppState::new())
        .oneshot(remote_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_peer_with_device_token_is_accepted() {
    let state = AppState::new();
    state
        .devices
        .write()
        .unwrap()
        .add("Phone".into(), "dev-tok-1".into(), chrono::Utc::now());
    let resp = build_app(state)
        .oneshot(remote_request_with_token("/agents", "dev-tok-1"))
        .await
        .unwrap();
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
async fn create_task_custom_type_is_accepted() {
    // A non-builtin type ("aider") is a custom CLI and is enqueued, not rejected.
    let req = json_request(
        "POST",
        "/tasks",
        serde_json::json!({ "agent_type": "aider", "workspace": "/tmp" }),
    );
    let resp = build_app(AppState::new()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn create_task_blank_type_is_rejected() {
    let req = json_request(
        "POST",
        "/tasks",
        serde_json::json!({ "agent_type": "  ", "workspace": "/tmp" }),
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
    assert!(html.contains(r#"src="/assets/dashboard.js""#));
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
async fn custom_agent_type_is_accepted() {
    // A non-builtin type ("aider") is treated as a custom CLI and created.
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
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn blank_agent_type_is_rejected() {
    let state = AppState::new();
    let req = json_request(
        "POST",
        "/agents",
        serde_json::json!({
            "agent_type": "  ",
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

#[tokio::test]
async fn dashboard_scripts_are_served_publicly() {
    for path in ["/assets/dashboard.js", "/assets/dashboard-utils.js"] {
        let app = kaiju_daemon::server::build_app(kaiju_daemon::server::AppState::new());
        let res = app.oneshot(get_request(path)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("javascript"), "{path} content-type {ct}");
    }
}

#[tokio::test]
async fn dashboard_page_references_the_extracted_scripts() {
    let app = kaiju_daemon::server::build_app(kaiju_daemon::server::AppState::new());
    let res = app.oneshot(get_request("/")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains(r#"src="/assets/dashboard-utils.js""#));
    assert!(html.contains(r#"src="/assets/dashboard.js""#));
}

// -- Pairing + Device endpoints --

#[tokio::test]
async fn pair_claim_with_valid_code_returns_a_token() {
    isolate_devices_path("pair_claim_valid");
    let state = AppState::new();
    let now = chrono::Utc::now();
    state
        .pending_codes
        .lock()
        .unwrap()
        .issue("TESTCODE".into(), now);
    let app = build_app(state.clone());

    let resp = app
        .oneshot(json_request(
            "POST",
            "/pair/claim",
            serde_json::json!({ "code": "TESTCODE", "name": "Phone" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["token"].as_str().is_some());
    assert_eq!(state.devices.read().unwrap().devices.len(), 1);
}

#[tokio::test]
async fn pair_claim_with_bad_code_is_rejected() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/pair/claim",
            serde_json::json!({ "code": "WRONG", "name": "Phone" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_devices_returns_paired_without_tokens() {
    isolate_devices_path("list_devices");
    let state = AppState::new();
    state
        .devices
        .write()
        .unwrap()
        .add("Phone".into(), "tok-xyz".into(), chrono::Utc::now());
    let app = build_app(state);
    let resp = app.oneshot(get_request("/devices")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // One device, name present, token NOT leaked.
    assert_eq!(json[0]["name"], "Phone");
    assert!(json[0].get("token").is_none());
}

#[tokio::test]
async fn revoke_device_removes_it() {
    isolate_devices_path("revoke_device");
    let state = AppState::new();
    let dev = state
        .devices
        .write()
        .unwrap()
        .add("Phone".into(), "tok-xyz".into(), chrono::Utc::now());
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/devices/{}", dev.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(state.devices.read().unwrap().devices.len(), 0);
}

#[tokio::test]
async fn list_sessions_unsupported_type_returns_empty() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(get_request("/sessions?workspace=/tmp/x&type=gemini"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn adopt_rejects_blank_session_id() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/agents/adopt",
            serde_json::json!({ "agent_type": "claude", "workspace": "/tmp/x", "session_id": "" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn adopt_rejects_session_id_with_shell_metachars() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/agents/adopt",
            serde_json::json!({ "agent_type": "claude", "workspace": "/tmp/x", "session_id": "a; rm -rf ~" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compare_rejects_empty_agent_types() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(json_request(
        "POST", "/compare",
        serde_json::json!({ "workspace": "/tmp/x", "prompt": "do X", "agent_types": [] }),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compare_rejects_blank_prompt() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(json_request(
        "POST", "/compare",
        serde_json::json!({ "workspace": "/tmp/x", "prompt": "", "agent_types": ["claude"] }),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compare_rejects_non_git_workspace() {
    let dir = std::env::temp_dir().join("kaiju-compare-nongit");
    std::fs::create_dir_all(&dir).unwrap();
    let app = build_app(AppState::new());
    let resp = app.oneshot(json_request(
        "POST", "/compare",
        serde_json::json!({ "workspace": dir.to_string_lossy(), "prompt": "do X", "agent_types": ["claude"] }),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_response_includes_compare_group_field() {
    use kaiju_core::agent::{Agent, AgentConfig, AgentType};
    let state = AppState::new();
    let agent = Agent::new(AgentConfig {
        agent_type: AgentType::Claude, model: None,
        workspace: std::path::PathBuf::from("/tmp"), prompt: None, extra_args: vec![],
    });
    let id = agent.id.clone();
    state.store.insert(agent);
    let app = build_app(state);
    let resp = app.oneshot(get_request(&format!("/agents/{id}"))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.get("compare_group").is_some());
}

#[tokio::test]
async fn compare_rejects_blank_agent_type_entry() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/compare",
            serde_json::json!({ "workspace": "/tmp/x", "prompt": "do X", "agent_types": ["claude", ""] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn judge_rejects_group_with_fewer_than_two_runs() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request("POST", "/compare/judge", serde_json::json!({ "group_id": "nope" })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn judge_accepts_test_cmd_field() {
    // The optional test_cmd must deserialize cleanly; with an unknown group the
    // outcome is still the <2-runs 400 (proves the field round-trips).
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/compare/judge",
            serde_json::json!({ "group_id": "nope", "test_cmd": "echo ok" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
