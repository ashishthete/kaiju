use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use nexus_core::agent::{AgentConfig, AgentType};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::server::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/:id", get(get_agent).delete(delete_agent))
        .route("/agents/:id/start", post(start_agent))
        .route("/agents/:id/stop", post(stop_agent))
        .route("/agents/:id/logs", get(get_logs))
        .route("/agents/:id/status", get(get_status))
        .route("/agents/:id/interrupt", post(interrupt_agent))
        .route("/agents/:id/input", post(send_input))
        .route("/health", get(health))
}

// -- Request / Response types --

#[derive(Deserialize)]
pub struct CreateAgentRequest {
    pub agent_type: String,
    pub model: Option<String>,
    pub workspace: String,
    pub prompt: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// If true, start the agent immediately after creation.
    #[serde(default)]
    pub auto_start: bool,
    /// If true, run the agent in its own git worktree (requires a git workspace).
    #[serde(default)]
    pub isolate: bool,
}

#[derive(Deserialize)]
pub struct SendInputRequest {
    /// Text to type into the agent's session, submitted with Enter.
    pub text: String,
}

#[derive(Serialize)]
pub struct AgentResponse {
    pub id: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub workspace: String,
    pub status: String,
    pub session_name: String,
    pub prompt: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metrics: MetricsResponse,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub runtime_secs: u64,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl From<&nexus_core::agent::Agent> for AgentResponse {
    fn from(agent: &nexus_core::agent::Agent) -> Self {
        Self {
            id: agent.id.clone(),
            agent_type: agent.agent_type.to_string(),
            model: agent.model.clone(),
            workspace: agent.workspace.display().to_string(),
            status: format!("{:?}", agent.status).to_lowercase(),
            session_name: agent.session_name.clone(),
            prompt: agent.prompt.clone(),
            created_at: agent.created_at.to_rfc3339(),
            updated_at: agent.updated_at.to_rfc3339(),
            metrics: MetricsResponse {
                runtime_secs: agent.metrics.runtime_secs,
                tokens_used: agent.metrics.tokens_used,
                estimated_cost_usd: agent.metrics.estimated_cost_usd,
            },
        }
    }
}

fn err(status: StatusCode, msg: &str) -> impl IntoResponse {
    (status, Json(ErrorResponse { error: msg.to_string() }))
}

// -- Handlers --

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.store.list();
    let responses: Vec<AgentResponse> = agents.iter().map(AgentResponse::from).collect();
    Json(responses)
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.get(&id) {
        Some(agent) => Ok(Json(AgentResponse::from(&agent))),
        None => Err(err(StatusCode::NOT_FOUND, "agent not found")),
    }
}

async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> impl IntoResponse {
    // parse() is infallible for AgentType (unknown strings become Custom),
    // but we still verify an adapter exists below.
    let agent_type: AgentType = req.agent_type.parse().expect("infallible");

    // Verify adapter exists
    if state.adapters.get(&agent_type).is_none() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            &format!("unsupported agent type: {}", req.agent_type),
        ));
    }

    let config = AgentConfig {
        agent_type,
        model: req.model,
        workspace: PathBuf::from(&req.workspace),
        prompt: req.prompt,
        extra_args: req.extra_args,
    };

    let mut agent = nexus_core::agent::Agent::new(config);
    agent.isolate = req.isolate;
    let id = agent.id.clone();
    state.store.insert(agent);

    if req.auto_start {
        if let Err(e) = crate::server::start_agent_internal(&state, &id) {
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()));
        }
    }

    let agent = state.store.get(&id).unwrap();
    Ok((StatusCode::CREATED, Json(AgentResponse::from(&agent))))
}

async fn start_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match crate::server::start_agent_internal(&state, &id) {
        Ok(()) => {
            let agent = state.store.get(&id).unwrap();
            Ok(Json(AgentResponse::from(&agent)))
        }
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn stop_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match crate::server::stop_agent_internal(&state, &id) {
        Ok(()) => {
            let agent = state.store.get(&id).unwrap();
            Ok(Json(AgentResponse::from(&agent)))
        }
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn interrupt_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    match crate::tmux::TmuxManager::send_interrupt(&agent.session_name) {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "interrupted" }))),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

/// Send a line of input (a follow-up message or approval) to a running agent.
async fn send_input(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendInputRequest>,
) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    // Input only makes sense for a live session.
    if agent.status.is_terminal() {
        return Err(err(StatusCode::CONFLICT, "agent is not running"));
    }

    // Typing input is the same tmux operation as sending the launch command:
    // the text followed by Enter. Reuse `send_keys` rather than duplicating it.
    match crate::tmux::TmuxManager::send_keys(&agent.session_name, &req.text) {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "sent" }))),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    match crate::tmux::TmuxManager::capture_pane(&agent.session_name, 200) {
        Ok(output) => Ok(Json(serde_json::json!({ "logs": output }))),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn get_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    Ok(Json(serde_json::json!({
        "id": agent.id,
        "status": format!("{:?}", agent.status).to_lowercase(),
        "runtime_secs": agent.metrics.runtime_secs,
        "tokens_used": agent.metrics.tokens_used,
        "estimated_cost_usd": agent.metrics.estimated_cost_usd,
    })))
}

async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Stop if running, then clean up any isolated worktree.
    if let Some(agent) = state.store.get(&id) {
        if agent.status.is_active() {
            let _ = crate::server::stop_agent_internal(&state, &id);
        }
        if let Some(worktree) = &agent.worktree_path {
            if let Err(e) =
                crate::worktree::WorktreeManager::remove(&agent.workspace, worktree)
            {
                tracing::warn!("failed to clean up worktree for agent {id}: {e}");
            }
        }
    }

    match state.store.remove(&id) {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err(err(StatusCode::NOT_FOUND, "agent not found")),
    }
}
