use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use kaiju_core::agent::{AgentConfig, AgentType};
use kaiju_core::task::{Task, TaskSpec};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::server::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/:id", get(get_agent).delete(delete_agent))
        .route("/agents/:id/start", post(start_agent))
        .route("/agents/:id/resume", post(resume_agent))
        .route("/agents/:id/stop", post(stop_agent))
        .route("/agents/:id/logs", get(get_logs))
        .route("/agents/:id/diff", get(get_diff))
        .route("/agents/:id/status", get(get_status))
        .route("/agents/:id/interrupt", post(interrupt_agent))
        .route("/agents/:id/input", post(send_input))
        .route(
            "/agents/:id/files",
            post(crate::files::upload_file)
                .layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .route("/agents/:id/terminal/ws", get(crate::terminal::terminal_ws))
        .route(
            "/agents/:id/terminal/size",
            get(crate::terminal::terminal_size).post(crate::terminal::terminal_resize),
        )
        .route("/assets/xterm.js", get(crate::terminal::xterm_js))
        .route("/assets/xterm.css", get(crate::terminal::xterm_css))
        .route("/assets/dashboard.js", get(crate::dashboard::dashboard_js))
        .route(
            "/assets/dashboard-utils.js",
            get(crate::dashboard::dashboard_utils_js),
        )
        .route("/settings", get(get_settings).put(put_settings))
        .route("/pair", get(crate::pair_api::pair_page))
        .route("/pair/code", post(crate::pair_api::pair_code))
        .route("/pair/claim", post(crate::pair_api::pair_claim))
        .route("/devices", get(crate::pair_api::list_devices))
        .route(
            "/devices/:id",
            axum::routing::delete(crate::pair_api::revoke_device),
        )
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/:id", get(get_task))
        .route("/tasks/:id/cancel", post(cancel_task))
        .route("/sessions", get(list_sessions))
        .route("/agents/adopt", post(adopt_agent))
        .route("/compare", post(compare))
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
    /// If true, run non-interactively via the CLI's structured (stream-json)
    /// mode. Requires a prompt and an adapter that supports it.
    #[serde(default)]
    pub batch: bool,
}

#[derive(Deserialize)]
pub struct SendInputRequest {
    /// Text to type into the agent's session, submitted with Enter.
    pub text: String,
}

#[derive(Deserialize)]
pub struct AdoptRequest {
    pub agent_type: String,
    pub workspace: String,
    pub session_id: String,
    pub model: Option<String>,
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub workspace: String,
    #[serde(rename = "type")]
    pub agent_type: String,
}

#[derive(Deserialize)]
pub struct CompareRequest {
    pub workspace: String,
    pub prompt: String,
    pub agent_types: Vec<String>,
    pub model: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub agent_type: String,
    pub model: Option<String>,
    pub workspace: String,
    pub prompt: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub isolate: bool,
}

#[derive(Serialize)]
pub struct TaskResponse {
    pub id: String,
    pub status: String,
    pub agent_type: String,
    pub workspace: String,
    pub prompt: Option<String>,
    pub agent_id: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&Task> for TaskResponse {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.clone(),
            status: format!("{:?}", task.status).to_lowercase(),
            agent_type: task.spec.agent_type.to_string(),
            workspace: task.spec.workspace.display().to_string(),
            prompt: task.spec.prompt.clone(),
            agent_id: task.agent_id.clone(),
            error: task.error.clone(),
            created_at: task.created_at.to_rfc3339(),
            updated_at: task.updated_at.to_rfc3339(),
        }
    }
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
    pub compare_group: Option<String>,
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

impl From<&kaiju_core::agent::Agent> for AgentResponse {
    fn from(agent: &kaiju_core::agent::Agent) -> Self {
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
            compare_group: agent.compare_group.clone(),
        }
    }
}

/// `GET /settings` — the current daemon defaults (Preferences).
async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.settings.read().expect("settings lock").clone();
    Json(snapshot)
}

/// `PUT /settings` — persist new defaults and apply them live. They take effect
/// for agents created *after* the change; running agents are unaffected.
async fn put_settings(
    State(state): State<AppState>,
    Json(new_settings): Json<crate::settings::Settings>,
) -> impl IntoResponse {
    if let Err(e) = crate::settings::save(&new_settings) {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()));
    }
    *state.settings.write().expect("settings lock") = new_settings.clone();
    Ok(Json(new_settings))
}

