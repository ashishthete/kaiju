use nexus_core::adapter::{
    controlling_prompt_line, ends_with_selection_menu, last_non_empty_line, looks_like_prompt,
    Adapter, ParsedOutput,
};
use nexus_core::agent::{AgentConfig, AgentStatus, AgentType};

/// Adapter for Google Gemini CLI.
pub struct GeminiAdapter;

impl Adapter for GeminiAdapter {
    fn agent_type(&self) -> AgentType {
        AgentType::Gemini
    }

    fn default_model(&self) -> Option<&str> {
        Some("gemini-2.5-pro")
    }

    fn build_command(&self, config: &AgentConfig) -> String {
        // `-i` seeds the first prompt and stays interactive, unlike `-p` which
        // runs once and exits. Keeps the session alive for supervision.
        let bin = crate::binary::agent_binary("NEXUS_GEMINI_BIN", "gemini");
        let mut cmd = format!("cd {} && {bin}", config.workspace.display());

        let model = config.model.as_deref().or(self.default_model());
        if let Some(model) = model {
            cmd.push_str(&format!(" --model {model}"));
        }

        if let Some(prompt) = &config.prompt {
            let escaped = prompt.replace('\'', "'\\''");
            cmd.push_str(&format!(" -i '{escaped}'"));
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

        if ends_with_selection_menu(output) || looks_like_prompt(prompt) {
            result.status = Some(AgentStatus::WaitingForInput);
        } else if output.contains("Done") || output.contains("completed") {
            result.status = Some(AgentStatus::Completed);
        } else if last.contains("ERROR") || last.contains("error:") {
            result.status = Some(AgentStatus::Error);
        } else if output.contains("Generating")
            || output.contains("Analyzing")
            || output.contains("Thinking")
        {
            result.status = Some(AgentStatus::Running);
        }

        result
    }

    fn display_name(&self) -> &str {
        "Gemini CLI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_command_basic() {
        let adapter = GeminiAdapter;
        let config = AgentConfig {
            agent_type: AgentType::Gemini,
            model: Some("pro".to_string()),
            workspace: PathBuf::from("/tmp/work"),
            prompt: Some("review this code".to_string()),
            extra_args: vec![],
        };
        assert_eq!(
            adapter.build_command(&config),
            "cd /tmp/work && gemini --model pro -i 'review this code'"
        );
    }

    #[test]
    fn build_command_uses_default_model_when_none() {
        let adapter = GeminiAdapter;
        let config = AgentConfig {
            agent_type: AgentType::Gemini,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        };
        assert_eq!(
            adapter.build_command(&config),
            "cd /tmp && gemini --model gemini-2.5-pro"
        );
    }

    #[test]
    fn parse_running() {
        let adapter = GeminiAdapter;
        let parsed = adapter.parse_output("Analyzing the codebase...");
        assert_eq!(parsed.status, Some(AgentStatus::Running));
    }

    #[test]
    fn parse_error() {
        let adapter = GeminiAdapter;
        let parsed = adapter.parse_output("ERROR: quota exceeded");
        assert_eq!(parsed.status, Some(AgentStatus::Error));
    }
}
