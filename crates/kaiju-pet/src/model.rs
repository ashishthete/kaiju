use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    WaitingForInput,
    Stuck,
    Limited,
    Error,
    Completed,
    Stopped,
    Unknown,
}

/// Parse the daemon's Debug-lowercased status string (api.rs:228).
/// NOTE: the daemon emits `waitingforinput` (no underscore), not serde's
/// `waiting_for_input`. Any unrecognized value maps to `Unknown`.
pub fn parse_status(s: &str) -> Status {
    match s {
        "starting" => Status::Starting,
        "running" => Status::Running,
        "waitingforinput" => Status::WaitingForInput,
        "stuck" => Status::Stuck,
        "limited" => Status::Limited,
        "error" => Status::Error,
        "completed" => Status::Completed,
        "stopped" => Status::Stopped,
        _ => Status::Unknown,
    }
}

impl Status {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Status::Completed | Status::Stopped | Status::Error)
    }

    pub fn is_attention(&self) -> bool {
        matches!(
            self,
            Status::WaitingForInput | Status::Stuck | Status::Limited | Status::Error
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Status::Starting
                | Status::Running
                | Status::WaitingForInput
                | Status::Stuck
                | Status::Limited
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawAgent {
    id: String,
    agent_type: String,
    status: String,
    workspace: String,
    #[serde(default)]
    prompt: Option<String>,
    updated_at: String,
    session_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSnapshot {
    pub id: String,
    pub agent_type: String,
    pub status: Status,
    pub workspace: String,
    pub prompt: Option<String>,
    pub updated_at: String,
    pub session_name: String,
}

pub fn decode_agents(body: &str) -> Result<Vec<AgentSnapshot>, serde_json::Error> {
    let raw: Vec<RawAgent> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .map(|r| AgentSnapshot {
            id: r.id,
            agent_type: r.agent_type,
            status: parse_status(&r.status),
            workspace: r.workspace,
            prompt: r.prompt,
            updated_at: r.updated_at,
            session_name: r.session_name,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_daemon_wire_strings() {
        assert_eq!(parse_status("waitingforinput"), Status::WaitingForInput);
        assert_eq!(parse_status("running"), Status::Running);
        assert_eq!(parse_status("stuck"), Status::Stuck);
        assert_eq!(parse_status("limited"), Status::Limited);
        assert_eq!(parse_status("error"), Status::Error);
        assert_eq!(parse_status("completed"), Status::Completed);
        assert_eq!(parse_status("stopped"), Status::Stopped);
        assert_eq!(parse_status("starting"), Status::Starting);
    }

    #[test]
    fn unknown_status_is_non_attention_and_does_not_panic() {
        let s = parse_status("some_future_state");
        assert_eq!(s, Status::Unknown);
        assert!(!s.is_attention());
        assert!(!s.is_terminal());
    }

    #[test]
    fn decodes_real_agentresponse_shape() {
        // Mirrors crates/kaiju-daemon/src/api.rs:189 AgentResponse exactly.
        let body = r#"[{
            "id":"ag_1","agent_type":"claude","model":null,
            "workspace":"/home/u/proj","status":"waitingforinput",
            "session_name":"kaiju-ag_1","prompt":"fix the bug",
            "created_at":"2026-07-16T10:00:00+00:00",
            "updated_at":"2026-07-16T10:05:00+00:00",
            "metrics":{"runtime_secs":300,"tokens_used":null,"estimated_cost_usd":null},
            "compare_group":null,"port":null,"resume_at":null,"auto_continue":false
        }]"#;
        let agents = decode_agents(body).expect("decode");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "ag_1");
        assert_eq!(agents[0].status, Status::WaitingForInput);
        assert_eq!(agents[0].workspace, "/home/u/proj");
    }
}