fn err(status: StatusCode, msg: &str) -> impl IntoResponse {
    (
        status,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

// -- Handlers --

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// The live fleet dashboard (polls `/agents` from the browser).
async fn dashboard() -> Html<&'static str> {
    Html(crate::dashboard::PAGE)
}

// -- Task queue handlers --

async fn list_tasks(State(state): State<AppState>) -> impl IntoResponse {
    let tasks = state.tasks.list();
    let responses: Vec<TaskResponse> = tasks.iter().map(TaskResponse::from).collect();
    Json(responses)
}

async fn get_task(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.tasks.get(&id) {
        Some(task) => Ok(Json(TaskResponse::from(&task))),
        None => Err(err(StatusCode::NOT_FOUND, "task not found")),
    }
}

async fn create_task(
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    if req.agent_type.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "agent type must not be empty"));
    }
    // Any non-builtin type is treated as a custom CLI (the type name is the binary).
    let agent_type: AgentType = req.agent_type.parse().expect("infallible");

    let spec = TaskSpec {
        agent_type,
        model: req.model,
        workspace: PathBuf::from(&req.workspace),
        prompt: req.prompt,
        extra_args: req.extra_args,
        isolate: req.isolate,
    };

    let task = state.tasks.enqueue(spec);
    Ok((StatusCode::CREATED, Json(TaskResponse::from(&task))))
}

async fn cancel_task(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    if state.tasks.get(&id).is_none() {
        return Err(err(StatusCode::NOT_FOUND, "task not found"));
    }

    match state.tasks.cancel(&id) {
        Some(task) => {
            // If the task was already running, stop its agent too.
            if let Some(agent_id) = &task.agent_id {
                let _ = crate::server::stop_agent_internal(&state, agent_id);
            }
            Ok(Json(TaskResponse::from(&task)))
        }
        None => Err(err(StatusCode::CONFLICT, "task already finished")),
    }
}

async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.store.list();
    let responses: Vec<AgentResponse> = agents.iter().map(AgentResponse::from).collect();
    Json(responses)
}

async fn get_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.store.get(&id) {
        Some(agent) => Ok(Json(AgentResponse::from(&agent))),
        None => Err(err(StatusCode::NOT_FOUND, "agent not found")),
    }
}

async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> impl IntoResponse {
    let defaults = state.settings.read().expect("settings lock").clone();
    // Fall back to the configured default agent type when none is given.
    let type_str = if req.agent_type.trim().is_empty() {
        defaults.default_agent_type.clone().unwrap_or_default()
    } else {
        req.agent_type.clone()
    };
    if type_str.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "agent type must not be empty"));
    }
    // parse() is infallible: builtins map to their adapter, any other non-blank
    // string becomes a custom CLI (the type name is the executable).
    let agent_type: AgentType = type_str.parse().expect("infallible");

    // Apply global defaults (model, extra args) for fields the request omits.
    let config = defaults.apply(AgentConfig {
        agent_type,
        model: req.model,
        workspace: PathBuf::from(&req.workspace),
        prompt: req.prompt,
        extra_args: req.extra_args,
    });

    let mut agent = kaiju_core::agent::Agent::new(config);
    agent.isolate = req.isolate || defaults.isolate;
    agent.batch = req.batch;
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

