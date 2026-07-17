use std::collections::{HashMap, HashSet};

use crate::model::{AgentSnapshot, Status};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VisualState {
    Disconnected,
    Idle,
    Working,
    Waiting,
    Stuck,
    Limited,
    Error,
    Celebrate,
}

impl VisualState {
    /// Calm states get a slow, small idle bob; attention/celebrate states get a
    /// livelier one. Used only for animation tuning.
    pub fn is_calm(&self) -> bool {
        matches!(
            self,
            VisualState::Idle | VisualState::Working | VisualState::Disconnected
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttentionEvent {
    pub agent_id: String,
    pub status: Status,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReducerOutput {
    pub visual: VisualState,
    pub agents: Vec<AgentSnapshot>,
    pub ordered_attention: Vec<AgentSnapshot>,
    pub new_events: Vec<AttentionEvent>,
    pub unread_count: usize,
}

pub struct Reducer {
    prev: HashMap<String, Status>,
    unread: HashSet<String>,
    // Set true when the next snapshot must be treated as a baseline
    // (startup or first snapshot after a disconnect) — no events, no sound.
    baseline_next: bool,
}

impl Reducer {
    pub fn new() -> Self {
        Reducer {
            prev: HashMap::new(),
            unread: HashSet::new(),
            baseline_next: true,
        }
    }

    pub fn on_disconnect(&mut self) -> ReducerOutput {
        self.baseline_next = true;
        ReducerOutput {
            visual: VisualState::Disconnected,
            agents: Vec::new(),
            ordered_attention: Vec::new(),
            new_events: Vec::new(),
            unread_count: self.unread.len(),
        }
    }

    pub fn mark_read(&mut self, agent_id: &str) {
        self.unread.remove(agent_id);
    }

    pub fn on_snapshot(&mut self, agents: &[AgentSnapshot]) -> ReducerOutput {
        let is_baseline = self.baseline_next;
        self.baseline_next = false;

        let mut new_events = Vec::new();
        let mut just_completed = false;

        for a in agents {
            let previous = self.prev.get(&a.id).copied();
            let changed = previous != Some(a.status);
            if changed && !is_baseline {
                if a.status.is_attention() {
                    new_events.push(AttentionEvent {
                        agent_id: a.id.clone(),
                        status: a.status,
                        updated_at: a.updated_at.clone(),
                    });
                    self.unread.insert(a.id.clone());
                }
                if a.status == Status::Completed {
                    just_completed = true;
                }
            }
            // On a baseline snapshot, existing attention agents still count as unread
            // so the badge is populated — but no event/sound is produced.
            if is_baseline && a.status.is_attention() {
                self.unread.insert(a.id.clone());
            }
        }

        // Rebuild prev from the current snapshot; drop agents that disappeared.
        self.prev = agents.iter().map(|a| (a.id.clone(), a.status)).collect();
        let present: HashSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        self.unread.retain(|id| present.contains(id.as_str()));

        let ordered_attention = order_attention(agents);
        let visual = select_visual(agents, &ordered_attention, just_completed);

        ReducerOutput {
            visual,
            agents: agents.to_vec(),
            ordered_attention,
            new_events,
            unread_count: self.unread.len(),
        }
    }
}

impl Default for Reducer {
    fn default() -> Self {
        Self::new()
    }
}

fn priority(status: Status) -> u8 {
    match status {
        Status::WaitingForInput => 0,
        Status::Stuck => 1,
        Status::Limited => 2,
        Status::Error => 3,
        _ => 4,
    }
}

fn order_attention(agents: &[AgentSnapshot]) -> Vec<AgentSnapshot> {
    let mut attention: Vec<AgentSnapshot> = agents
        .iter()
        .filter(|a| a.status.is_attention())
        .cloned()
        .collect();
    // Priority ascending, then most recent updated_at first. RFC3339 strings
    // sort lexicographically in timestamp order for a fixed offset.
    attention.sort_by(|a, b| {
        priority(a.status)
            .cmp(&priority(b.status))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    attention
}

fn select_visual(
    agents: &[AgentSnapshot],
    ordered_attention: &[AgentSnapshot],
    just_completed: bool,
) -> VisualState {
    if just_completed {
        return VisualState::Celebrate;
    }
    if let Some(top) = ordered_attention.first() {
        return match top.status {
            Status::WaitingForInput => VisualState::Waiting,
            Status::Stuck => VisualState::Stuck,
            Status::Limited => VisualState::Limited,
            Status::Error => VisualState::Error,
            _ => VisualState::Working,
        };
    }
    if agents.iter().any(|a| a.status.is_active()) {
        VisualState::Working
    } else {
        VisualState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentSnapshot, Status};

    fn agent(id: &str, status: Status, updated: &str) -> AgentSnapshot {
        AgentSnapshot {
            id: id.into(),
            agent_type: "claude".into(),
            status,
            workspace: "/w".into(),
            prompt: None,
            updated_at: updated.into(),
            session_name: format!("kaiju-{id}"),
        }
    }

    #[test]
    fn initial_snapshot_emits_no_events_but_populates_attention() {
        let mut r = Reducer::new();
        let out = r.on_snapshot(&[agent("a", Status::WaitingForInput, "t1")]);
        assert!(out.new_events.is_empty(), "startup must not replay history");
        assert_eq!(out.ordered_attention.len(), 1);
        assert_eq!(out.unread_count, 1);
        assert_eq!(out.visual, VisualState::Waiting);
    }

    #[test]
    fn genuine_transition_emits_one_event() {
        let mut r = Reducer::new();
        r.on_snapshot(&[agent("a", Status::Running, "t1")]);
        let out = r.on_snapshot(&[agent("a", Status::WaitingForInput, "t2")]);
        assert_eq!(out.new_events.len(), 1);
        assert_eq!(out.new_events[0].agent_id, "a");
    }

    #[test]
    fn unchanged_attention_status_does_not_repeat() {
        let mut r = Reducer::new();
        r.on_snapshot(&[agent("a", Status::Running, "t1")]);
        r.on_snapshot(&[agent("a", Status::WaitingForInput, "t2")]);
        let out = r.on_snapshot(&[agent("a", Status::WaitingForInput, "t2")]);
        assert!(out.new_events.is_empty());
    }

    #[test]
    fn aggregate_priority_waiting_over_stuck_over_limited_over_error() {
        let mut r = Reducer::new();
        let out = r.on_snapshot(&[
            agent("e", Status::Error, "t1"),
            agent("l", Status::Limited, "t1"),
            agent("s", Status::Stuck, "t1"),
            agent("w", Status::WaitingForInput, "t1"),
        ]);
        assert_eq!(out.visual, VisualState::Waiting);
        assert_eq!(out.ordered_attention[0].id, "w");
        assert_eq!(out.ordered_attention[1].id, "s");
        assert_eq!(out.ordered_attention[2].id, "l");
        assert_eq!(out.ordered_attention[3].id, "e");
    }

    #[test]
    fn equal_priority_sorts_by_most_recent_updated_at() {
        let mut r = Reducer::new();
        let out = r.on_snapshot(&[
            agent("old", Status::WaitingForInput, "2026-07-16T10:00:00+00:00"),
            agent("new", Status::WaitingForInput, "2026-07-16T10:05:00+00:00"),
        ]);
        assert_eq!(out.ordered_attention[0].id, "new");
    }

    #[test]
    fn active_agents_show_working_idle_when_none() {
        let mut r = Reducer::new();
        assert_eq!(
            r.on_snapshot(&[agent("a", Status::Running, "t1")]).visual,
            VisualState::Working
        );
        let mut r2 = Reducer::new();
        assert_eq!(r2.on_snapshot(&[]).visual, VisualState::Idle);
    }

    #[test]
    fn mark_read_reduces_unread_without_touching_status() {
        let mut r = Reducer::new();
        r.on_snapshot(&[agent("a", Status::WaitingForInput, "t1")]);
        r.mark_read("a");
        let out = r.on_snapshot(&[agent("a", Status::WaitingForInput, "t1")]);
        assert_eq!(out.unread_count, 0);
        assert_eq!(out.ordered_attention[0].status, Status::WaitingForInput);
    }

    #[test]
    fn reconnect_after_disconnect_does_not_replay_events() {
        let mut r = Reducer::new();
        r.on_snapshot(&[agent("a", Status::Running, "t1")]);
        r.on_disconnect();
        let out = r.on_snapshot(&[agent("a", Status::WaitingForInput, "t2")]);
        assert!(
            out.new_events.is_empty(),
            "first snapshot after reconnect is a baseline"
        );
        assert_eq!(out.visual, VisualState::Waiting);
    }

    #[test]
    fn completed_transition_shows_celebrate_once() {
        let mut r = Reducer::new();
        r.on_snapshot(&[agent("a", Status::Running, "t1")]);
        let out = r.on_snapshot(&[agent("a", Status::Completed, "t2")]);
        assert_eq!(out.visual, VisualState::Celebrate);
        // next poll with same terminal state is no longer a celebration
        let out2 = r.on_snapshot(&[agent("a", Status::Completed, "t2")]);
        assert_ne!(out2.visual, VisualState::Celebrate);
    }
}
