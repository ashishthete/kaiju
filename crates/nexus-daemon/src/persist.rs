//! JSON file persistence for the stores (agents and tasks).
//!
//! Two small generic IO functions, kept separate from the in-memory stores so
//! the serialization concern is isolated and testable on its own.

use nexus_core::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// Load a list of items from a JSON file. A missing file is not an error — it
/// yields an empty list (first run).
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(path)?;
    let items = serde_json::from_str(&data)?;
    Ok(items)
}

/// Write a list of items to a JSON file atomically (write to a temp file, then
/// rename) so a crash mid-write cannot corrupt the existing state.
pub fn save<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(items)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::agent::{Agent, AgentConfig, AgentType};
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!("kaiju-test-{}-{}.json", std::process::id(), name);
        path.push(unique);
        path
    }

    fn sample_agent() -> Agent {
        Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: Some("sonnet".to_string()),
            workspace: PathBuf::from("/tmp/project"),
            prompt: Some("do the thing".to_string()),
            extra_args: vec![],
        })
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        let agents: Vec<Agent> = load(&path).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let path = temp_path("roundtrip");
        let agent = sample_agent();
        let id = agent.id.clone();

        save(&path, &[agent]).unwrap();
        let loaded: Vec<Agent> = load(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, id);
        assert_eq!(loaded[0].agent_type, AgentType::Claude);

        let _ = std::fs::remove_file(&path);
    }
}
