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

use kaiju_core::agent::{AgentConfig, AgentMetrics};
use kaiju_core::{NexusError, Result};
use serde::{Deserialize, Serialize};

/// Daemon-wide defaults for new agents.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
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
    /// Auto-stop an agent once its tokens-used reaches this many.
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Auto-stop an agent once its estimated cost reaches this many USD
    /// (requires pricing — [`crate::settings`] / `~/.kaiju/pricing.json`).
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
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

    /// If a configured budget is reached, return a human-readable reason the
    /// agent should be stopped; otherwise `None`. A threshold that isn't set, or
    /// a metric that isn't known yet, never triggers.
    pub fn budget_exceeded(&self, metrics: &AgentMetrics) -> Option<String> {
        if let (Some(max), Some(used)) = (self.max_tokens, metrics.tokens_used) {
            if used >= max {
                return Some(format!("token budget reached ({used} >= {max})"));
            }
        }
        if let (Some(max), Some(cost)) = (self.max_cost_usd, metrics.estimated_cost_usd) {
            if cost >= max {
                return Some(format!("cost budget reached (${cost:.2} >= ${max:.2})"));
            }
        }
        None
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

/// Persist settings to the config file (pretty JSON), creating the directory if
/// needed. Errors when there's no writable path (no `KAIJU_CONFIG` and no `HOME`).
pub fn save(settings: &Settings) -> Result<()> {
    let path = settings_path().ok_or_else(|| {
        NexusError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config path (set KAIJU_CONFIG or HOME)",
        ))
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(NexusError::Io)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| NexusError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, json).map_err(NexusError::Io)?;
    Ok(())
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
        assert_eq!(
            s.apply(config(None, &[])).model.as_deref(),
            Some("claude-opus-4-8")
        );
        // An explicit model is preserved.
        assert_eq!(
            s.apply(config(Some("claude-haiku-4-5"), &[]))
                .model
                .as_deref(),
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
        assert_eq!(
            out.extra_args,
            vec!["--permission-mode", "acceptEdits", "--foo"]
        );
    }

    #[test]
    fn default_settings_change_nothing() {
        let s = Settings::default();
        let out = s.apply(config(Some("m"), &["--x"]));
        assert_eq!(out.model.as_deref(), Some("m"));
        assert_eq!(out.extra_args, vec!["--x"]);
    }

    fn metrics(tokens: Option<u64>, cost: Option<f64>) -> AgentMetrics {
        AgentMetrics {
            runtime_secs: 0,
            tokens_used: tokens,
            estimated_cost_usd: cost,
        }
    }

    #[test]
    fn budget_triggers_at_or_above_token_threshold() {
        let s = Settings {
            max_tokens: Some(1000),
            ..Settings::default()
        };
        assert!(s.budget_exceeded(&metrics(Some(1000), None)).is_some());
        assert!(s.budget_exceeded(&metrics(Some(1500), None)).is_some());
        assert!(s.budget_exceeded(&metrics(Some(999), None)).is_none());
    }

    #[test]
    fn budget_triggers_on_cost_threshold() {
        let s = Settings {
            max_cost_usd: Some(5.0),
            ..Settings::default()
        };
        assert!(s.budget_exceeded(&metrics(None, Some(5.0))).is_some());
        assert!(s.budget_exceeded(&metrics(None, Some(4.99))).is_none());
    }

    #[test]
    fn no_budget_or_unknown_metric_never_triggers() {
        // No thresholds set.
        assert!(Settings::default()
            .budget_exceeded(&metrics(Some(10_000_000), Some(999.0)))
            .is_none());
        // Threshold set but the metric isn't known yet.
        let s = Settings {
            max_tokens: Some(10),
            ..Settings::default()
        };
        assert!(s.budget_exceeded(&metrics(None, None)).is_none());
    }
}
