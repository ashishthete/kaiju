use axum::Router;
use nexus_adapters::AdapterRegistry;
use nexus_core::agent::AgentStatus;
use nexus_core::{NexusError, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::api;
use crate::monitor;
use crate::reconcile;
use crate::store::AgentStore;
use crate::tmux::TmuxManager;
use crate::worktree::{self, WorktreeManager};

/// How often the background monitor polls running agents.
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: AgentStore,
    pub adapters: std::sync::Arc<AdapterRegistry>,
}

impl AppState {
    /// In-memory state (no persistence) — used by tests.
    pub fn new() -> Self {
        Self::with_store(AgentStore::new())
    }

    /// State backed by the given store (e.g. a persistent one).
    pub fn with_store(store: AgentStore) -> Self {
        Self {
            store,
            adapters: std::sync::Arc::new(AdapterRegistry::with_defaults()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the daemon persists agent state. Override with `NEXUS_STATE`.
fn state_file_path() -> PathBuf {
    if let Ok(path) = std::env::var("NEXUS_STATE") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".agentnexus").join("state.json");
    }
    PathBuf::from("nexus-state.json")
}

/// Base directory for isolated agent worktrees. Override with `NEXUS_WORKTREES`.
fn worktrees_base() -> PathBuf {
    if let Ok(path) = std::env::var("NEXUS_WORKTREES") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".agentnexus").join("worktrees");
    }
    PathBuf::from("nexus-worktrees")
}

/// On startup, mark agents that were recorded active but whose tmux session is
/// gone (the daemon was down while they ended) as stopped.
fn reconcile_startup(store: &AgentStore) {
    let live = TmuxManager::list_nexus_sessions().unwrap_or_default();
    for id in reconcile::orphaned_active_ids(&store.list(), &live) {
        store.update_status(&id, AgentStatus::Stopped);
        info!("reconciled orphaned agent {id} as stopped");
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
    let store = AgentStore::load_or_new(state_file_path())?;
    reconcile_startup(&store);
    let state = AppState::with_store(store);

    // Background task: poll running agents and update their status/metrics.
    tokio::spawn(monitor::run_monitor(state.clone(), MONITOR_INTERVAL));

    let app = build_app(state);

    info!("AgentNexus daemon listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(NexusError::Io)?;

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

    // Determine where the agent actually runs: its own git worktree if
    // isolation was requested, otherwise the workspace directly.
    let run_dir = prepare_run_dir(state, &agent)?;

    let config = nexus_core::agent::AgentConfig {
        agent_type: agent.agent_type.clone(),
        model: agent.model.clone(),
        workspace: run_dir.clone(),
        prompt: agent.prompt.clone(),
        extra_args: agent.extra_args.clone(),
    };

    // Create tmux session
    TmuxManager::create_session(&agent.session_name, &run_dir.display().to_string())?;

    // Build and send the CLI command
    let command = adapter.build_command(&config);
    TmuxManager::send_keys(&agent.session_name, &command)?;

    state.store.mark_started(id, chrono::Utc::now());

    Ok(())
}

/// Resolve the working directory for an agent, creating a git worktree when
/// isolation is requested. Returns the workspace unchanged when not isolating.
fn prepare_run_dir(state: &AppState, agent: &nexus_core::agent::Agent) -> Result<PathBuf> {
    if !agent.isolate {
        return Ok(agent.workspace.clone());
    }

    if !WorktreeManager::is_git_repo(&agent.workspace) {
        return Err(NexusError::Git(format!(
            "cannot isolate: {} is not a git repository",
            agent.workspace.display()
        )));
    }

    let path = worktree::worktree_path(&worktrees_base(), &agent.id);
    let branch = worktree::branch_name(&agent.id);
    WorktreeManager::create(&agent.workspace, &path, &branch)?;
    state.store.set_worktree_path(&agent.id, path.clone());
    Ok(path)
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
