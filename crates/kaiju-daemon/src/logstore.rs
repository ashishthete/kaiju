//! Persisting an agent's recent terminal output so its logs survive the tmux
//! session ending.
//!
//! Live logs come from `capture-pane`, which returns nothing once the session is
//! gone (a completed/stopped agent). The monitor mirrors each fresh capture here
//! to `~/.kaiju/logs/<id>.log` (override the dir with `KAIJU_LOGS`); `get_logs`
//! falls back to it when the session no longer exists.

use std::path::PathBuf;

/// Directory holding persisted per-agent logs: `KAIJU_LOGS`, else `~/.kaiju/logs`.
fn logs_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_LOGS") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("logs"))
}

fn log_file(id: &str) -> Option<PathBuf> {
    logs_dir().map(|dir| dir.join(format!("{id}.log")))
}

/// Persist the latest captured output for `id`. Best-effort: a failure to write
/// must never disrupt monitoring, so errors are swallowed.
pub fn save(id: &str, content: &str) {
    let Some(dir) = logs_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(format!("{id}.log")), content);
}

/// The last persisted output for `id`, if any.
pub fn load(id: &str) -> Option<String> {
    std::fs::read_to_string(log_file(id)?).ok()
}

/// Delete the persisted log for `id` (on agent removal).
pub fn remove(id: &str) {
    if let Some(path) = log_file(id) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_remove_round_trip() {
        let dir = std::env::temp_dir().join("kaiju-logstore-test");
        std::env::set_var("KAIJU_LOGS", &dir);
        let id = "round-trip-agent";

        save(id, "hello logs");
        assert_eq!(load(id).as_deref(), Some("hello logs"));

        // A later capture overwrites the previous one.
        save(id, "newer output");
        assert_eq!(load(id).as_deref(), Some("newer output"));

        remove(id);
        assert_eq!(load(id), None);

        std::env::remove_var("KAIJU_LOGS");
    }
}
