//! Terminal-state run records for analytics.
//!
//! When an agent first transitions into a terminal status the store appends
//! one JSON line to `~/.kaiju/analytics.jsonl` (override the path with
//! `KAIJU_ANALYTICS_FILE`). Records outlive agent deletion so trends survive
//! cleanup, and are pruned on the same retention window as persisted logs
//! during the monitor's daily rotation pass.

use chrono::{DateTime, Utc};
use kaiju_core::agent::Agent;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One finished agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub finished_at: DateTime<Utc>,
    pub agent_id: String,
    pub agent_type: String,
    pub workspace: String,
    /// Terminal status, lowercase: `completed` | `stopped` | `error`.
    pub status: String,
    pub runtime_secs: u64,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

/// Aggregates over the records within a trailing window.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct WindowSummary {
    pub days: u64,
    pub runs: u64,
    pub completed: u64,
    pub stopped: u64,
    pub error: u64,
    pub tokens_used: u64,
    pub estimated_cost_usd: f64,
}

/// Records file: `KAIJU_ANALYTICS_FILE`, else `~/.kaiju/analytics.jsonl`.
/// Unit tests must opt in via the env var — never touch `$HOME` from tests.
fn file_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_ANALYTICS_FILE") {
        return Some(PathBuf::from(path));
    }
    if cfg!(test) {
        return None;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("analytics.jsonl"))
}

/// Append a record for a just-finished agent. Best-effort: analytics must
/// never disrupt the status update that triggered it, so errors are swallowed.
pub fn record(agent: &Agent) {
    let Some(path) = file_path() else {
        return;
    };
    let record = RunRecord {
        finished_at: Utc::now(),
        agent_id: agent.id.clone(),
        agent_type: agent.agent_type.to_string(),
        workspace: agent.workspace.display().to_string(),
        status: format!("{:?}", agent.status).to_lowercase(),
        runtime_secs: agent.metrics.runtime_secs,
        tokens_used: agent.metrics.tokens_used,
        estimated_cost_usd: agent.metrics.estimated_cost_usd,
    };
    let _ = append(&path, &record);
}

fn append(path: &Path, record: &RunRecord) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    let mut file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

/// All parseable records, oldest first. Unparseable lines are skipped.
fn load(path: &Path) -> Vec<RunRecord> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Summaries over trailing 7- and 45-day windows, for `GET /analytics`.
pub fn summary() -> Vec<WindowSummary> {
    let records = match file_path() {
        Some(path) => load(&path),
        None => Vec::new(),
    };
    [7, 45]
        .into_iter()
        .map(|days| summarize(&records, days, Utc::now()))
        .collect()
}

fn summarize(records: &[RunRecord], days: u64, now: DateTime<Utc>) -> WindowSummary {
    let cutoff = now - chrono::Duration::days(days as i64);
    let mut window = WindowSummary {
        days,
        ..WindowSummary::default()
    };
    for record in records.iter().filter(|r| r.finished_at >= cutoff) {
        window.runs += 1;
        match record.status.as_str() {
            "completed" => window.completed += 1,
            "stopped" => window.stopped += 1,
            _ => window.error += 1,
        }
        window.tokens_used += record.tokens_used.unwrap_or(0);
        window.estimated_cost_usd += record.estimated_cost_usd.unwrap_or(0.0);
    }
    window
}

/// Drop records older than the log retention window. Returns how many lines
/// were removed. Best-effort like [`record`].
pub fn prune_expired() -> usize {
    let Some(path) = file_path() else {
        return 0;
    };
    let cutoff =
        Utc::now() - chrono::Duration::from_std(crate::logstore::retention()).unwrap_or_default();
    prune_file(&path, cutoff)
}

fn prune_file(path: &Path, cutoff: DateTime<Utc>) -> usize {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| {
            serde_json::from_str::<RunRecord>(line)
                .map(|r| r.finished_at >= cutoff)
                .unwrap_or(false)
        })
        .collect();
    let dropped = content.lines().count() - kept.len();
    if dropped > 0 {
        let mut body = kept.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        let _ = std::fs::write(path, body);
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_at(finished_at: DateTime<Utc>, status: &str, tokens: u64) -> RunRecord {
        RunRecord {
            finished_at,
            agent_id: "a".into(),
            agent_type: "claude".into(),
            workspace: "/tmp/w".into(),
            status: status.into(),
            runtime_secs: 60,
            tokens_used: Some(tokens),
            estimated_cost_usd: Some(0.5),
        }
    }

    #[test]
    fn append_load_round_trip() {
        let path = std::env::temp_dir().join("kaiju-analytics-roundtrip.jsonl");
        let _ = std::fs::remove_file(&path);

        append(&path, &record_at(Utc::now(), "completed", 100)).unwrap();
        append(&path, &record_at(Utc::now(), "error", 20)).unwrap();

        let records = load(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, "completed");
        assert_eq!(records[1].status, "error");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn summarize_counts_only_records_in_window() {
        let now = Utc::now();
        let records = vec![
            record_at(now - chrono::Duration::days(1), "completed", 100),
            record_at(now - chrono::Duration::days(2), "stopped", 10),
            record_at(now - chrono::Duration::days(30), "error", 5),
        ];

        let week = summarize(&records, 7, now);
        assert_eq!(
            week,
            WindowSummary {
                days: 7,
                runs: 2,
                completed: 1,
                stopped: 1,
                error: 0,
                tokens_used: 110,
                estimated_cost_usd: 1.0,
            }
        );

        let full = summarize(&records, 45, now);
        assert_eq!(full.runs, 3);
        assert_eq!(full.error, 1);
        assert_eq!(full.tokens_used, 115);
    }

    #[test]
    fn prune_drops_expired_and_unparseable_lines() {
        let path = std::env::temp_dir().join("kaiju-analytics-prune.jsonl");
        let _ = std::fs::remove_file(&path);

        append(
            &path,
            &record_at(Utc::now() - chrono::Duration::days(60), "completed", 1),
        )
        .unwrap();
        append(&path, &record_at(Utc::now(), "completed", 2)).unwrap();
        std::fs::File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();

        let cutoff = Utc::now() - chrono::Duration::days(45);
        assert_eq!(prune_file(&path, cutoff), 2);
        let records = load(&path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tokens_used, Some(2));

        let _ = std::fs::remove_file(&path);
    }
}
