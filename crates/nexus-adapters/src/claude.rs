use nexus_core::adapter::{
    controlling_prompt_line, ends_with_selection_menu, last_non_empty_line, looks_like_prompt,
    Adapter, ParsedOutput,
};
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
        // Launch the interactive TUI (no `-p`/print mode) so the session stays
        // alive and can be supervised. The prompt is passed positionally to seed
        // the first turn.
        let bin = crate::binary::agent_binary("KAIJU_CLAUDE_BIN", "claude");
        let mut cmd = format!("cd {} && {bin}", config.workspace.display());

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
        let last = last_non_empty_line(output);
        let prompt = controlling_prompt_line(output);

        // A waiting prompt is the current, actionable state, so it takes
        // priority. A trailing selection menu (question may sit well above it) or
        // a controlling prompt line both count; stale scrollback does not.
        if ends_with_selection_menu(output)
            || looks_like_prompt(prompt)
            || prompt.contains("Do you want")
        {
            result.status = Some(AgentStatus::WaitingForInput);
        } else if output.contains("Task completed") || output.contains("Done!") {
            result.status = Some(AgentStatus::Completed);
        } else if last.contains("Error:") || last.contains("error:") {
            result.status = Some(AgentStatus::Error);
        } else if output.contains("Working")
            || output.contains("Thinking")
            || output.contains("Reading")
        {
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

    fn parse_event(&self, line: &str) -> Option<ParsedOutput> {
        crate::claude_events::parse_claude_event(line)
    }

    fn structured_command(&self, config: &AgentConfig) -> Option<String> {
        // Batch mode is non-interactive, so a prompt is required.
        let prompt = config.prompt.as_ref()?;
        let bin = crate::binary::agent_binary("KAIJU_CLAUDE_BIN", "claude");
        let escaped = prompt.replace('\'', "'\\''");

        let mut cmd = format!(
            "cd {} && {bin} -p --output-format stream-json --verbose",
            config.workspace.display()
        );
        if let Some(model) = config.model.as_deref().or(self.default_model()) {
            cmd.push_str(&format!(" --model {model}"));
        }
        cmd.push_str(&format!(" '{escaped}'"));
        Some(cmd)
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
            "cd /home/user/project && claude --model sonnet 'fix the auth bug'"
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
        assert_eq!(
            adapter.build_command(&cfg),
            "cd /tmp && claude --model claude-opus-4-8 --verbose"
        );
    }

    #[test]
    fn build_command_escapes_single_quotes_in_prompt() {
        let adapter = ClaudeAdapter;
        let cmd = adapter.build_command(&config(Some("fix the user's login")));
        assert!(cmd.contains("user'\\''s"));
    }

    #[test]
    fn structured_command_uses_stream_json_with_prompt() {
        let adapter = ClaudeAdapter;
        let cmd = adapter
            .structured_command(&config(Some("fix the bug")))
            .unwrap();
        assert!(cmd.contains("-p --output-format stream-json"));
        assert!(cmd.contains("--model sonnet"));
        assert!(cmd.ends_with("'fix the bug'"));
    }

    #[test]
    fn structured_command_requires_a_prompt() {
        let adapter = ClaudeAdapter;
        let cfg = AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        };
        assert!(adapter.structured_command(&cfg).is_none());
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
    fn waiting_prompt_on_last_line_wins_over_earlier_working_text() {
        let adapter = ClaudeAdapter;
        // Earlier scrollback says "Working", but the current line is a prompt.
        let output = "Working on the task...\nEdited 3 files\nApply these changes? (y/n)";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
    }

    #[test]
    fn waiting_detected_when_question_sits_above_a_menu() {
        let adapter = ClaudeAdapter;
        // The actionable question is above the selection menu options.
        let output =
            "● I'll edit main.rs\n\nDo you want to make this edit to main.rs?\n❯ 1. Yes\n  2. No";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
    }

    #[test]
    fn waiting_detected_on_trust_folder_prompt() {
        let adapter = ClaudeAdapter;
        // Claude Code's real startup prompt: the question is several lines above
        // the menu, with intervening text and a keyboard-hint footer below it.
        let output = "Quick safety check: Is this a project you trust?\n\
            Claude Code'll be able to read, edit, and execute files here.\n\
            Security guide\n\
            ❯ 1. Yes, I trust this folder\n\
              2. No, exit\n\
            Enter to confirm · Esc to cancel";
        let parsed = adapter.parse_output(output);
        assert_eq!(parsed.status, Some(AgentStatus::WaitingForInput));
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
