use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Claude,
    Codex,
    Gemini,
    Custom(String),
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::Claude => write!(f, "claude"),
            AgentType::Codex => write!(f, "codex"),
            AgentType::Gemini => write!(f, "gemini"),
            AgentType::Custom(name) => write!(f, "{name}"),
        }
    }
}

impl std::str::FromStr for AgentType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "claude" => AgentType::Claude,
            "codex" => AgentType::Codex,
            "gemini" => AgentType::Gemini,
            other => AgentType::Custom(other.to_string()),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Running,
    WaitingForInput,
    Error,
    Completed,
    Stuck,
    Stopped,
}

impl AgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentStatus::Completed | AgentStatus::Stopped | AgentStatus::Error)
    }

    pub fn is_active(&self) -> bool {
        matches!(self, AgentStatus::Starting | AgentStatus::Running | AgentStatus::WaitingForInput)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetrics {
    pub runtime_secs: u64,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

impl Default for AgentMetrics {
    fn default() -> Self {
        Self {
            runtime_secs: 0,
            tokens_used: None,
            estimated_cost_usd: None,
        }
    }
}

/// Configuration for spawning a new agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_type: AgentType,
    pub model: Option<String>,
    pub workspace: PathBuf,
    pub prompt: Option<String>,
    pub extra_args: Vec<String>,
}

/// A running or completed agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub agent_type: AgentType,
    pub model: Option<String>,
    pub workspace: PathBuf,
    pub status: AgentStatus,
    pub session_name: String,
    pub prompt: Option<String>,
    pub extra_args: Vec<String>,
    pub created_at: DateTime<Utc>,
    /// Set when the agent's CLI process is actually launched. `None` until started.
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub metrics: AgentMetrics,
}

impl Agent {
    pub fn new(config: AgentConfig) -> Self {
        let id = Uuid::new_v4().to_string();
        let session_name = format!("nexus-{}-{}", config.agent_type, &id[..8]);
        let now = Utc::now();

        Self {
            id,
            agent_type: config.agent_type,
            model: config.model,
            workspace: config.workspace,
            status: AgentStatus::Starting,
            session_name,
            prompt: config.prompt,
            extra_args: config.extra_args,
            created_at: now,
            started_at: None,
            updated_at: now,
            metrics: AgentMetrics::default(),
        }
    }

    /// Mark the agent as started: record the launch time and move to Running.
    pub fn mark_started(&mut self, now: DateTime<Utc>) {
        self.started_at = Some(now);
        self.status = AgentStatus::Running;
        self.updated_at = now;
    }

    pub fn update_status(&mut self, status: AgentStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    pub fn update_metrics(&mut self, metrics: AgentMetrics) {
        self.metrics = metrics;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn agent_type_roundtrip() {
        let types = vec![
            ("claude", AgentType::Claude),
            ("codex", AgentType::Codex),
            ("gemini", AgentType::Gemini),
        ];
        for (s, expected) in types {
            let parsed: AgentType = s.parse().unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn custom_agent_type_roundtrip() {
        let parsed: AgentType = "mycli".parse().unwrap();
        assert_eq!(parsed, AgentType::Custom("mycli".to_string()));
        assert_eq!(parsed.to_string(), "mycli");
    }

    #[test]
    fn agent_status_terminal_vs_active() {
        assert!(AgentStatus::Completed.is_terminal());
        assert!(AgentStatus::Stopped.is_terminal());
        assert!(AgentStatus::Error.is_terminal());
        assert!(!AgentStatus::Running.is_terminal());

        assert!(AgentStatus::Starting.is_active());
        assert!(AgentStatus::Running.is_active());
        assert!(AgentStatus::WaitingForInput.is_active());
        assert!(!AgentStatus::Completed.is_active());
    }

    #[test]
    fn new_agent_has_starting_status() {
        let agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: Some("sonnet".to_string()),
            workspace: PathBuf::from("/tmp/project"),
            prompt: Some("fix the bug".to_string()),
            extra_args: vec![],
        });

        assert_eq!(agent.status, AgentStatus::Starting);
        assert!(agent.session_name.starts_with("nexus-claude-"));
        assert_eq!(agent.metrics.runtime_secs, 0);
        assert!(agent.metrics.tokens_used.is_none());
        assert!(agent.started_at.is_none());
    }

    #[test]
    fn new_agent_preserves_extra_args() {
        let agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec!["--verbose".to_string(), "--dangerously".to_string()],
        });

        assert_eq!(agent.extra_args, vec!["--verbose", "--dangerously"]);
    }

    #[test]
    fn mark_started_sets_started_at_and_running() {
        let mut agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        });
        let now = Utc::now();

        agent.mark_started(now);

        assert_eq!(agent.started_at, Some(now));
        assert_eq!(agent.status, AgentStatus::Running);
        assert_eq!(agent.updated_at, now);
    }

    #[test]
    fn update_status_changes_updated_at() {
        let mut agent = Agent::new(AgentConfig {
            agent_type: AgentType::Codex,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        });

        let before = agent.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        agent.update_status(AgentStatus::Running);

        assert_eq!(agent.status, AgentStatus::Running);
        assert!(agent.updated_at >= before);
    }

    #[test]
    fn agent_serializes_to_json() {
        let agent = Agent::new(AgentConfig {
            agent_type: AgentType::Gemini,
            model: Some("pro".to_string()),
            workspace: PathBuf::from("/home/user/project"),
            prompt: None,
            extra_args: vec![],
        });

        let json = serde_json::to_string(&agent).unwrap();
        let deserialized: Agent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.agent_type, AgentType::Gemini);
        assert_eq!(deserialized.model, Some("pro".to_string()));
    }
}
