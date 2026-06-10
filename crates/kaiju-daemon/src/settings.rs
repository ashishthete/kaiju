//! Global defaults applied to every newly-created agent.
//!
//! Optional, like the pricing file: drop a JSON file at `~/.kaiju/config.json`
//! (override with `KAIJU_CONFIG`) and its values fill in fields a request leaves
//! unset, so you don't repeat the same model / flags / isolation on every spawn.
//!
//! ```json
//! {
//!   "default_agent_type": "claude",
//!   "default_model": "claude-opus-4-8",
//!   "default_extra_args": ["--permission-mode", "acceptEdits"],
//!   "isolate": true
//! }
//! ```
//!
//! Everything is optional; an absent or malformed file means "no defaults".
//! Edits are picked up on daemon restart (read once at startup).

use std::path::PathBuf;

use kaiju_core::agent::AgentConfig;
use serde::Deserialize;

/// Daemon-wide defaults for new agents.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Settings {
    /// Agent type to use when a request doesn't specify one.
    #[serde(default)]
    pub default_agent_type: Option<String>,
    /// Model to use when a request doesn't specify one.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Extra CLI args prepended to every agent's own args.
    #[serde(default)]
    pub default_extra_args: Vec<String>,
    /// Run agents in an isolated git worktree by default.
    #[serde(default)]
    pub isolate: bool,
}

impl Settings {
    /// Fill in a config's unset fields from these defaults: model falls back to
    /// `default_model`, and `default_extra_args` are prepended to the config's
    /// own args (globals first, so per-agent args can still override later).
    pub fn apply(&self, mut config: AgentConfig) -> AgentConfig {
        if config.model.is_none() {
            config.model = self.default_model.clone();
        }
        if !self.default_extra_args.is_empty() {
            let mut args = self.default_extra_args.clone();
            args.extend(config.extra_args);
            config.extra_args = args;
        }
        config
    }
}

/// Path to the settings file: `KAIJU_CONFIG`, else `~/.kaiju/config.json`.
fn settings_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_CONFIG") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("config.json"))
}

/// Read and parse the settings file; defaults when absent or malformed (a typo
/// shouldn't stop the daemon — it just means no global defaults apply).
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaiju_core::agent::AgentType;
    use std::path::PathBuf;

    fn config(model: Option<&str>, extra: &[&str]) -> AgentConfig {
        AgentConfig {
            agent_type: AgentType::Claude,
            model: model.map(str::to_string),
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: extra.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn apply_fills_model_only_when_unset() {
        let s = Settings {
            default_model: Some("claude-opus-4-8".to_string()),
            ..Settings::default()
        };
        assert_eq!(s.apply(config(None, &[])).model.as_deref(), Some("claude-opus-4-8"));
        // An explicit model is preserved.
        assert_eq!(
            s.apply(config(Some("claude-haiku-4-5"), &[])).model.as_deref(),
            Some("claude-haiku-4-5")
        );
    }

    #[test]
    fn apply_prepends_default_extra_args() {
        let s = Settings {
            default_extra_args: vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
            ..Settings::default()
        };
        let out = s.apply(config(None, &["--foo"]));
        assert_eq!(out.extra_args, vec!["--permission-mode", "acceptEdits", "--foo"]);
    }

    #[test]
    fn default_settings_change_nothing() {
        let s = Settings::default();
        let out = s.apply(config(Some("m"), &["--x"]));
        assert_eq!(out.model.as_deref(), Some("m"));
        assert_eq!(out.extra_args, vec!["--x"]);
    }
}
