use crate::agent::{AgentConfig, AgentStatus, AgentType};

/// Output from parsing a CLI's terminal output.
#[derive(Debug, Clone, Default)]
pub struct ParsedOutput {
    pub status: Option<AgentStatus>,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

/// Return the last non-empty, trimmed line of terminal output.
///
/// Adapters use this to inspect the *current* prompt line rather than the whole
/// scrollback, which avoids matching stale text from earlier in the session.
pub fn last_non_empty_line(output: &str) -> &str {
    output
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// Heuristic: does this line look like an interactive prompt awaiting input?
///
/// Deliberately conservative to avoid false positives (which would falsely
/// signal "agent needs you"). Adapters can layer CLI-specific markers on top.
pub fn looks_like_prompt(line: &str) -> bool {
    const YES_NO_MARKERS: [&str; 6] = ["(y/n)", "(Y/n)", "[y/N]", "[Y/n]", "(yes/no)", "[y/n]"];

    let line = line.trim_end();
    line.ends_with('?') || YES_NO_MARKERS.iter().any(|marker| line.contains(marker))
}

/// The active-selection cursor of an interactive menu (the highlighted choice).
/// A stronger signal than a bare numbered option, which also appears in ordinary
/// output (e.g. a numbered list the agent printed).
pub fn is_selection_arrow_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('❯')
        || line.starts_with('›')
        || line.starts_with('»')
        || line.starts_with("> ")
}

/// Is this line part of an interactive selection menu — a chooser arrow or a
/// numbered/bulleted option — rather than the prompt question itself?
pub fn is_menu_option_line(line: &str) -> bool {
    is_selection_arrow_line(line) || starts_with_numbered_option(line.trim_start())
}

/// e.g. "1. Yes" or "2) No".
fn starts_with_numbered_option(line: &str) -> bool {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    let rest = &line[digits..];
    rest.starts_with('.') || rest.starts_with(')')
}

/// A keyboard-hint footer rendered beneath a selection menu, e.g.
/// "Enter to confirm · Esc to cancel" or "↑/↓ to navigate". These describe how
/// to answer rather than asking the question, so detection steps over them.
///
/// A line ending in `?` is never a hint — that protects real questions that
/// happen to mention a key (e.g. "What do you want to confirm?").
pub fn is_menu_hint_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.ends_with('?') {
        return false;
    }
    let lower = trimmed.to_lowercase();
    line.contains('↑')
        || line.contains('↓')
        || lower.contains("esc to ")
        || lower.contains("enter to ")
        || lower.contains(" to confirm")
        || lower.contains(" to cancel")
        || lower.contains(" to select")
        || lower.contains(" to navigate")
}

/// True when the trailing content of `output` is an interactive selection menu:
/// a run of option lines carrying the active-selection arrow (`❯`), optionally
/// followed by a keyboard-hint footer. A menu at the very end means the agent is
/// waiting for the operator to choose — even when the question sits several lines
/// above the menu (so `controlling_prompt_line` can't reach it).
///
/// Requires the arrow specifically: a bare numbered list in normal output is not
/// treated as a prompt.
pub fn ends_with_selection_menu(output: &str) -> bool {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let mut idx = lines.len();
    while idx > 0 && is_menu_hint_line(lines[idx - 1]) {
        idx -= 1;
    }

    let mut saw_arrow = false;
    while idx > 0 && is_menu_option_line(lines[idx - 1]) {
        if is_selection_arrow_line(lines[idx - 1]) {
            saw_arrow = true;
        }
        idx -= 1;
    }

    saw_arrow
}

