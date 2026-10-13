use axum::Router;
use kaiju_adapters::AdapterRegistry;
use kaiju_core::agent::AgentStatus;
use kaiju_core::{NexusError, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
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
    /// Defaults applied to every newly-created agent. Behind a lock so the
    /// Preferences endpoint can update it live (no daemon restart needed).
    pub settings: std::sync::Arc<std::sync::RwLock<crate::settings::Settings>>,
    /// Paired devices and their tokens. Mutable at runtime (pair / revoke).
    pub devices: std::sync::Arc<std::sync::RwLock<crate::devices::Devices>>,
    /// Outstanding one-time pairing codes (in-memory; lost on restart).
    pub pending_codes: std::sync::Arc<std::sync::Mutex<crate::pairing::PendingCodes>>,
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
            settings: std::sync::Arc::new(std::sync::RwLock::new(
                crate::settings::Settings::default(),
            )),
            devices: std::sync::Arc::new(
                std::sync::RwLock::new(crate::devices::Devices::default()),
            ),
            pending_codes: std::sync::Arc::new(std::sync::Mutex::new(
                crate::pairing::PendingCodes::default(),
            )),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the daemon persists agent state. Override with `KAIJU_STATE`.
fn state_file_path() -> PathBuf {
    if let Ok(path) = std::env::var("KAIJU_STATE") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".kaiju").join("state.json");
    }
    PathBuf::from("kaiju-state.json")
}

/// Base directory for isolated agent worktrees. Override with `KAIJU_WORKTREES`.
fn worktrees_base() -> PathBuf {
    if let Ok(path) = std::env::var("KAIJU_WORKTREES") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".kaiju").join("worktrees");
    }
    PathBuf::from("kaiju-worktrees")
}

/// Where the daemon persists the task queue. Override with `KAIJU_TASKS`.
fn tasks_file_path() -> PathBuf {
    if let Ok(path) = std::env::var("KAIJU_TASKS") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".kaiju").join("tasks.json");
    }
    PathBuf::from("kaiju-tasks.json")
}

