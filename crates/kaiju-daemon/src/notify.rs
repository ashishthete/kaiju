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

/// Best-effort operator alert: a console bell + warning line always, and a
/// Slack post when `KAIJU_SLACK_WEBHOOK` is configured.
pub fn alert(agent: &Agent, status: AgentStatus) {
    let message = alert_message(agent, status);
    // `\x07` rings the terminal bell wherever the daemon is running.
    warn!("\x07{message}");

    if let Ok(url) = std::env::var("KAIJU_SLACK_WEBHOOK") {
        if !url.is_empty() {
            post_to_slack(url, message);
        }
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
