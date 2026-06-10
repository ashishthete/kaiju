//! Operator alerts for status changes that need a human.
//!
//! The decision logic ([`should_alert`]) and the message text ([`alert_message`])
//! are pure and tested; delivery ([`alert`]) is a best-effort side effect — a
//! console bell always, plus a Slack post when `KAIJU_SLACK_WEBHOOK` is set.

use kaiju_core::agent::{Agent, AgentStatus};
use tracing::warn;

/// Should a status change pull the operator in?
///
/// Only on a genuine *transition* into a state that needs a human — waiting for
/// input, or an error. Returns false when the status is unchanged, so a steady
/// `WaitingForInput` across many polls alerts exactly once.
pub fn should_alert(previous: AgentStatus, next: AgentStatus) -> bool {
    previous != next
        && matches!(
            next,
            AgentStatus::WaitingForInput | AgentStatus::Error | AgentStatus::Stuck
        )
}

/// Pure: the human-readable alert line for an agent reaching `status`.
pub fn alert_message(agent: &Agent, status: AgentStatus) -> String {
    let reason = match status {
        AgentStatus::WaitingForInput => "is waiting for your input",
        AgentStatus::Error => "hit an error",
        AgentStatus::Stuck => "appears stuck (no output for a while)",
        _ => "needs attention",
    };
    format!("agent {} ({}) {}", agent.id, agent.agent_type, reason)
}

/// Best-effort operator alert: a console bell + warning line always, a native
/// desktop notification when `KAIJU_DESKTOP_NOTIFY` is set, and a Slack post when
/// `KAIJU_SLACK_WEBHOOK` is configured. Unlike the browser toast, the desktop
/// one fires from the always-running daemon — no open tab or window focus needed.
pub fn alert(agent: &Agent, status: AgentStatus) {
    let message = alert_message(agent, status);
    // `\x07` rings the terminal bell wherever the daemon is running.
    warn!("\x07{message}");

    if env_enabled("KAIJU_DESKTOP_NOTIFY") {
        desktop_notify(&message);
    }

    if let Ok(url) = std::env::var("KAIJU_SLACK_WEBHOOK") {
        if !url.is_empty() {
            post_to_slack(url, message);
        }
    }
}

/// Is an opt-in env flag set to a truthy value (`1`/`true`)?
fn env_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Escape a string for embedding in an AppleScript double-quoted literal.
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Fire a native desktop notification (macOS `osascript`, otherwise
/// `notify-send`). Best-effort and fire-and-forget: spawn it and ignore the
/// outcome so a missing tool never disturbs the monitor.
fn desktop_notify(message: &str) {
    let title = "Kaiju";
    let spawned = if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape_applescript(message),
            title
        );
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn()
    } else {
        std::process::Command::new("notify-send")
            .arg(title)
            .arg(message)
            .spawn()
    };
    if let Err(e) = spawned {
        warn!("desktop notification failed: {e}");
    }
}

/// Fire-and-forget POST to a Slack incoming webhook. Spawned onto the current
/// runtime; failures are ignored (alerts must never block or crash the monitor).
fn post_to_slack(url: String, text: String) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let client = reqwest::Client::new();
        if let Err(e) = client
            .post(&url)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await
        {
            warn!("slack notification failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_on_transition_into_waiting() {
        assert!(should_alert(
            AgentStatus::Running,
            AgentStatus::WaitingForInput
        ));
    }

    #[test]
    fn alerts_on_transition_into_error() {
        assert!(should_alert(AgentStatus::Running, AgentStatus::Error));
    }

    #[test]
    fn no_alert_when_status_unchanged() {
        assert!(!should_alert(
            AgentStatus::WaitingForInput,
            AgentStatus::WaitingForInput
        ));
        assert!(!should_alert(AgentStatus::Error, AgentStatus::Error));
    }

    #[test]
    fn no_alert_leaving_waiting() {
        assert!(!should_alert(
            AgentStatus::WaitingForInput,
            AgentStatus::Running
        ));
    }

    #[test]
    fn no_alert_on_completion() {
        assert!(!should_alert(AgentStatus::Running, AgentStatus::Completed));
    }

    #[test]
    fn escape_applescript_escapes_quotes_and_backslashes() {
        assert_eq!(escape_applescript(r#"a "b" \c"#), r#"a \"b\" \\c"#);
        assert_eq!(escape_applescript("plain"), "plain");
    }

    #[test]
    fn env_enabled_only_for_truthy_values() {
        std::env::set_var("KAIJU_NOTIFY_TEST_FLAG", "1");
        assert!(env_enabled("KAIJU_NOTIFY_TEST_FLAG"));
        std::env::set_var("KAIJU_NOTIFY_TEST_FLAG", "0");
        assert!(!env_enabled("KAIJU_NOTIFY_TEST_FLAG"));
        std::env::remove_var("KAIJU_NOTIFY_TEST_FLAG");
        assert!(!env_enabled("KAIJU_NOTIFY_TEST_FLAG"));
    }

    #[test]
    fn alerts_on_transition_into_stuck() {
        assert!(should_alert(AgentStatus::Running, AgentStatus::Stuck));
    }

    #[test]
    fn no_alert_when_already_stuck() {
        assert!(!should_alert(AgentStatus::Stuck, AgentStatus::Stuck));
    }

    #[test]
    fn alert_message_names_agent_and_reason() {
        use kaiju_core::agent::{AgentConfig, AgentType};
        let agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: std::path::PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        });

        let waiting = alert_message(&agent, AgentStatus::WaitingForInput);
        assert!(waiting.contains(&agent.id));
        assert!(waiting.contains("waiting for your input"));

        assert!(alert_message(&agent, AgentStatus::Stuck).contains("stuck"));
        assert!(alert_message(&agent, AgentStatus::Error).contains("error"));
    }
}
