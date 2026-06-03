//! Operator alerts for status changes that need a human.
//!
//! The decision logic ([`should_alert`]) is pure and tested; the delivery
//! ([`alert`]) is a best-effort side effect kept separate from it.

use nexus_core::agent::{Agent, AgentStatus};
use tracing::warn;

/// Should a status change pull the operator in?
///
/// Only on a genuine *transition* into a state that needs a human — waiting for
/// input, or an error. Returns false when the status is unchanged, so a steady
/// `WaitingForInput` across many polls alerts exactly once.
pub fn should_alert(previous: AgentStatus, next: AgentStatus) -> bool {
    previous != next && matches!(next, AgentStatus::WaitingForInput | AgentStatus::Error)
}

/// Best-effort operator alert: a prominent warning line plus a terminal bell
/// in the daemon's console. Richer (OS-level) notifications can layer on later.
pub fn alert(agent: &Agent, status: AgentStatus) {
    let reason = match status {
        AgentStatus::WaitingForInput => "is waiting for your input",
        AgentStatus::Error => "hit an error",
        _ => "needs attention",
    };
    // `\x07` rings the terminal bell wherever the daemon is running.
    warn!("\x07agent {} ({}) {}", agent.id, agent.agent_type, reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_on_transition_into_waiting() {
        assert!(should_alert(AgentStatus::Running, AgentStatus::WaitingForInput));
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
}
