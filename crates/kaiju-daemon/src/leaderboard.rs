//! Per-repo bake-off history: which model won, whose tests passed. Persisted to
//! `~/.kaiju/leaderboard.json` (override `KAIJU_LEADERBOARD`). Mirrors the
//! settings/devices stores: optional file, best-effort load, explicit save.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use kaiju_core::{NexusError, Result};
use serde::{Deserialize, Serialize};

/// One candidate's outcome within a judged bake-off.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CandidateResult {
    pub agent_type: String,
    #[serde(default)]
    pub model: Option<String>,
    /// `Some(true/false)` when a test command was run; `None` if diff-only.
    #[serde(default)]
    pub tests_passed: Option<bool>,
    #[serde(default)]
    pub is_winner: bool,
}

/// One judged bake-off (a compare group that was judged).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BakeoffRecord {
    pub workspace: String,
    pub prompt: String,
    pub at: DateTime<Utc>,
    pub candidates: Vec<CandidateResult>,
}

/// Aggregated standing for one model in a workspace.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AgentStanding {
    pub agent_type: String,
    pub runs: u32,
    pub wins: u32,
    pub tests_passed: u32,
    pub tests_run: u32,
}

/// Pure: standings for `workspace`, sorted by wins then tests-passed, descending.
pub fn standings(records: &[BakeoffRecord], workspace: &str) -> Vec<AgentStanding> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, AgentStanding> = BTreeMap::new();
    for rec in records.iter().filter(|r| r.workspace == workspace) {
        for c in &rec.candidates {
            let s = map.entry(c.agent_type.clone()).or_insert_with(|| AgentStanding {
                agent_type: c.agent_type.clone(),
                runs: 0,
                wins: 0,
                tests_passed: 0,
                tests_run: 0,
            });
            s.runs += 1;
            if c.is_winner {
                s.wins += 1;
            }
            if let Some(passed) = c.tests_passed {
                s.tests_run += 1;
                if passed {
                    s.tests_passed += 1;
                }
            }
        }
    }
    let mut v: Vec<AgentStanding> = map.into_values().collect();
    v.sort_by(|a, b| b.wins.cmp(&a.wins).then(b.tests_passed.cmp(&a.tests_passed)));
    v
}

fn leaderboard_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_LEADERBOARD") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("leaderboard.json"))
}

/// Read all records; an absent/malformed file means "no history".
pub fn load() -> Vec<BakeoffRecord> {
    let Some(path) = leaderboard_path() else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(path) else { return Vec::new() };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Append one record (best-effort persist).
pub fn record(entry: BakeoffRecord) -> Result<()> {
    let path = leaderboard_path().ok_or_else(|| {
        NexusError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no leaderboard path (set KAIJU_LEADERBOARD or HOME)",
        ))
    })?;
    let mut all = load();
    all.push(entry);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(NexusError::Io)?;
    }
    let json = serde_json::to_string_pretty(&all)
        .map_err(|e| NexusError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, json).map_err(NexusError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> { "2026-06-15T00:00:00Z".parse().unwrap() }

    fn rec(ws: &str, cands: Vec<CandidateResult>) -> BakeoffRecord {
        BakeoffRecord { workspace: ws.into(), prompt: "p".into(), at: at(), candidates: cands }
    }
    fn cand(t: &str, win: bool, tests: Option<bool>) -> CandidateResult {
        CandidateResult { agent_type: t.into(), model: None, tests_passed: tests, is_winner: win }
    }

    #[test]
    fn standings_aggregate_wins_and_tests_for_workspace() {
        let records = vec![
            rec("/repo", vec![cand("claude", true, Some(true)), cand("codex", false, Some(false))]),
            rec("/repo", vec![cand("claude", false, Some(false)), cand("codex", true, Some(true))]),
            rec("/other", vec![cand("claude", true, None)]),
        ];
        let s = standings(&records, "/repo");
        assert_eq!(s.len(), 2);
        let claude = s.iter().find(|x| x.agent_type == "claude").unwrap();
        assert_eq!(claude.runs, 2);
        assert_eq!(claude.wins, 1);
        assert_eq!(claude.tests_run, 2);
        assert_eq!(claude.tests_passed, 1);
    }

    #[test]
    fn standings_empty_for_unknown_workspace() {
        assert!(standings(&[], "/nope").is_empty());
    }
}
