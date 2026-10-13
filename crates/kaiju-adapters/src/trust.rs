//! Pre-trust an agent's working directory so a CLI's first-run "do you trust
//! this folder?" prompt doesn't stall an unattended / bake-off launch.
//!
//! Isolated agents run in a brand-new git worktree the CLI has never seen, so
//! Claude and Codex both raise a folder-trust gate on launch — and their
//! auto-approve flags (`--dangerously-skip-permissions`, `--full-auto`) do
//! *not* skip it. We seed each CLI's own trust store for the path before launch:
//!
//! - Claude: `~/.claude.json` → `projects."<path>".hasTrustDialogAccepted = true`
//! - Codex:  `~/.codex/config.toml` → `[projects."<path>"] trust_level = "trusted"`
//!
//! The pure transforms ([`claude_trusted_json`], [`codex_trust_appended`]) are
//! unit-tested; the IO wrappers are best-effort and never fail a launch (a write
//! failure just means the agent shows the prompt, as before).

use std::path::{Path, PathBuf};

/// Pure: given the contents of `~/.claude.json` (empty if absent) and an
/// absolute workspace path, return the JSON to write back, or `None` when no
/// write is needed — the path is already trusted, or the existing file is
/// unparseable (in which case we must never clobber it).
pub fn claude_trusted_json(existing: &str, abs_path: &str) -> Option<String> {
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing).ok()?
    };

    let projects = root
        .as_object_mut()?
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()?;

    let entry = projects
        .entry(abs_path.to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()?;

    if entry
        .get("hasTrustDialogAccepted")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return None;
    }
    entry.insert("hasTrustDialogAccepted".to_string(), true.into());
    serde_json::to_string_pretty(&root).ok()
}

/// Pure: given the contents of `~/.codex/config.toml` (empty if absent) and an
/// absolute path, return the config with a trusted `[projects."<path>"]` table
/// appended, or `None` if a table for that path is already present.
pub fn codex_trust_appended(existing: &str, abs_path: &str) -> Option<String> {
    let header = format!("[projects.\"{abs_path}\"]");
    if existing.contains(&header) {
        return None;
    }
    let mut out = existing.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&header);
    out.push('\n');
    out.push_str("trust_level = \"trusted\"\n");
    Some(out)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Write `contents` to `path` atomically (temp file + rename) so a crash mid-write
/// can never leave a half-written config.
///
/// The temp name is per-process so concurrent daemons don't share it, and the
/// original file's permissions are carried over (these configs are often 0600).
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("kaiju-tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, path)
}

/// Read a config file for a read-modify-write. A missing file is an empty
/// config; any other failure (permissions, bad UTF-8, I/O) yields `None` so the
/// caller skips the write rather than replacing a file it could not read.
fn read_existing(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
        Err(_) => None,
    }
}

/// Serializes the read-modify-write of the trust files across concurrent starts.
static TRUST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Mark `run_dir` as trusted in `~/.claude.json` (best-effort).
pub fn ensure_claude_trust(run_dir: &Path) {
    let (Some(home), Some(abs)) = (home_dir(), run_dir.to_str()) else {
        return;
    };
    let path = home.join(".claude.json");
    let _guard = TRUST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(existing) = read_existing(&path) else {
        return;
    };
    if let Some(updated) = claude_trusted_json(&existing, abs) {
        let _ = write_atomic(&path, &updated);
    }
}

/// Mark `run_dir` as trusted in `~/.codex/config.toml` (best-effort).
pub fn ensure_codex_trust(run_dir: &Path) {
    let (Some(home), Some(abs)) = (home_dir(), run_dir.to_str()) else {
        return;
    };
    let dir = home.join(".codex");
    let path = dir.join("config.toml");
    let _guard = TRUST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(existing) = read_existing(&path) else {
        return;
    };
    if let Some(updated) = codex_trust_appended(&existing, abs) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = write_atomic(&path, &updated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kaiju-trust-{}-{name}", std::process::id()))
    }

    #[test]
    fn read_existing_treats_only_a_missing_file_as_empty() {
        assert_eq!(read_existing(&temp_file("absent")), Some(String::new()));

        // Unreadable content (invalid UTF-8) must not be mistaken for empty,
        // or the caller would overwrite the real config.
        let bad = temp_file("bad-utf8");
        std::fs::write(&bad, [0xff, 0xfe]).unwrap();
        assert_eq!(read_existing(&bad), None);
        let _ = std::fs::remove_file(&bad);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_the_original_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("perms.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_atomic(&path, "{\"a\":1}").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn claude_adds_trust_to_empty_config() {
        let out = claude_trusted_json("", "/tmp/wt").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["projects"]["/tmp/wt"]["hasTrustDialogAccepted"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn claude_preserves_existing_projects_and_keys() {
        let existing = r#"{"numStartups":7,"projects":{"/other":{"hasTrustDialogAccepted":true}}}"#;
        let out = claude_trusted_json(existing, "/tmp/wt").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["numStartups"], serde_json::json!(7));
        assert_eq!(
            v["projects"]["/other"]["hasTrustDialogAccepted"],
            serde_json::json!(true)
        );
        assert_eq!(
            v["projects"]["/tmp/wt"]["hasTrustDialogAccepted"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn claude_returns_none_when_already_trusted() {
        let existing = r#"{"projects":{"/tmp/wt":{"hasTrustDialogAccepted":true}}}"#;
        assert!(claude_trusted_json(existing, "/tmp/wt").is_none());
    }

    #[test]
    fn claude_returns_none_on_unparseable_config() {
        // Must never clobber a config we can't parse.
        assert!(claude_trusted_json("{ not json", "/tmp/wt").is_none());
    }

    #[test]
    fn codex_appends_block_when_absent() {
        let out = codex_trust_appended("model = \"o3\"\n", "/tmp/wt").unwrap();
        assert!(out.starts_with("model = \"o3\""));
        assert!(out.contains("[projects.\"/tmp/wt\"]"));
        assert!(out.contains("trust_level = \"trusted\""));
    }

    #[test]
    fn codex_returns_none_when_already_present() {
        let existing = "[projects.\"/tmp/wt\"]\ntrust_level = \"trusted\"\n";
        assert!(codex_trust_appended(existing, "/tmp/wt").is_none());
    }

    #[test]
    fn codex_handles_empty_file() {
        let out = codex_trust_appended("", "/tmp/wt").unwrap();
        assert!(out.starts_with("[projects.\"/tmp/wt\"]"));
        assert!(out.ends_with("trust_level = \"trusted\"\n"));
    }
}
