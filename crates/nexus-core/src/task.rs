use crate::agent::{AgentConfig, AgentType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Lifecycle of a queued task.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Canceled
        )
    }
}

/// What to run when a task is scheduled — mirrors the inputs to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub agent_type: AgentType,
    pub model: Option<String>,
    pub workspace: PathBuf,
    pub prompt: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub isolate: bool,
}

impl TaskSpec {
    /// Build the agent configuration this task should spawn.
    pub fn to_config(&self) -> AgentConfig {
        AgentConfig {
            agent_type: self.agent_type.clone(),
            model: self.model.clone(),
            workspace: self.workspace.clone(),
            prompt: self.prompt.clone(),
            extra_args: self.extra_args.clone(),
        }
    }
}

/// A unit of queued work and its progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub spec: TaskSpec,
    pub status: TaskStatus,
    /// The agent spawned for this task, once it starts running.
    pub agent_id: Option<String>,
    /// Failure reason, if it failed.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(spec: TaskSpec) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            spec,
            status: TaskStatus::Queued,
            agent_id: None,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Task has been scheduled onto an agent.
    pub fn mark_running(&mut self, agent_id: String) {
        self.status = TaskStatus::Running;
        self.agent_id = Some(agent_id);
        self.updated_at = Utc::now();
    }

    /// Task reached a terminal outcome (done/canceled).
    pub fn finish(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Task failed with a reason.
    pub fn fail(&mut self, reason: String) {
        self.status = TaskStatus::Failed;
        self.error = Some(reason);
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TaskSpec {
        TaskSpec {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp/project"),
            prompt: Some("do the thing".to_string()),
            extra_args: vec![],
            isolate: true,
        }
    }

    #[test]
    fn new_task_is_queued() {
        let task = Task::new(spec());
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.agent_id.is_none());
        assert!(!task.status.is_terminal());
    }

    #[test]
    fn mark_running_sets_agent_and_status() {
        let mut task = Task::new(spec());
        task.mark_running("agent-123".to_string());
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.agent_id.as_deref(), Some("agent-123"));
    }

    #[test]
    fn fail_records_reason() {
        let mut task = Task::new(spec());
        task.fail("boom".to_string());
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("boom"));
        assert!(task.status.is_terminal());
    }

    #[test]
    fn spec_to_config_preserves_fields() {
        let config = spec().to_config();
        assert_eq!(config.agent_type, AgentType::Claude);
        assert_eq!(config.prompt.as_deref(), Some("do the thing"));
    }

    #[test]
    fn terminal_statuses() {
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Canceled.is_terminal());
        assert!(!TaskStatus::Queued.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }
}
