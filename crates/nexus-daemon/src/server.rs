use axum::Router;
use nexus_adapters::AdapterRegistry;
use nexus_core::agent::AgentStatus;
use nexus_core::{NexusError, Result};
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::api;
use crate::monitor;
use crate::store::AgentStore;
use crate::tmux::TmuxManager;

/// How often the background monitor polls running agents.
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: AgentStore,
    pub adapters: std::sync::Arc<AdapterRegistry>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            store: AgentStore::new(),
            adapters: std::sync::Arc::new(AdapterRegistry::with_defaults()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the Axum router with all routes, state, and middleware.
///
/// Separated from `run` so it can be exercised by integration tests
/// without binding a TCP socket.
pub fn build_app(state: AppState) -> Router {
    api::routes()
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

/// Start the HTTP API server.
pub async fn run(addr: SocketAddr) -> Result<()> {
    let state = AppState::new();

    // Background task: poll running agents and update their status/metrics.
    tokio::spawn(monitor::run_monitor(state.clone(), MONITOR_INTERVAL));

    let app = build_app(state);

    info!("AgentNexus daemon listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|e| NexusError::Io(e.into()))?;

    Ok(())
}

/// Internal helper: start an agent's tmux session and CLI process.
pub fn start_agent_internal(state: &AppState, id: &str) -> Result<()> {
    let agent = state
        .store
        .get(id)
        .ok_or_else(|| NexusError::AgentNotFound(id.to_string()))?;

    if agent.status.is_active() && agent.status != AgentStatus::Starting {
        return Err(NexusError::AlreadyRunning(id.to_string()));
    }

    let adapter = state
        .adapters
        .get(&agent.agent_type)
        .ok_or_else(|| NexusError::Adapter(format!("no adapter for {}", agent.agent_type)))?;

    let config = nexus_core::agent::AgentConfig {
        agent_type: agent.agent_type.clone(),
        model: agent.model.clone(),
        workspace: agent.workspace.clone(),
        prompt: agent.prompt.clone(),
        extra_args: agent.extra_args.clone(),
    };

    // Create tmux session
    TmuxManager::create_session(&agent.session_name, &agent.workspace.display().to_string())?;

    // Build and send the CLI command
    let command = adapter.build_command(&config);
    TmuxManager::send_keys(&agent.session_name, &command)?;

    state.store.mark_started(id, chrono::Utc::now());

    Ok(())
}

/// Internal helper: stop an agent by killing its tmux session.
pub fn stop_agent_internal(state: &AppState, id: &str) -> Result<()> {
    let agent = state
        .store
        .get(id)
        .ok_or_else(|| NexusError::AgentNotFound(id.to_string()))?;

    if agent.status.is_terminal() {
        return Err(NexusError::NotRunning(id.to_string()));
    }

    if TmuxManager::session_exists(&agent.session_name) {
        TmuxManager::kill_session(&agent.session_name)?;
    }

    state.store.update_status(id, AgentStatus::Stopped);

    Ok(())
}
