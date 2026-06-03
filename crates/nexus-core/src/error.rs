use thiserror::Error;

#[derive(Error, Debug)]
pub enum NexusError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("tmux error: {0}")]
    Tmux(String),

    #[error("adapter error: {0}")]
    Adapter(String),

    #[error("spawn failed for agent {agent_id}: {reason}")]
    SpawnFailed { agent_id: String, reason: String },

    #[error("agent {0} is already running")]
    AlreadyRunning(String),

    #[error("agent {0} is not running")]
    NotRunning(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
