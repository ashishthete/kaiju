//! Persisting an agent's recent terminal output so its logs survive the tmux
//! session ending.
//!
//! Live logs come from `capture-pane`, which returns nothing once the session is
//! gone (a completed/stopped agent). The monitor mirrors each fresh capture here
//! to `~/.kaiju/logs/<agent-uuid>.log` (override the dir with `KAIJU_LOGS`);
//! `get_logs` falls back to it when the session no longer exists.
//!
//! Retention: log files are kept after the agent is deleted so they can feed
//! analytics, and rotated out once older than `KAIJU_LOG_RETENTION_DAYS`
//! (default 45). The monitor calls [`purge_expired`] daily.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long persisted logs are kept before rotation deletes them.
const DEFAULT_RETENTION_DAYS: u64 = 45;

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

/// Retention age: `KAIJU_LOG_RETENTION_DAYS`, else 45 days. Shared with the
/// analytics records so both age out together.
pub(crate) fn retention() -> Duration {
    let days = std::env::var("KAIJU_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS);
    Duration::from_secs(days * 24 * 60 * 60)
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

/// Delete log files older than the retention window. Returns how many were
/// removed. Best-effort like [`save`]: unreadable entries are skipped.
pub fn purge_expired() -> usize {
    match logs_dir() {
        Some(dir) => purge_dir(&dir, retention()),
        None => 0,
    }
}

fn purge_dir(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut purged = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        let expired = now.duration_since(modified).is_ok_and(|age| age > max_age);
        if expired && std::fs::remove_file(&path).is_ok() {
            purged += 1;
        }
    }
    purged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join("kaiju-logstore-test");
        std::env::set_var("KAIJU_LOGS", &dir);
        let id = "round-trip-agent";

        save(id, "hello logs");
        assert_eq!(load(id).as_deref(), Some("hello logs"));

        // A later capture overwrites the previous one.
        save(id, "newer output");
        assert_eq!(load(id).as_deref(), Some("newer output"));

        std::env::remove_var("KAIJU_LOGS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn purge_removes_only_expired_logs() {
        let dir = std::env::temp_dir().join("kaiju-logstore-purge-test");
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old-agent.log");
        let fresh = dir.join("fresh-agent.log");
        let other = dir.join("not-a-log.txt");
        for path in [&old, &fresh, &other] {
            std::fs::write(path, "output").unwrap();
        }

        // Backdate the old log past the retention window.
        let past = SystemTime::now() - Duration::from_secs(46 * 24 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();

        assert_eq!(purge_dir(&dir, Duration::from_secs(45 * 24 * 60 * 60)), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
