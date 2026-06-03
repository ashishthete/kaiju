use crate::agent::{AgentConfig, AgentStatus, AgentType};

/// Output from parsing a CLI's terminal output.
#[derive(Debug, Clone, Default)]
pub struct ParsedOutput {
    pub status: Option<AgentStatus>,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

/// Trait that each CLI adapter must implement.
///
/// Adapters know how to:
/// 1. Build the shell command to launch a CLI agent
/// 2. Parse the CLI's terminal output to extract status and metrics
pub trait Adapter: Send + Sync {
    /// Which agent type this adapter handles.
    fn agent_type(&self) -> AgentType;

    /// Build the shell command string to spawn this agent in a tmux session.
    ///
    /// Returns a full command string ready for `tmux send-keys`.
    fn build_command(&self, config: &AgentConfig) -> String;

    /// Parse captured terminal output to extract status and metrics.
    ///
    /// Called periodically by the daemon with the latest pane output.
    /// Return `ParsedOutput::default()` if nothing useful can be extracted.
    fn parse_output(&self, output: &str) -> ParsedOutput;

    /// Human-readable name for this adapter.
    fn display_name(&self) -> &str;

    /// Default model to use when none is specified.
    /// Returns `None` if the CLI should pick its own default.
    fn default_model(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A mock adapter for testing the trait interface.
    struct MockAdapter;

    impl Adapter for MockAdapter {
        fn agent_type(&self) -> AgentType {
            AgentType::Custom("mock".to_string())
        }

        fn build_command(&self, config: &AgentConfig) -> String {
            format!("mock-cli --workspace {}", config.workspace.display())
        }

        fn parse_output(&self, output: &str) -> ParsedOutput {
            if output.contains("done") {
                ParsedOutput {
                    status: Some(AgentStatus::Completed),
                    ..Default::default()
                }
            } else {
                ParsedOutput::default()
            }
        }

        fn display_name(&self) -> &str {
            "Mock CLI"
        }
    }

    #[test]
    fn adapter_trait_is_object_safe() {
        let adapter: Box<dyn Adapter> = Box::new(MockAdapter);
        assert_eq!(adapter.agent_type(), AgentType::Custom("mock".to_string()));
        assert_eq!(adapter.display_name(), "Mock CLI");
    }

    #[test]
    fn adapter_builds_command() {
        let adapter = MockAdapter;
        let config = AgentConfig {
            agent_type: AgentType::Custom("mock".to_string()),
            model: None,
            workspace: PathBuf::from("/tmp/project"),
            prompt: None,
            extra_args: vec![],
        };

        let cmd = adapter.build_command(&config);
        assert_eq!(cmd, "mock-cli --workspace /tmp/project");
    }

    #[test]
    fn adapter_parses_completed_output() {
        let adapter = MockAdapter;
        let parsed = adapter.parse_output("task done successfully");
        assert_eq!(parsed.status, Some(AgentStatus::Completed));
    }

    #[test]
    fn adapter_returns_none_for_unknown_output() {
        let adapter = MockAdapter;
        let parsed = adapter.parse_output("working on something...");
        assert!(parsed.status.is_none());
    }
}
