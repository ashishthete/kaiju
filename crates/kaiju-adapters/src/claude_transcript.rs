//! Reading precise token usage from Claude Code's session transcript.
//!
//! Claude Code writes a JSONL transcript per session at
//! `~/.claude/projects/<slug>/<session-id>.jsonl`, where `<slug>` is the working
//! directory with every non-alphanumeric character replaced by `-`. Each
//! `assistant` line carries `message.usage` with exact token counts — a far more
//! reliable source than scraping the TUI, whose layout and (user-configured)
//! status line vary.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use kaiju_core::adapter::SessionInfo;
use serde_json::Value;

/// Seconds of slack: a transcript's first event lands a moment after the daemon
/// records the agent as started, and a small clock skew is possible.
const START_SLACK_SECS: i64 = 15;

/// Exact token usage aggregated across a session's assistant messages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    /// Model of the most recent assistant message (for per-model pricing).
    pub model: Option<String>,
}

impl Usage {
    /// Tokens to surface as "tokens used": fresh input, generated output, and
    /// newly cached input. Cache *reads* are deliberately excluded — they re-count
    /// the same context on every turn and would inflate the figure by orders of
    /// magnitude (a long session reads millions of cached tokens it never paid
    /// full price for).
    pub fn tokens_used(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens
    }

    /// True when no usage was found (no assistant turns yet).
    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_creation_tokens == 0
            && self.cache_read_tokens == 0
    }
}

/// Map a working directory to Claude Code's project-dir slug: every character
/// that is not ASCII-alphanumeric becomes `-` (e.g. `/Users/a/.kaiju/wt` ->
/// `-Users-a--kaiju-wt`).
pub fn project_slug(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Pure: sum token usage across all `assistant` messages in a transcript,
/// deduped by message id (one message can span more than one JSONL line).
pub fn aggregate_usage(jsonl: &str) -> Usage {
    let mut total = Usage::default();
    let mut seen = std::collections::HashSet::new();

    for line in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        // Dedup by message id when present, so a re-emitted message is counted once.
        if let Some(id) = message.get("id").and_then(Value::as_str) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let field = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        total.input_tokens += field("input_tokens");
        total.output_tokens += field("output_tokens");
        total.cache_creation_tokens += field("cache_creation_input_tokens");
        total.cache_read_tokens += field("cache_read_input_tokens");
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            total.model = Some(model.to_string());
        }
    }

    total
}

/// Base directory holding Claude Code's per-project transcripts. Honors
/// `KAIJU_CLAUDE_PROJECTS` (an explicit override, mainly for tests), then
/// `CLAUDE_CONFIG_DIR`, else `~/.claude`; transcripts live under `projects/`.
pub(crate) fn projects_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("KAIJU_CLAUDE_PROJECTS") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".claude")))
        .ok()?;
    Some(base.join("projects"))
}

/// Newest transcript file for `run_dir`, last modified at or after `since_unix`
/// Unix timestamp of a transcript's first dated event (when its session began).
/// Reads only the first lines, so it's cheap even on a huge transcript.
fn session_start_unix(path: &Path) -> Option<i64> {
    let reader = BufReader::new(std::fs::File::open(path).ok()?);
    for line in reader.lines().take(50).map_while(Result::ok) {
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if let Some(ts) = value.get("timestamp").and_then(Value::as_str) {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    return Some(dt.timestamp());
                }
            }
        }
    }
    None
}

/// Find the transcript for *this* agent's session. Several Claude sessions can
/// share a working directory (the operator's own + each agent's), so newest-
/// modified is wrong — it cross-attributes usage. Instead match by start time:
/// among sessions that began at/after the agent started, take the earliest (the
/// one that began when the agent launched). `None` if none matches yet.
fn find_transcript(run_dir: &Path, since_unix: i64) -> Option<PathBuf> {
    let dir = projects_root()?.join(project_slug(run_dir));
    let mut best: Option<(i64, PathBuf)> = None;

    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(start) = session_start_unix(&path) else {
            continue;
        };
        if start < since_unix - START_SLACK_SECS {
            continue; // began before this agent — a different (e.g. operator) session
        }
        let is_earlier = match &best {
            Some((seen, _)) => start < *seen,
            None => true,
        };
        if is_earlier {
            best = Some((start, path));
        }
    }

    best.map(|(_, path)| path)
}

