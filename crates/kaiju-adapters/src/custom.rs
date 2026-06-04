//! Generic adapter for any CLI not covered by a built-in adapter.
//!
//! For `AgentType::Custom(name)`, the `name` is treated as the executable. The
//! command mirrors the built-in adapters (optional `--model`, positional
//! prompt, extra args), which covers the common case; use `extra_args` for CLIs
//! that need a different invocation. Status is inferred with the shared, CLI-
//! agnostic heuristics; completion is detected by the daemon when the session
//! ends (the agent is the session's main process).

use kaiju_core::adapter::{
    controlling_prompt_line, ends_with_selection_menu, looks_like_prompt, Adapter, ParsedOutput,
};
use kaiju_core::agent::{AgentConfig, AgentStatus, AgentType};

/// Adapter used for any `AgentType::Custom(_)`.
pub struct CustomAdapter;

impl Adapter for CustomAdapter {
    fn agent_type(&self) -> AgentType {
        // Sentinel: the registry routes all `Custom(_)` types here directly, so
        // this is never used as a registration key.
        AgentType::Custom(String::new())
    }

    fn build_command(&self, config: &AgentConfig) -> String {
        // The custom agent type's name is the executable to run.
        let bin = config.agent_type.to_string();
        let mut cmd = format!("cd {} && {bin}", config.workspace.display());

        if let Some(model) = &config.model {
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
        let prompt = controlling_prompt_line(output);

        if ends_with_selection_menu(output) || looks_like_prompt(prompt) {
            result.status = Some(AgentStatus::WaitingForInput);
        } else if output.contains("error:") || output.contains("Error:") {
            result.status = Some(AgentStatus::Error);
        }
        result
    }

    fn display_name(&self) -> &str {
        "Custom CLI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config(name: &str, prompt: Option<&str>) -> AgentConfig {
        AgentConfig {
            agent_type: AgentType::Custom(name.to_string()),
            model: None,
            workspace: PathBuf::from("/home/user/project"),
            prompt: prompt.map(|s| s.to_string()),
            extra_args: vec![],
        }
    }

    #[test]
    fn build_command_uses_type_name_as_binary() {
        let cmd = CustomAdapter.build_command(&config("aider", Some("fix bug")));
        assert_eq!(cmd, "cd /home/user/project && aider 'fix bug'");
    }

    #[test]
    fn build_command_without_prompt() {
        let cmd = CustomAdapter.build_command(&config("mycli", None));
        assert_eq!(cmd, "cd /home/user/project && mycli");
    }

    #[test]
    fn parse_detects_waiting_prompt() {
        let parsed = CustomAdapter.parse_output("Proceed? (y/n)");
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
    }
}
