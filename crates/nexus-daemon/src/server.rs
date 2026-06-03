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
use crate::scheduler;
use crate::store::AgentStore;
use crate::task_store::TaskStore;
use crate::tmux::TmuxManager;
use crate::worktree::{self, WorktreeManager};

/// How often the background monitor polls running agents.
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);

/// How often the scheduler reconciles tasks and fills free slots.
const SCHEDULER_INTERVAL: Duration = Duration::from_secs(2);

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: AgentStore,
    pub tasks: TaskStore,
    pub adapters: std::sync::Arc<AdapterRegistry>,
    /// When set, requests must present this bearer token. `None` disables auth.
    pub auth_token: Option<String>,
}

impl AppState {
    /// In-memory state (no persistence, no auth) — used by tests.
    pub fn new() -> Self {
        Self::with_stores(AgentStore::new(), TaskStore::new())
    }

    /// State backed by the given agent store, with an in-memory task store.
    pub fn with_store(store: AgentStore) -> Self {
        Self::with_stores(store, TaskStore::new())
    }

    /// State backed by the given stores (e.g. persistent ones).
    pub fn with_stores(store: AgentStore, tasks: TaskStore) -> Self {
        Self {
            store,
            tasks,
            adapters: std::sync::Arc::new(AdapterRegistry::with_defaults()),
            auth_token: None,
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

/// Where the daemon persists the task queue. Override with `NEXUS_TASKS`.
fn tasks_file_path() -> PathBuf {
    if let Ok(path) = std::env::var("NEXUS_TASKS") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".agentnexus").join("tasks.json");
    }
    PathBuf::from("nexus-tasks.json")
}

/// Max agents the scheduler runs at once. Override with `NEXUS_CONCURRENCY`.
fn concurrency() -> usize {
    std::env::var("NEXUS_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(scheduler::DEFAULT_CONCURRENCY)
}

/// Create an agent from `config`, start it, and return its id. Shared by the
/// create-agent endpoint and the scheduler.
pub fn spawn_started_agent(
    state: &AppState,
    config: &nexus_core::agent::AgentConfig,
    isolate: bool,
) -> Result<String> {
    let mut agent = nexus_core::agent::Agent::new(config.clone());
    agent.isolate = isolate;
    let id = agent.id.clone();
    state.store.insert(agent);
    start_agent_internal(state, &id)?;
    Ok(id)
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
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

/// Start the HTTP API server.
pub async fn run(addr: SocketAddr) -> Result<()> {
    let store = AgentStore::load_or_new(state_file_path())?;
    reconcile_startup(&store);
    let tasks = TaskStore::load_or_new(tasks_file_path())?;
    let mut state = AppState::with_stores(store, tasks);
    state.auth_token = std::env::var("NEXUS_TOKEN").ok().filter(|t| !t.is_empty());
    if state.auth_token.is_some() {
        info!("token authentication enabled");
    }

    // Background task: poll running agents and update their status/metrics.
    tokio::spawn(monitor::run_monitor(state.clone(), MONITOR_INTERVAL));

    // Background task: schedule queued tasks onto a bounded agent pool.
    tokio::spawn(scheduler::run_scheduler(
        state.clone(),
        SCHEDULER_INTERVAL,
        concurrency(),
    ));

    let app = build_app(state);

    info!("AgentNexus daemon listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await.map_err(NexusError::Io)?;

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

    // Launch the agent as the tmux session's main process, so the session ends
    // when the agent exits (a clean completion signal).
    let command = adapter.build_command(&config);
    TmuxManager::create_session(
        &agent.session_name,
        &run_dir.display().to_string(),
        &command,
    )?;

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

    // Mark Stopped *before* killing the session. The agent then leaves the
    // active set, so the monitor won't see the vanished session and misread the
    // kill as a natural completion.
    state.store.update_status(id, AgentStatus::Stopped);

    if TmuxManager::session_exists(&agent.session_name) {
        TmuxManager::kill_session(&agent.session_name)?;
    }

    Ok(())
}
