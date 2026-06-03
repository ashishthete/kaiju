//! Parser for Claude Code's structured `--output-format stream-json` events.
//!
//! Each line of that stream is a JSON object with a `type`. Status comes from
//! the event type and exact cost/tokens from the terminal `result` event — this
//! replaces the heuristic text scraping with precise data where the CLI offers
//! a structured stream.
//!
//! Deliberately tolerant (keyed on field presence, not a rigid schema) so minor
//! differences between CLI versions degrade gracefully rather than breaking.

use nexus_core::adapter::ParsedOutput;
use nexus_core::agent::AgentStatus;

/// Parse a single line of Claude stream-json. Returns `None` for blank lines,
/// non-JSON, or unrecognized event types.
pub fn parse_claude_event(line: &str) -> Option<ParsedOutput> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let event_type = value.get("type")?.as_str()?;

    let mut out = ParsedOutput::default();
    match event_type {
        // The final event of a run: success or error, with exact cost/tokens.
        "result" => {
            let is_error = value
                .get("is_error")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            out.status = Some(if is_error {
                AgentStatus::Error
            } else {
                AgentStatus::Completed
            });
            if let Some(cost) = value.get("total_cost_usd").and_then(|c| c.as_f64()) {
                out.estimated_cost_usd = Some(cost);
            }
            out.tokens_used = total_tokens(value.get("usage"));
        }
        // Intermediate progress events: the agent is working.
        "assistant" | "user" | "system" | "tool_use" | "tool_result" => {
            out.status = Some(AgentStatus::Running);
            // Surface a running token total when an event carries usage.
            out.tokens_used = total_tokens(value.get("message").and_then(|m| m.get("usage")));
        }
        _ => return None,
    }
    Some(out)
}

/// Sum input + output tokens from a `usage` object, if present and non-zero.
fn total_tokens(usage: Option<&serde_json::Value>) -> Option<u64> {
    let usage = usage?;
    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = input + output;
    if total > 0 {
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_success_yields_completed_with_cost_and_tokens() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0123,"usage":{"input_tokens":100,"output_tokens":50}}"#;
        let parsed = parse_claude_event(line).unwrap();
        assert_eq!(parsed.status, Some(AgentStatus::Completed));
        assert_eq!(parsed.estimated_cost_usd, Some(0.0123));
        assert_eq!(parsed.tokens_used, Some(150));
    }

    #[test]
    fn result_error_yields_error_status() {
        let line =
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"total_cost_usd":0.5}"#;
        let parsed = parse_claude_event(line).unwrap();
        assert_eq!(parsed.status, Some(AgentStatus::Error));
        assert_eq!(parsed.estimated_cost_usd, Some(0.5));
    }

    #[test]
    fn assistant_event_is_running_and_surfaces_usage() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let parsed = parse_claude_event(line).unwrap();
        assert_eq!(parsed.status, Some(AgentStatus::Running));
        assert_eq!(parsed.tokens_used, Some(15));
    }

    #[test]
    fn system_init_is_running() {
        let line = r#"{"type":"system","subtype":"init","model":"claude-opus-4-8"}"#;
        let parsed = parse_claude_event(line).unwrap();
        assert_eq!(parsed.status, Some(AgentStatus::Running));
        assert_eq!(parsed.tokens_used, None);
    }

    #[test]
    fn blank_or_non_json_is_none() {
        assert!(parse_claude_event("").is_none());
        assert!(parse_claude_event("   ").is_none());
        assert!(parse_claude_event("not json at all").is_none());
    }

    #[test]
    fn unknown_event_type_is_none() {
        assert!(parse_claude_event(r#"{"type":"telemetry","data":1}"#).is_none());
        // Missing `type` is also ignored.
        assert!(parse_claude_event(r#"{"foo":"bar"}"#).is_none());
    }
}
