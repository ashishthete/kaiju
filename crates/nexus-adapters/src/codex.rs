use nexus_core::adapter::{Adapter, ParsedOutput};
use nexus_core::agent::{AgentConfig, AgentStatus, AgentType};
use regex::Regex;

/// Adapter for OpenAI Codex CLI.
pub struct CodexAdapter;

impl Adapter for CodexAdapter {
    fn agent_type(&self) -> AgentType {
        AgentType::Codex
    }

    fn default_model(&self) -> Option<&str> {
        Some("o3")
    }

    fn build_command(&self, config: &AgentConfig) -> String {
        let mut cmd = format!("cd {} && codex", config.workspace.display());

        let model = config.model.as_deref().or(self.default_model());
        if let Some(model) = model {
            cmd.push_str(&format!(" --model {model}"));
        }

        if let Some(prompt) = &config.prompt {
            let escaped = prompt.replace('\'', "'\\''");
            cmd.push_str(&format!(" '{escaped}'"));
        }

        for arg in &config.extra_args {
            cmd.push_str(&format!(" {arg}"));
        }

        cmd
    }

    fn parse_output(&self, output: &str) -> ParsedOutput {
        let mut result = ParsedOutput::default();

        if output.contains("completed") || output.contains("Finished") {
            result.status = Some(AgentStatus::Completed);
        } else if output.contains("approve") || output.contains("confirm") || output.contains("(y/n)") {
            result.status = Some(AgentStatus::WaitingForInput);
        } else if output.contains("error") || output.contains("failed") {
            result.status = Some(AgentStatus::Error);
        } else if output.contains("running") || output.contains("processing") || output.contains("generating") {
            result.status = Some(AgentStatus::Running);
        }

        let cost_re = Regex::new(r"\$(\d+\.?\d*)").unwrap();
        if let Some(caps) = cost_re.captures(output) {
            result.estimated_cost_usd = caps[1].parse().ok();
        }

        result
    }

    fn display_name(&self) -> &str {
        "Codex CLI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_config() -> AgentConfig {
        AgentConfig {
            agent_type: AgentType::Codex,
            model: Some("o3".to_string()),
            workspace: PathBuf::from("/home/user/repo"),
            prompt: Some("add unit tests".to_string()),
            extra_args: vec![],
        }
    }

    #[test]
    fn build_command_with_prompt() {
        let adapter = CodexAdapter;
        let cmd = adapter.build_command(&make_config());
        assert_eq!(cmd, "cd /home/user/repo && codex --model o3 'add unit tests'");
    }

    #[test]
    fn build_command_uses_default_model_when_none() {
        let adapter = CodexAdapter;
        let cfg = AgentConfig {
            agent_type: AgentType::Codex,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        };
        assert_eq!(adapter.build_command(&cfg), "cd /tmp && codex --model o3");
    }

    #[test]
    fn parse_approval_waiting() {
        let adapter = CodexAdapter;
        let parsed = adapter.parse_output("Please approve these changes (y/n)");
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
    }

    #[test]
    fn parse_completed() {
        let adapter = CodexAdapter;
        let parsed = adapter.parse_output("Task completed successfully");
        assert_eq!(parsed.status, Some(AgentStatus::Completed));
    }

    #[test]
    fn parse_cost() {
        let adapter = CodexAdapter;
        let parsed = adapter.parse_output("Cost: $0.85");
        assert_eq!(parsed.estimated_cost_usd, Some(0.85));
    }
}