async fn start_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match crate::server::start_agent_internal(&state, &id) {
        Ok(()) => {
            let agent = state.store.get(&id).unwrap();
            Ok(Json(AgentResponse::from(&agent)))
        }
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn resume_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    use kaiju_core::NexusError;
    match crate::server::resume_agent_internal(&state, &id) {
        Ok(()) => {
            let agent = state.store.get(&id).unwrap();
            Ok(Json(AgentResponse::from(&agent)))
        }
        Err(e) => {
            let code = match e {
                NexusError::AgentNotFound(_) => StatusCode::NOT_FOUND,
                NexusError::AlreadyRunning(_) => StatusCode::CONFLICT,
                NexusError::Adapter(_) | NexusError::Git(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(err(code, &e.to_string()))
        }
    }
}

async fn stop_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
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

async fn get_logs(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    // Live capture while the session is up; otherwise the last persisted output
    // (the session ended, so its pane is gone).
    if let Ok(output) = crate::tmux::TmuxManager::capture_pane(&agent.session_name, 200) {
        return Ok(Json(serde_json::json!({ "logs": output })));
    }
    match crate::logstore::load(&id) {
        Some(logs) => Ok(Json(serde_json::json!({ "logs": logs }))),
        None => Err(err(
            StatusCode::NOT_FOUND,
            "no logs — session ended and nothing was captured",
        )),
    }
}

/// Show the changes the agent has made in its run directory.
async fn get_diff(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err(err(StatusCode::NOT_FOUND, "agent not found")),
    };

    match crate::worktree::WorktreeManager::diff(agent.run_dir()) {
        Ok(diff) => Ok(Json(serde_json::json!({ "diff": diff }))),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

async fn get_status(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
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

async fn delete_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    // Stop if running, then clean up any isolated worktree.
    if let Some(agent) = state.store.get(&id) {
        if agent.status.is_active() {
            let _ = crate::server::stop_agent_internal(&state, &id);
        }
        if let Some(worktree) = &agent.worktree_path {
            if let Err(e) = crate::worktree::WorktreeManager::remove(&agent.workspace, worktree) {
                tracing::warn!("failed to clean up worktree for agent {id}: {e}");
            }
        }
        crate::logstore::remove(&id);
    }

    match state.store.remove(&id) {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err(err(StatusCode::NOT_FOUND, "agent not found")),
    }
}

/// `GET /sessions?workspace=<path>&type=<agent_type>` — resumable CLI sessions
/// the adapter can discover for that workspace (empty if it can't).
async fn list_sessions(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SessionsQuery>,
) -> impl IntoResponse {
    let agent_type: AgentType = match q.agent_type.parse() {
        Ok(t) => t,
        Err(_) => return Json(Vec::<kaiju_core::adapter::SessionInfo>::new()),
    };
    let sessions = match state.adapters.get(&agent_type) {
        Some(adapter) => adapter.list_sessions(std::path::Path::new(&q.workspace)),
        None => Vec::new(),
    };
    Json(sessions)
}

/// `POST /agents/adopt` — create an agent that resumes an existing session by id.
async fn adopt_agent(
    State(state): State<AppState>,
    Json(req): Json<AdoptRequest>,
) -> impl IntoResponse {
    use kaiju_core::NexusError;
    if req.agent_type.trim().is_empty()
        || req.workspace.trim().is_empty()
        || req.session_id.trim().is_empty()
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "agent_type, workspace, and session_id are required",
        ));
    }
    // Defense-in-depth: the session id is interpolated into a shell command, so
    // restrict it to a safe character class (real CLI session ids are UUIDs).
    if !req
        .session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(err(StatusCode::BAD_REQUEST, "invalid session_id"));
    }
    // parse() is infallible: any non-blank string becomes a custom CLI type.
    let agent_type: AgentType = req.agent_type.parse().expect("infallible");
    // Apply global defaults (model, extra args) just like create_agent, so an
    // adopted agent honors the same Preferences as a freshly-created one.
    let defaults = state.settings.read().expect("settings lock").clone();
    let config = defaults.apply(AgentConfig {
        agent_type,
        model: req.model,
        workspace: PathBuf::from(&req.workspace),
        prompt: None,
        extra_args: vec![],
    });
    match crate::server::adopt_agent_internal(&state, &config, &req.session_id) {
        Ok(id) => {
            let agent = state.store.get(&id).unwrap();
            Ok((StatusCode::CREATED, Json(AgentResponse::from(&agent))))
        }
        Err(e) => {
            let code = match e {
                NexusError::Adapter(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(err(code, &e.to_string()))
        }
    }
}

/// `POST /compare` — run one prompt across several CLIs, each isolated, grouped.
async fn compare(
    State(state): State<AppState>,
    Json(req): Json<CompareRequest>,
) -> impl IntoResponse {
    use kaiju_core::NexusError;
    if req.workspace.trim().is_empty() || req.prompt.trim().is_empty() || req.agent_types.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "workspace, prompt, and at least one agent_type are required",
        ));
    }
    match crate::server::spawn_compare_group(
        &state,
        std::path::Path::new(&req.workspace),
        &req.prompt,
        &req.agent_types,
        req.model,
    ) {
        Ok((group_id, ids)) => {
            let agents: Vec<AgentResponse> = ids
                .iter()
                .filter_map(|id| state.store.get(id).map(|a| AgentResponse::from(&a)))
                .collect();
            Ok((
                StatusCode::CREATED,
                Json(serde_json::json!({ "group_id": group_id, "agents": agents })),
            ))
        }
        Err(e) => {
            let code = match e {
                NexusError::Git(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(err(code, &e.to_string()))
        }
    }
}
