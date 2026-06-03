use crate::persist;
use chrono::{DateTime, Utc};
use nexus_core::agent::{Agent, AgentMetrics, AgentStatus};
use nexus_core::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Thread-safe store for agent state.
///
/// State lives in memory behind an `RwLock`. When a `path` is set, every
/// mutation is flushed to a JSON file so the daemon can recover its fleet
/// after a restart. With no `path` (e.g. in tests) the store is purely
/// in-memory.
#[derive(Clone)]
pub struct AgentStore {
    agents: Arc<RwLock<HashMap<String, Agent>>>,
    path: Option<Arc<PathBuf>>,
}

impl Default for AgentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentStore {
    /// In-memory store with no persistence.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            path: None,
        }
    }

    /// Load any persisted agents from `path` (empty if the file is absent) and
    /// return a store that flushes future mutations back to that file.
    pub fn load_or_new(path: PathBuf) -> Result<Self> {
        let loaded = persist::load(&path)?;
        let map = loaded
            .into_iter()
            .map(|agent| (agent.id.clone(), agent))
            .collect();
        Ok(Self {
            agents: Arc::new(RwLock::new(map)),
            path: Some(Arc::new(path)),
        })
    }

    /// Flush the current state to disk if a path is configured. Persistence
    /// failures are logged, not propagated, so a disk hiccup never fails an
    /// API call.
    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let snapshot: Vec<Agent> = {
            let agents = self.agents.read().unwrap();
            agents.values().cloned().collect()
        };
        if let Err(e) = persist::save(path, &snapshot) {
            tracing::warn!("failed to persist agent store to {}: {e}", path.display());
        }
    }

    pub fn insert(&self, agent: Agent) {
        {
            let mut agents = self.agents.write().unwrap();
            agents.insert(agent.id.clone(), agent);
        }
        self.persist();
    }

    pub fn get(&self, id: &str) -> Option<Agent> {
        let agents = self.agents.read().unwrap();
        agents.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Agent> {
        let agents = self.agents.read().unwrap();
        agents.values().cloned().collect()
    }

    pub fn list_active(&self) -> Vec<Agent> {
        let agents = self.agents.read().unwrap();
        agents
            .values()
            .filter(|a| a.status.is_active())
            .cloned()
            .collect()
    }

    pub fn update_status(&self, id: &str, status: AgentStatus) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.update_status(status);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }

    pub fn update_metrics(&self, id: &str, metrics: AgentMetrics) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.update_metrics(metrics);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }

    /// Record the launch time and move the agent to Running.
    pub fn mark_started(&self, id: &str, now: DateTime<Utc>) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.mark_started(now);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }

    pub fn set_worktree_path(&self, id: &str, path: PathBuf) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.set_worktree(path);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }

    pub fn remove(&self, id: &str) -> Option<Agent> {
        let removed = {
            let mut agents = self.agents.write().unwrap();
            agents.remove(id)
        };
        if removed.is_some() {
            self.persist();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::agent::{AgentConfig, AgentType};
    use std::path::PathBuf;

    fn test_agent(agent_type: AgentType) -> Agent {
        Agent::new(AgentConfig {
            agent_type,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        })
    }

    #[test]
    fn insert_and_get() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Claude);
        let id = agent.id.clone();

        store.insert(agent);
        let retrieved = store.get(&id).unwrap();
        assert_eq!(retrieved.agent_type, AgentType::Claude);
    }

    #[test]
    fn get_missing_returns_none() {
        let store = AgentStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn list_returns_all() {
        let store = AgentStore::new();
        store.insert(test_agent(AgentType::Claude));
        store.insert(test_agent(AgentType::Codex));
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn list_active_filters_terminal() {
        let store = AgentStore::new();
        let mut agent = test_agent(AgentType::Claude);
        agent.update_status(AgentStatus::Running);
        store.insert(agent);

        let mut completed = test_agent(AgentType::Codex);
        completed.update_status(AgentStatus::Completed);
        store.insert(completed);

        assert_eq!(store.list_active().len(), 1);
    }

    #[test]
    fn update_status_works() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Gemini);
        let id = agent.id.clone();
        store.insert(agent);

        assert!(store.update_status(&id, AgentStatus::Running));
        assert_eq!(store.get(&id).unwrap().status, AgentStatus::Running);
    }

    #[test]
    fn remove_works() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Claude);
        let id = agent.id.clone();
        store.insert(agent);

        let removed = store.remove(&id).unwrap();
        assert_eq!(removed.agent_type, AgentType::Claude);
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn update_metrics_works() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Claude);
        let id = agent.id.clone();
        store.insert(agent);

        let metrics = AgentMetrics {
            runtime_secs: 42,
            tokens_used: Some(1000),
            estimated_cost_usd: Some(0.25),
        };
        assert!(store.update_metrics(&id, metrics));

        let stored = store.get(&id).unwrap();
        assert_eq!(stored.metrics.runtime_secs, 42);
        assert_eq!(stored.metrics.tokens_used, Some(1000));
    }

    #[test]
    fn update_metrics_missing_returns_false() {
        let store = AgentStore::new();
        assert!(!store.update_metrics("nope", AgentMetrics::default()));
    }

    #[test]
    fn mark_started_sets_started_at_and_running() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Claude);
        let id = agent.id.clone();
        store.insert(agent);

        let now = Utc::now();
        assert!(store.mark_started(&id, now));

        let stored = store.get(&id).unwrap();
        assert_eq!(stored.started_at, Some(now));
        assert_eq!(stored.status, AgentStatus::Running);
    }

    #[test]
    fn mutations_persist_and_reload_from_disk() {
        let mut path = std::env::temp_dir();
        path.push(format!("nexus-store-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let id = {
            let store = AgentStore::load_or_new(path.clone()).unwrap();
            let agent = test_agent(AgentType::Codex);
            let id = agent.id.clone();
            store.insert(agent);
            id
        };

        // A fresh store loading the same file sees the persisted agent.
        let reloaded = AgentStore::load_or_new(path.clone()).unwrap();
        assert_eq!(reloaded.get(&id).unwrap().agent_type, AgentType::Codex);

        let _ = std::fs::remove_file(&path);
    }
}