/// Max agents the scheduler runs at once. Override with `KAIJU_CONCURRENCY`.
fn concurrency() -> usize {
    std::env::var("KAIJU_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(scheduler::DEFAULT_CONCURRENCY)
}

/// Create an agent from `config`, start it, and return its id. Shared by the
/// create-agent endpoint and the scheduler.
pub fn spawn_started_agent(
    state: &AppState,
    config: &kaiju_core::agent::AgentConfig,
    isolate: bool,
) -> Result<String> {
    let defaults = state.settings.read().expect("settings lock").clone();
    let config = defaults.apply(config.clone());
    let isolate = isolate || defaults.isolate;
    let mut agent = kaiju_core::agent::Agent::new(config);
    agent.isolate = isolate;
    agent.auto_continue = defaults.auto_continue;
    let id = agent.id.clone();
    state.store.insert(agent);
    start_agent_internal(state, &id)?;
    Ok(id)
}

/// On startup, mark agents that were recorded active but whose tmux session is
/// gone (the daemon was down while they ended) as stopped.
fn reconcile_startup(store: &AgentStore) {
    let live = TmuxManager::list_kaiju_sessions().unwrap_or_default();
    for id in reconcile::orphaned_active_ids(&store.list(), &live) {
        store.update_status(&id, AgentStatus::Stopped);
        info!("reconciled orphaned agent {id} as stopped");
    }
}

/// On startup, delete worktree directories under the managed base that no agent
/// references — leftovers from a crash. Best-effort; bounded to the daemon's own
/// worktrees directory.
fn sweep_orphaned_worktrees(store: &AgentStore) {
    let Ok(entries) = std::fs::read_dir(worktrees_base()) else {
        return;
    };
    let existing: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    for dir in reconcile::orphaned_worktree_dirs(&existing, &store.list()) {
        if std::fs::remove_dir_all(&dir).is_ok() {
            info!("removed orphaned worktree {}", dir.display());
        }
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
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

/// Start the HTTP API server.
pub async fn run(addr: SocketAddr) -> Result<()> {
    let state_path = state_file_path();
    // With no state file there is no record of which worktrees are in use (a
    // first run, or a throwaway daemon pointed at a fresh `KAIJU_STATE`), so
    // "unreferenced" proves nothing — don't delete anything.
    let has_state = state_path.exists();
    let store = AgentStore::load_or_new(state_path)?;
    reconcile_startup(&store);
    if has_state {
        sweep_orphaned_worktrees(&store);
    }
    let tasks = TaskStore::load_or_new(tasks_file_path())?;
    let mut state = AppState::with_stores(store, tasks);
    state.auth_token = std::env::var("KAIJU_TOKEN").ok().filter(|t| !t.is_empty());
    if state.auth_token.is_some() {
        info!("token authentication enabled");
    }
    state.settings = std::sync::Arc::new(std::sync::RwLock::new(crate::settings::load()));
    state.devices = std::sync::Arc::new(std::sync::RwLock::new(crate::devices::load()));

    // Background task: poll running agents and update their status/metrics.
    tokio::spawn(monitor::run_monitor(state.clone(), MONITOR_INTERVAL));

    // Background task: schedule queued tasks onto a bounded agent pool.
    tokio::spawn(scheduler::run_scheduler(
        state.clone(),
        SCHEDULER_INTERVAL,
        concurrency(),
    ));

    let app = build_app(state);

    info!("Kaiju daemon listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(NexusError::Io)?;

    Ok(())
}

/// Start an agent's tmux session and CLI process. A launch that fails marks the
/// agent `Error`, so it doesn't sit in `Starting` forever.
pub fn start_agent_internal(state: &AppState, id: &str) -> Result<()> {
    let result = launch_agent(state, id);
    if let Err(e) = &result {
        let launch_failed = !matches!(
            e,
            NexusError::AgentNotFound(_) | NexusError::AlreadyRunning(_)
        );
        if launch_failed {
            state.store.update_status(id, AgentStatus::Error);
        }
    }
    result
}

fn launch_agent(state: &AppState, id: &str) -> Result<()> {
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

    // Pre-trust the run directory in the CLI's own trust store so its first-run
    // folder-trust prompt doesn't stall an unattended / bake-off launch.
    adapter.ensure_trusted(&run_dir);

    let config = kaiju_core::agent::AgentConfig {
        agent_type: agent.agent_type.clone(),
        model: agent.model.clone(),
        workspace: run_dir.clone(),
        prompt: agent.prompt.clone(),
        extra_args: agent.extra_args.clone(),
    };

    // Batch mode: run the CLI's structured (non-interactive) command and drive
    // status/metrics from its event stream, instead of an interactive session.
    if agent.batch {
        let command = adapter.structured_command(&config).ok_or_else(|| {
            NexusError::Adapter(format!(
                "{} has no structured (batch) mode",
                agent.agent_type
            ))
        })?;
        state.store.mark_started(id, chrono::Utc::now());
        crate::batch::spawn_batch(
            state.clone(),
            id.to_string(),
            command,
            agent.agent_type.clone(),
        );
        return Ok(());
    }

    // Isolated agents get a private dev-server port so parallel worktrees don't
    // fight over the default port. Best-effort: if the range is exhausted the
    // agent still launches, just without an injected port.
    let port = if agent.isolate {
        let p = crate::port::allocate_port(&taken_ports(state));
        if let Some(p) = p {
            state.store.set_port(id, p);
        }
        p
    } else {
        None
    };

    // Launch the agent as the tmux session's main process, so the session ends
    // when the agent exits (a clean completion signal).
    let command = adapter.build_command(&config);
    TmuxManager::create_session(
        &agent.session_name,
        &run_dir.display().to_string(),
        &command,
        &session_env(port),
    )?;

    state.store.mark_started(id, chrono::Utc::now());

    Ok(())
}

/// Ports currently reserved by live agents, so a new agent never reuses one
/// whose dev server may still be running.
fn taken_ports(state: &AppState) -> std::collections::HashSet<u16> {
    state
        .store
        .list_active()
        .iter()
        .filter_map(|a| a.port)
        .collect()
}

/// Session environment for an agent's reserved port. `PORT` covers the common
/// dev servers (Next.js, CRA, Express); `KAIJU_AGENT_PORT` is the explicit
/// handle a project can wire up itself.
fn session_env(port: Option<u16>) -> Vec<(String, String)> {
    match port {
        Some(p) => vec![
            ("PORT".to_string(), p.to_string()),
            ("KAIJU_AGENT_PORT".to_string(), p.to_string()),
        ],
        None => Vec::new(),
    }
}

/// Resolve the working directory for an agent, creating a git worktree when
/// isolation is requested. Returns the workspace unchanged when not isolating.
fn prepare_run_dir(state: &AppState, agent: &kaiju_core::agent::Agent) -> Result<PathBuf> {
    if !agent.isolate {
        return Ok(agent.workspace.clone());
    }

    // Isolation needs a git repo to branch a worktree from; initialize one if
    // the workspace isn't already a repo (rather than refusing).
    WorktreeManager::ensure_repo(&agent.workspace)?;

    let path = worktree::worktree_path(&worktrees_base(), &agent.id);
    let branch = worktree::branch_name(&agent.id);
    WorktreeManager::create(&agent.workspace, &path, &branch)?;
    state.store.set_worktree_path(&agent.id, path.clone());
    Ok(path)
}

/// Internal helper: resume a previously-ended agent in its original directory,
/// reusing its CLI's resume mode (e.g. `claude --continue`).
///
/// Unlike start, it never creates a worktree — an isolated agent resumes in the
/// worktree it already has, and fails if that has been swept.
pub fn resume_agent_internal(state: &AppState, id: &str) -> Result<()> {
    let agent = state
        .store
        .get(id)
        .ok_or_else(|| NexusError::AgentNotFound(id.to_string()))?;

    if agent.status.is_active() {
        return Err(NexusError::AlreadyRunning(id.to_string()));
    }

    let adapter = state
        .adapters
        .get(&agent.agent_type)
        .ok_or_else(|| NexusError::Adapter(format!("no adapter for {}", agent.agent_type)))?;

    let run_dir = if agent.isolate {
        match &agent.worktree_path {
            Some(path) if path.exists() => path.clone(),
            _ => {
                return Err(NexusError::Git(format!(
                    "cannot resume {id}: its worktree no longer exists"
                )))
            }
        }
    } else {
        agent.workspace.clone()
    };

    let config = kaiju_core::agent::AgentConfig {
        agent_type: agent.agent_type.clone(),
        model: agent.model.clone(),
        workspace: run_dir.clone(),
        prompt: agent.prompt.clone(),
        extra_args: agent.extra_args.clone(),
    };

    let command = adapter.resume_command(&config).ok_or_else(|| {
        NexusError::Adapter(format!("{} does not support resume", agent.agent_type))
    })?;

    // Clear any lingering session for this name before recreating it.
    if TmuxManager::session_exists(&agent.session_name) {
        TmuxManager::kill_session(&agent.session_name)?;
    }

    TmuxManager::create_session(
        &agent.session_name,
        &run_dir.display().to_string(),
        &command,
        &session_env(agent.port),
    )?;
    state.store.mark_started(id, chrono::Utc::now());
    state.store.update_status(id, AgentStatus::Running);

    Ok(())
}

/// Create an agent that resumes an existing CLI session by id, spawn it, and
/// return its id. Like start, but the launch command is the adapter's
/// resume-by-id command. Never creates a worktree (resume runs in place).
pub fn adopt_agent_internal(
    state: &AppState,
    config: &kaiju_core::agent::AgentConfig,
    session_id: &str,
) -> Result<String> {
    let adapter = state
        .adapters
        .get(&config.agent_type)
        .ok_or_else(|| NexusError::Adapter(format!("no adapter for {}", config.agent_type)))?;

    let command = adapter
        .resume_session_command(config, session_id)
        .ok_or_else(|| {
            NexusError::Adapter(format!("{} does not support resume", config.agent_type))
        })?;

    let agent = kaiju_core::agent::Agent::new(config.clone());
    let id = agent.id.clone();
    let session_name = agent.session_name.clone();
    state.store.insert(agent);

    // Adopt resumes an existing session in place (no worktree), so no port is
    // reserved.
    TmuxManager::create_session(
        &session_name,
        &config.workspace.display().to_string(),
        &command,
        &[],
    )?;
    state.store.mark_started(&id, chrono::Utc::now());
    Ok(id)
}

/// Launch one isolated agent per CLI type, all running `prompt` in `workspace`,
/// tagged with a shared compare-group id. Returns (group_id, agent ids).
/// Comparison needs isolation (each run diffs its own worktree), so a non-git
/// workspace is rejected up front. Per-agent start is best-effort: a CLI that
/// fails to launch surfaces as that agent's error, not a whole-group failure.
pub fn spawn_compare_group(
    state: &AppState,
    workspace: &std::path::Path,
    prompt: &str,
    agent_types: &[String],
    model: Option<String>,
    auto_approve: bool,
) -> Result<(String, Vec<String>)> {
    // Each run is isolated in its own worktree, which needs a git repo to branch
    // from; initialize one if the workspace isn't already a repo.
    WorktreeManager::ensure_repo(workspace)?;
    let group_id = uuid::Uuid::new_v4().to_string();
    let defaults = state.settings.read().expect("settings lock").clone();
    let mut ids = Vec::new();
    for type_str in agent_types {
        let agent_type: kaiju_core::agent::AgentType = type_str.parse().expect("infallible");
        let extra_args = if auto_approve {
            state
                .adapters
                .get(&agent_type)
                .map(|a| a.auto_approve_args())
                .unwrap_or_default()
        } else {
            vec![]
        };
        let config = defaults.apply(kaiju_core::agent::AgentConfig {
            agent_type,
            model: model.clone(),
            workspace: workspace.to_path_buf(),
            prompt: Some(prompt.to_string()),
            extra_args,
        });
        let mut agent = kaiju_core::agent::Agent::new(config);
        agent.isolate = true;
        agent.compare_group = Some(group_id.clone());
        let id = agent.id.clone();
        state.store.insert(agent);
        let _ = start_agent_internal(state, &id);
        ids.push(id);
    }
    Ok((group_id, ids))
}

/// Result of promoting a run into its workspace.
pub enum PromoteOutcome {
    Merged { branch: String },
    NothingToPromote,
    WorkspaceDirty,
    Conflict { branch: String },
}

/// Commit an isolated run's changes on its branch and merge that branch into the
/// workspace's current branch. Non-destructive on failure: refuses a dirty
/// workspace up front and aborts a conflicting merge.
pub fn promote_agent_internal(state: &AppState, id: &str) -> Result<PromoteOutcome> {
    use crate::worktree::MergeOutcome;
    let agent = state
        .store
        .get(id)
        .ok_or_else(|| NexusError::AgentNotFound(id.to_string()))?;
    if agent.status.is_active() {
        return Err(NexusError::Git(
            "stop the run (or let it finish) before promoting — it's still active".to_string(),
        ));
    }
    let worktree = match (&agent.isolate, &agent.worktree_path) {
        (true, Some(p)) if p.exists() => p.clone(),
        _ => {
            return Err(NexusError::Git(
                "only an isolated run with a live worktree can be promoted".to_string(),
            ))
        }
    };
    if !WorktreeManager::is_clean(&agent.workspace)? {
        return Ok(PromoteOutcome::WorkspaceDirty);
    }
    let branch = worktree::branch_name(&agent.id);
    let short = &agent.id[..agent.id.len().min(8)];
    let made = WorktreeManager::commit_all(
        &worktree,
        &format!("kaiju: promote {} run {short}", agent.agent_type),
    )?;
    match WorktreeManager::merge_branch(
        &agent.workspace,
        &branch,
        &format!("kaiju: promote run {short}"),
    )? {
        MergeOutcome::Merged => Ok(PromoteOutcome::Merged { branch }),
        MergeOutcome::AlreadyUpToDate => {
            if made {
                Ok(PromoteOutcome::Merged { branch })
            } else {
                Ok(PromoteOutcome::NothingToPromote)
            }
        }
        MergeOutcome::Conflict => Ok(PromoteOutcome::Conflict { branch }),
    }
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
