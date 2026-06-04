use kaiju_core::adapter::Adapter;
use kaiju_core::agent::AgentType;
use std::collections::HashMap;

use crate::claude::ClaudeAdapter;
use crate::codex::CodexAdapter;
use crate::gemini::GeminiAdapter;

/// Registry of available CLI adapters.
///
/// Maps agent types to their adapter implementations.
pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn Adapter>>,
}

impl AdapterRegistry {
    /// Create a registry with all built-in adapters.
    pub fn with_defaults() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(ClaudeAdapter));
        registry.register(Box::new(CodexAdapter));
        registry.register(Box::new(GeminiAdapter));
        registry
    }

    /// Register a custom adapter.
    pub fn register(&mut self, adapter: Box<dyn Adapter>) {
        let key = adapter.agent_type().to_string();
        self.adapters.insert(key, adapter);
    }

    /// Look up an adapter by agent type.
    pub fn get(&self, agent_type: &AgentType) -> Option<&dyn Adapter> {
        self.adapters
            .get(&agent_type.to_string())
            .map(|a| a.as_ref())
    }

    /// List all registered agent types.
    pub fn available_types(&self) -> Vec<&str> {
        self.adapters.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_all_adapters() {
        let registry = AdapterRegistry::with_defaults();
        assert!(registry.get(&AgentType::Claude).is_some());
        assert!(registry.get(&AgentType::Codex).is_some());
        assert!(registry.get(&AgentType::Gemini).is_some());
    }

    #[test]
    fn unknown_type_returns_none() {
        let registry = AdapterRegistry::with_defaults();
        assert!(registry
            .get(&AgentType::Custom("aider".to_string()))
            .is_none());
    }

    #[test]
    fn available_types_lists_all() {
        let registry = AdapterRegistry::with_defaults();
        let types = registry.available_types();
        assert_eq!(types.len(), 3);
    }
}
