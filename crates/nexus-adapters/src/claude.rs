use nexus_core::adapter::{Adapter, ParsedOutput};
use nexus_core::agent::{AgentConfig, AgentStatus, AgentType};
use regex::Regex;

/// Adapter for Claude Code CLI.
///
/// Claude Code outputs structured status lines and cost information
/// that we parse to determine agent state.
pub struct ClaudeAdapter;

impl Adapter for ClaudeAdapter {
    fn agent_type(&self) -> AgentType {
        AgentType::Claude
    }

    fn default_model(&self) -> Option<&str> {
        Some("claude-opus-4-8")
    }

    fn build_command(&self, config: &AgentConfig) -> String {
        let mut cmd = format!("cd {} && claude", config.workspace.display());

        let model = config.model.as_deref().or(self.default_model());
        if let Some(model) = model {
            cmd.push_str(&format!(" --model {model}"));
        }

        if let Some(prompt) = &config.prompt {
            let escaped = prompt.replace('\'', "'\\''");
            cmd.push_str(&format!(" -p '{escaped}'"));
        }

        for arg in &config.extra_args {
            cmd.push_str(&format!(" {arg}"));
        }

        cmd
    }

    fn parse_output(&self, output: &str) -> ParsedOutput {
        let mut result = ParsedOutput::default();

        // Detect status from output patterns
        if output.contains("Task completed") || output.contains("Done!") {
            result.status = Some(AgentStatus::Completed);
        } else if output.contains("waiting for input")
            || output.contains("? ")
            || output.contains("Do you want")
        {
            result.status = Some(AgentStatus::WaitingForInput);
        } else if output.contains("Error:") || output.contains("error:") {
            result.status = Some(AgentStatus::Error);
        } else if output.contains("Working") || output.contains("Thinking") || output.contains("Reading") {
            result.status = Some(AgentStatus::Running);
        }

        // Parse cost: "Total cost: $1.42"
        let cost_re = Regex::new(r"Total cost:\s*\$(\d+\.?\d*)").unwrap();
        if let Some(caps) = cost_re.captures(output) {
            result.estimated_cost_usd = caps[1].parse().ok();
        }

        // Parse tokens: "Tokens used: 12345" or similar
        let token_re = Regex::new(r"[Tt]okens?\s*(?:used)?:?\s*(\d[\d,]*)").unwrap();
        if let Some(caps) = token_re.captures(output) {
            let cleaned = caps[1].replace(',', "");
            result.tokens_used = cleaned.parse().ok();
        }

        result
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config(prompt: Option<&str>) -> AgentConfig {
        AgentConfig {
            agent_type: AgentType::Claude,
            model: Some("sonnet".to_string()),
            workspace: PathBuf::from("/home/user/project"),
            prompt: prompt.map(|s| s.to_string()),
            extra_args: vec![],
        }
    }

    #[test]
    fn build_command_with_model_and_prompt() {
        let adapter = ClaudeAdapter;
        let cmd = adapter.build_command(&config(Some("fix the auth bug")));
        assert_eq!(
            cmd,
            "cd /home/user/project && claude --model sonnet -p 'fix the auth bug'"
        );
    }

    #[test]
    fn build_command_uses_default_model_when_none() {
        let adapter = ClaudeAdapter;
        let cfg = AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        };
        assert_eq!(
            adapter.build_command(&cfg),
            "cd /tmp && claude --model claude-opus-4-8"
        );
    }

    #[test]
    fn build_command_without_prompt() {
        let adapter = ClaudeAdapter;
        let cfg = AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec!["--verbose".to_string()],
        };
        assert_eq!(adapter.build_command(&cfg), "cd /tmp && claude --model claude-opus-4-8 --verbose");
    }

    #[test]
    fn build_command_escapes_single_quotes_in_prompt() {
        let adapter = ClaudeAdapter;
        let cmd = adapter.build_command(&config(Some("fix the user's login")));
        assert!(cmd.contains("user'\\''s"));
    }

    #[test]
    fn parse_completed_status() {
        let adapter = ClaudeAdapter;
        let output = "Some work output...\nTask completed successfully.";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::Completed));
    }

    #[test]
    fn parse_waiting_for_input() {
        let adapter = ClaudeAdapter;
        let output = "Do you want to apply these changes?";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
    }

    #[test]
    fn parse_error_status() {
        let adapter = ClaudeAdapter;
        let output = "Error: API rate limit exceeded";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::Error));
    }

    #[test]
    fn parse_running_status() {
        let adapter = ClaudeAdapter;
        let output = "Thinking about the problem...";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::Running));
    }

    #[test]
    fn parse_cost_from_output() {
        let adapter = ClaudeAdapter;
        let output = "Total cost: $1.42\nDone!";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.estimated_cost_usd, Some(1.42));
    }

    #[test]
    fn parse_tokens_from_output() {
        let adapter = ClaudeAdapter;
        let output = "Tokens used: 12,345";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.tokens_used, Some(12345));
    }

    #[test]
    fn parse_unknown_output_returns_none() {
        let adapter = ClaudeAdapter;
        let output = "random unrecognized output";
        let parsed = adapter.parse_output(output);
        assert!(parsed.status.is_none());
        assert!(parsed.tokens_used.is_none());
        assert!(parsed.estimated_cost_usd.is_none());
    }
}
