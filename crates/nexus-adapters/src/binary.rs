//! Resolving the executable used to launch each agent CLI.

/// Resolve the binary used to launch an agent CLI.
///
/// Defaults to `default` (located via `PATH`), but an environment override lets
/// callers swap in a stub for tests or pin an absolute path — e.g.
/// `NEXUS_CLAUDE_BIN=/path/to/fake-claude`. An unset or blank value falls back
/// to the default.
pub(crate) fn agent_binary(env_key: &str, default: &str) -> String {
    std::env::var(env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test uses a unique env key so they never race with one another or
    // with the adapters' real keys, even when run in parallel.

    #[test]
    fn falls_back_to_default_when_unset() {
        std::env::remove_var("NEXUS_AGENT_BIN_TEST_UNSET");
        assert_eq!(
            agent_binary("NEXUS_AGENT_BIN_TEST_UNSET", "claude"),
            "claude"
        );
    }

    #[test]
    fn env_override_wins() {
        std::env::set_var("NEXUS_AGENT_BIN_TEST_SET", "/opt/fake/claude");
        assert_eq!(
            agent_binary("NEXUS_AGENT_BIN_TEST_SET", "claude"),
            "/opt/fake/claude"
        );
        std::env::remove_var("NEXUS_AGENT_BIN_TEST_SET");
    }

    #[test]
    fn blank_override_falls_back_to_default() {
        std::env::set_var("NEXUS_AGENT_BIN_TEST_BLANK", "   ");
        assert_eq!(agent_binary("NEXUS_AGENT_BIN_TEST_BLANK", "codex"), "codex");
        std::env::remove_var("NEXUS_AGENT_BIN_TEST_BLANK");
    }
}
