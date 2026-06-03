use chrono::{DateTime, Utc};
use nexus_core::agent::{Agent, AgentMetrics, AgentStatus};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// In-memory store for agent state.
///
/// Thread-safe via RwLock. Will be replaced with persistent storage later.
#[derive(Clone)]
pub struct AgentStore {
    agents: Arc<RwLock<HashMap<String, Agent>>>,
}

impl AgentStore {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, agent: Agent) {
        let mut agents = self.agents.write().unwrap();
        agents.insert(agent.id.clone(), agent);
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
        let mut agents = self.agents.write().unwrap();
        if let Some(agent) = agents.get_mut(id) {
            agent.update_status(status);
            true
        } else {
            false
        }
    }

    pub fn update_metrics(&self, id: &str, metrics: AgentMetrics) -> bool {
        let mut agents = self.agents.write().unwrap();
        if let Some(agent) = agents.get_mut(id) {
            agent.update_metrics(metrics);
            true
        } else {
            false
        }
    }

    /// Record the launch time and move the agent to Running.
    pub fn mark_started(&self, id: &str, now: DateTime<Utc>) -> bool {
        let mut agents = self.agents.write().unwrap();
        if let Some(agent) = agents.get_mut(id) {
            agent.mark_started(now);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, id: &str) -> Option<Agent> {
        let mut agents = self.agents.write().unwrap();
        agents.remove(id)
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
}