/// The line that decides whether the agent is waiting: the last non-empty line
/// that is *not* part of a trailing selection menu or its keyboard-hint footer.
///
/// Agent CLIs often render a question followed by menu choices
/// (`Do you want…?` then `❯ 1. Yes / 2. No`) and sometimes a footer
/// (`Esc to cancel`). The question is the meaningful part, so trailing menu and
/// hint lines are skipped — while deep scrollback is still ignored, since only
/// the trailing run of decoration is stepped over.
pub fn controlling_prompt_line(output: &str) -> &str {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let mut idx = lines.len();
    while idx > 0 && (is_menu_option_line(lines[idx - 1]) || is_menu_hint_line(lines[idx - 1])) {
        idx -= 1;
    }

    if idx == 0 {
        ""
    } else {
        lines[idx - 1]
    }
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

    #[test]
    fn last_non_empty_line_skips_trailing_blanks() {
        let output = "first line\nDo you want to continue?\n\n   \n";
        assert_eq!(last_non_empty_line(output), "Do you want to continue?");
    }

    #[test]
    fn last_non_empty_line_empty_input() {
        assert_eq!(last_non_empty_line(""), "");
        assert_eq!(last_non_empty_line("\n  \n"), "");
    }

    #[test]
    fn looks_like_prompt_detects_question_and_yes_no() {
        assert!(looks_like_prompt("Continue?"));
        assert!(looks_like_prompt("Apply these changes? (y/n)"));
        assert!(looks_like_prompt("Overwrite file [y/N]"));
    }

    #[test]
    fn looks_like_prompt_rejects_normal_output() {
        assert!(!looks_like_prompt("Reading src/main.rs"));
        assert!(!looks_like_prompt("Total cost: $1.42"));
        assert!(!looks_like_prompt(""));
    }

    #[test]
    fn is_menu_option_line_detects_arrows_and_numbers() {
        assert!(is_menu_option_line("❯ 1. Yes"));
        assert!(is_menu_option_line("  2. No"));
        assert!(is_menu_option_line("3) Cancel"));
        assert!(!is_menu_option_line("Do you want to continue?"));
        assert!(!is_menu_option_line("3 files changed"));
    }

    #[test]
    fn controlling_prompt_line_skips_trailing_menu() {
        let output = "I'll edit main.rs\n\nDo you want to make this edit?\n❯ 1. Yes\n  2. No";
        assert_eq!(
            controlling_prompt_line(output),
            "Do you want to make this edit?"
        );
    }

    #[test]
    fn controlling_prompt_line_returns_last_line_when_no_menu() {
        let output = "Working...\nThinking about the next step";
        assert_eq!(
            controlling_prompt_line(output),
            "Thinking about the next step"
        );
    }

    #[test]
    fn controlling_prompt_line_skips_trailing_hint_footer() {
        let output = "Apply these changes?\n❯ 1. Yes\n  2. No\nEnter to confirm · Esc to cancel";
        assert_eq!(controlling_prompt_line(output), "Apply these changes?");
    }

    #[test]
    fn is_menu_hint_line_detects_footers() {
        assert!(is_menu_hint_line("Enter to confirm · Esc to cancel"));
        assert!(is_menu_hint_line("↑/↓ to navigate"));
        assert!(is_menu_hint_line("Press Esc to cancel"));
    }

    #[test]
    fn is_menu_hint_line_rejects_questions_and_normal_text() {
        // A question that mentions a key must not be mistaken for a footer.
        assert!(!is_menu_hint_line("What do you want to confirm?"));
        assert!(!is_menu_hint_line("Reading src/main.rs"));
    }

    #[test]
    fn ends_with_selection_menu_detects_arrow_menu() {
        let output = "Do you want to apply these changes?\n❯ 1. Yes\n  2. No";
        assert!(ends_with_selection_menu(output));
    }

    #[test]
    fn ends_with_selection_menu_detects_question_far_above_menu_with_footer() {
        // Mirrors Claude Code's "trust this folder" prompt: the question is
        // several lines above the menu, with intervening text and a footer.
        let output = "Quick safety check: Is this a project you trust?\n\
            Claude Code'll be able to read, edit, and execute files here.\n\
            Security guide\n\
            ❯ 1. Yes, I trust this folder\n\
              2. No, exit\n\
            Enter to confirm · Esc to cancel";
        assert!(ends_with_selection_menu(output));
    }

    #[test]
    fn ends_with_selection_menu_ignores_bare_numbered_list() {
        // A numbered list in ordinary output (no chooser arrow) is not a prompt.
        let output = "Here is my plan:\n1. Refactor the parser\n2. Add tests";
        assert!(!ends_with_selection_menu(output));
    }

    #[test]
    fn ends_with_selection_menu_false_when_menu_already_answered() {
        // The menu scrolled into history; the current line is fresh work output.
        let output = "❯ 1. Yes\n  2. No\nApplying...\nWorking on the next step";
        assert!(!ends_with_selection_menu(output));
    }

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