/// Read aggregated token usage for a Claude session run in `run_dir` and started
/// at `since_unix` (Unix seconds). `None` if no transcript is found or it carries
/// no usage yet.
pub fn read_usage(run_dir: &Path, since_unix: i64) -> Option<Usage> {
    let path = find_transcript(run_dir, since_unix)?;
    let content = std::fs::read_to_string(path).ok()?;
    let usage = aggregate_usage(&content);
    if usage.is_empty() {
        None
    } else {
        Some(usage)
    }
}

/// First user-message text in a transcript, truncated for display. Reads only
/// the head of the file. Handles `content` as a plain string or an array of
/// content blocks (uses the first text block).
fn first_user_prompt(path: &Path) -> String {
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    for line in BufReader::new(file).lines().take(50).map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = match v.get("message").and_then(|m| m.get("content")) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(items)) => items
                .iter()
                .find_map(|it| it.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed.chars().take(80).collect();
    }
    String::new()
}

/// Last-modified time of a file as Unix seconds (0 if unavailable).
fn modified_unix(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resumable Claude sessions recorded for `workspace`, newest first.
/// Best-effort: a missing/unreadable projects dir yields an empty list.
pub(crate) fn list_workspace_sessions(workspace: &Path) -> Vec<SessionInfo> {
    let Some(dir) = projects_root().map(|r| r.join(project_slug(workspace))) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<SessionInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?.to_string();
            Some(SessionInfo {
                id,
                last_active_unix: modified_unix(&p),
                first_prompt: first_user_prompt(&p),
            })
        })
        .collect();
    sessions.sort_by(|a, b| b.last_active_unix.cmp(&a.last_active_unix));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_slug_replaces_non_alphanumeric() {
        assert_eq!(
            project_slug(Path::new("/Users/a/work/credibl-esg")),
            "-Users-a-work-credibl-esg"
        );
        // A leading dot (e.g. ~/.kaiju) yields a double dash, matching Claude.
        assert_eq!(
            project_slug(Path::new("/Users/a/.kaiju/worktrees/27c0")),
            "-Users-a--kaiju-worktrees-27c0"
        );
    }

    #[test]
    fn aggregate_sums_assistant_usage() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":3,"cache_read_input_tokens":100}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":2,"output_tokens":7,"cache_creation_input_tokens":0,"cache_read_input_tokens":200}}}"#,
        );
        let usage = aggregate_usage(jsonl);
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 12);
        assert_eq!(usage.cache_creation_tokens, 3);
        assert_eq!(usage.cache_read_tokens, 300);
        // tokens_used excludes cache reads: 12 + 12 + 3.
        assert_eq!(usage.tokens_used(), 27);
    }

    #[test]
    fn aggregate_dedups_repeated_message_ids() {
        let line = r#"{"type":"assistant","message":{"id":"dup","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let jsonl = format!("{line}\n{line}");
        let usage = aggregate_usage(&jsonl);
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
    }

    #[test]
    fn aggregate_ignores_malformed_and_non_assistant_lines() {
        let jsonl = concat!(
            "not json\n",
            r#"{"type":"system","subtype":"x"}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m1","usage":{"output_tokens":9}}}"#,
        );
        let usage = aggregate_usage(jsonl);
        assert_eq!(usage.output_tokens, 9);
        assert_eq!(usage.input_tokens, 0);
    }

    #[test]
    fn empty_usage_is_empty() {
        assert!(Usage::default().is_empty());
        assert!(!aggregate_usage(
            r#"{"type":"assistant","message":{"id":"m","usage":{"output_tokens":1}}}"#
        )
        .is_empty());
    }

    #[test]
    fn list_workspace_sessions_reads_id_and_first_prompt() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join("kaiju-sessions-test-list");
        let ws = std::path::Path::new("/Users/x/repo");
        let slug = project_slug(ws);
        let dir = tmp.join(&slug);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("sess-1.jsonl")).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"refactor the parser"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":{{"id":"m1","usage":{{"output_tokens":5}}}}}}"#).unwrap();

        std::env::set_var("KAIJU_CLAUDE_PROJECTS", &tmp);
        let sessions = list_workspace_sessions(ws);
        std::env::remove_var("KAIJU_CLAUDE_PROJECTS");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");
        assert_eq!(sessions[0].first_prompt, "refactor the parser");
        assert!(sessions[0].last_active_unix > 0);
    }

    #[test]
    fn list_workspace_sessions_empty_when_no_dir() {
        std::env::set_var("KAIJU_CLAUDE_PROJECTS", std::env::temp_dir().join("kaiju-nope-xyz"));
        let out = list_workspace_sessions(std::path::Path::new("/no/such/ws"));
        std::env::remove_var("KAIJU_CLAUDE_PROJECTS");
        assert!(out.is_empty());
    }
}
