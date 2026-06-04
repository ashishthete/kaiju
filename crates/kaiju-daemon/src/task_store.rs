//! Thread-safe, persisted store of queued tasks.
//!
//! Mirrors `AgentStore`: state lives in memory behind an `RwLock`, and every
//! mutation is flushed to JSON when a path is configured so the backlog
//! survives a daemon restart.

use crate::persist;
use kaiju_core::task::{Task, TaskSpec, TaskStatus};
use kaiju_core::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct TaskStore {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
    path: Option<Arc<PathBuf>>,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStore {
    /// In-memory store with no persistence.
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            path: None,
        }
    }

    /// Load any persisted tasks from `path` and persist future mutations there.
    pub fn load_or_new(path: PathBuf) -> Result<Self> {
        let loaded: Vec<Task> = persist::load(&path)?;
        let map = loaded.into_iter().map(|t| (t.id.clone(), t)).collect();
        Ok(Self {
            tasks: Arc::new(RwLock::new(map)),
            path: Some(Arc::new(path)),
        })
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let snapshot: Vec<Task> = {
            let tasks = self.tasks.read().unwrap();
            tasks.values().cloned().collect()
        };
        if let Err(e) = persist::save(path, &snapshot) {
            tracing::warn!("failed to persist task store to {}: {e}", path.display());
        }
    }

    pub fn enqueue(&self, spec: TaskSpec) -> Task {
        let task = Task::new(spec);
        {
            let mut tasks = self.tasks.write().unwrap();
            tasks.insert(task.id.clone(), task.clone());
        }
        self.persist();
        task
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Task> {
        self.tasks.read().unwrap().values().cloned().collect()
    }

    pub fn count_running(&self) -> usize {
        self.tasks
            .read()
            .unwrap()
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .count()
    }

    pub fn running(&self) -> Vec<Task> {
        self.tasks
            .read()
            .unwrap()
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .cloned()
            .collect()
    }

    /// The `n` oldest queued tasks, oldest first.
    pub fn next_queued(&self, n: usize) -> Vec<Task> {
        let mut queued: Vec<Task> = self
            .tasks
            .read()
            .unwrap()
            .values()
            .filter(|t| t.status == TaskStatus::Queued)
            .cloned()
            .collect();
        queued.sort_by_key(|t| t.created_at);
        queued.truncate(n);
        queued
    }

    pub fn mark_running(&self, id: &str, agent_id: String) -> bool {
        self.mutate(id, |t| t.mark_running(agent_id))
    }

    pub fn finish(&self, id: &str, status: TaskStatus) -> bool {
        self.mutate(id, |t| t.finish(status))
    }

    pub fn fail(&self, id: &str, reason: String) -> bool {
        self.mutate(id, |t| t.fail(reason))
    }

    /// Cancel a non-terminal task. Returns the updated task (whose `agent_id`
    /// the caller may stop), or `None` if missing or already terminal.
    pub fn cancel(&self, id: &str) -> Option<Task> {
        let result = {
            let mut tasks = self.tasks.write().unwrap();
            match tasks.get_mut(id) {
                Some(t) if !t.status.is_terminal() => {
                    t.finish(TaskStatus::Canceled);
                    Some(t.clone())
                }
                _ => None,
            }
        };
        if result.is_some() {
            self.persist();
        }
        result
    }

    fn mutate(&self, id: &str, f: impl FnOnce(&mut Task)) -> bool {
        let updated = {
            let mut tasks = self.tasks.write().unwrap();
            match tasks.get_mut(id) {
                Some(t) => {
                    f(t);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaiju_core::agent::AgentType;
    use std::path::PathBuf;

    fn spec(prompt: &str) -> TaskSpec {
        TaskSpec {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: Some(prompt.to_string()),
            extra_args: vec![],
            isolate: false,
        }
    }

    #[test]
    fn enqueue_and_get() {
        let store = TaskStore::new();
        let task = store.enqueue(spec("a"));
        assert_eq!(store.get(&task.id).unwrap().status, TaskStatus::Queued);
    }

    #[test]
    fn next_queued_respects_limit_and_order() {
        let store = TaskStore::new();
        let a = store.enqueue(spec("a"));
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _b = store.enqueue(spec("b"));

        let next = store.next_queued(1);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id, a.id); // oldest first
    }

    #[test]
    fn count_running_tracks_marked_tasks() {
        let store = TaskStore::new();
        let t = store.enqueue(spec("a"));
        assert_eq!(store.count_running(), 0);
        store.mark_running(&t.id, "agent-1".to_string());
        assert_eq!(store.count_running(), 1);
    }

    #[test]
    fn cancel_queued_task_becomes_canceled() {
        let store = TaskStore::new();
        let t = store.enqueue(spec("a"));
        let canceled = store.cancel(&t.id).unwrap();
        assert_eq!(canceled.status, TaskStatus::Canceled);
        // Already terminal -> cancel again is a no-op.
        assert!(store.cancel(&t.id).is_none());
    }
}
