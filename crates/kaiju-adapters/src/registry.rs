use kaiju_core::adapter::Adapter;
use kaiju_core::agent::AgentType;
use std::collections::HashMap;

use crate::claude::ClaudeAdapter;
use crate::codex::CodexAdapter;
use crate::custom::CustomAdapter;
use crate::gemini::GeminiAdapter;

/// Registry of available CLI adapters.
///
/// Maps agent types to their adapter implementations. Any `AgentType::Custom`
/// not explicitly registered falls back to the generic [`CustomAdapter`].
pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn Adapter>>,
    custom: CustomAdapter,
}

impl AdapterRegistry {
    /// Create a registry with all built-in adapters.
    pub fn with_defaults() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
            custom: CustomAdapter,
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

    /// Look up an adapter by agent type. Falls back to the generic
    /// [`CustomAdapter`] for any unregistered `AgentType::Custom`.
    pub fn get(&self, agent_type: &AgentType) -> Option<&dyn Adapter> {
        if let Some(adapter) = self.adapters.get(&agent_type.to_string()) {
            return Some(adapter.as_ref());
        }
        match agent_type {
            AgentType::Custom(_) => Some(&self.custom),
            _ => None,
        }
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
    fn custom_type_falls_back_to_generic_adapter() {
        let registry = AdapterRegistry::with_defaults();
        let adapter = registry.get(&AgentType::Custom("aider".to_string()));
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().display_name(), "Custom CLI");
    }

    #[test]
    fn available_types_lists_all() {
        let registry = AdapterRegistry::with_defaults();
        let types = registry.available_types();
        assert_eq!(types.len(), 3);
    }
}
