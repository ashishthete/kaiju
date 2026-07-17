use axum::extract::Path;
use axum::http::StatusCode;
use axum::{
    routing::{get, post},
    Json, Router,
};
use kaiju_pet::client::Client;
use kaiju_pet::error::PetError;
use std::net::SocketAddr;

async fn stub_agents() -> Json<serde_json::Value> {
    Json(serde_json::json!([{
        "id":"ag_1","agent_type":"claude","model":null,"workspace":"/w",
        "status":"waitingforinput","session_name":"kaiju-ag_1","prompt":"p",
        "created_at":"2026-07-16T10:00:00+00:00","updated_at":"2026-07-16T10:05:00+00:00",
        "metrics":{"runtime_secs":1,"tokens_used":null,"estimated_cost_usd":null},
        "compare_group":null,"port":null,"resume_at":null,"auto_continue":false
    }]))
}

async fn stub_input(Path(_id): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "sent" }))
}

async fn stub_input_conflict(Path(_id): Path<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({ "error": "agent is not running" })),
    )
}

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn get_agents_decodes_snapshot() {
    let base = spawn(Router::new().route("/agents", get(stub_agents))).await;
    let out = tokio::task::spawn_blocking(move || Client::new(base, None).get_agents())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "ag_1");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_input_ok() {
    let base = spawn(Router::new().route("/agents/:id/input", post(stub_input))).await;
    let res = tokio::task::spawn_blocking(move || Client::new(base, None).send_input("ag_1", "yes"))
        .await
        .unwrap();
    assert!(res.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn send_input_conflict_maps_to_agent_terminal() {
    let base = spawn(Router::new().route("/agents/:id/input", post(stub_input_conflict))).await;
    let res = tokio::task::spawn_blocking(move || Client::new(base, None).send_input("ag_1", "yes"))
        .await
        .unwrap();
    assert_eq!(res.unwrap_err(), PetError::AgentTerminal);
}
