//! Registers the kaiju daemon and pet as macOS launchd user agents.

pub mod plist;

use plist::ServiceSpec;
use std::path::{Path, PathBuf};

pub const DAEMON_LABEL: &str = "dev.kaiju.daemon";
pub const PET_LABEL: &str = "dev.kaiju.pet";

const DAEMON_BIN: &str = "kaiju-daemon";
const PET_BIN: &str = "kaiju-pet-shell";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{name} not found (looked in {}) — {hint}", .searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    BinaryNotFound {
        name: String,
        searched: Vec<PathBuf>,
        hint: String,
    },

    #[error("`{command}` failed with exit code {code}: {stderr}")]
    LaunchctlFailed {
        command: String,
        code: i32,
        stderr: String,
    },

    #[error("could not write {path}: {source}")]
    PlistWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not create log directory {path}: {source}")]
    LogDirCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("HOME is not set")]
    NoHomeDir,

    #[error("`kaiju service` uses launchd and is macOS-only")]
    NotMacos,
}

/// Where installed kaiju binaries are looked for, in priority order.
/// `cargo install --path` writes to ~/.cargo/bin by default.
fn bin_search_dirs(home: &Path) -> Vec<PathBuf> {
    vec![home.join(".cargo/bin"), home.join(".local/bin")]
}

/// First `dir/name` that exists, searched in order.
fn resolve_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn home_dir() -> Result<PathBuf, ServiceError> {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => Ok(PathBuf::from(home)),
        _ => Err(ServiceError::NoHomeDir),
    }
}

fn plist_path(home: &Path, label: &str) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
}

fn resolve_bin(home: &Path, name: &str, crate_dir: &str) -> Result<PathBuf, ServiceError> {
    let dirs = bin_search_dirs(home);
    resolve_in(&dirs, name).ok_or_else(|| ServiceError::BinaryNotFound {
        name: name.to_string(),
        searched: dirs,
        hint: format!("install it with `cargo install --path crates/{crate_dir}`"),
    })
}

/// The launchd agent environment.
///
/// launchd hands user agents a minimal PATH of /usr/bin:/bin:/usr/sbin:/sbin,
/// which omits /opt/homebrew/bin. The daemon spawns `tmux`, `sh`, and the agent
/// CLIs by bare name, so without this it would boot cleanly and then fail every
/// agent spawn. Snapshot the installing shell's PATH — it is known to work.
fn agent_environment() -> Vec<(String, String)> {
    let mut env = vec![(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_default(),
    )];
    for key in ["KAIJU_PORT", "KAIJU_HOST"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                env.push((key.to_string(), value));
            }
        }
    }
    env
}

fn log_dir(home: &Path) -> PathBuf {
    home.join(".kaiju/logs")
}

fn daemon_spec(home: &Path) -> Result<ServiceSpec, ServiceError> {
    let program = resolve_bin(home, DAEMON_BIN, "kaiju-daemon")?;
    let logs = log_dir(home);
    Ok(ServiceSpec {
        label: DAEMON_LABEL.to_string(),
        program,
        run_at_load: true,
        // The daemon is a background service: bring it back if it dies.
        keep_alive: true,
        stdout_path: Some(logs.join("daemon.out.log")),
        stderr_path: Some(logs.join("daemon.err.log")),
        environment: agent_environment(),
    })
}

fn pet_spec(home: &Path) -> Result<ServiceSpec, ServiceError> {
    let program = resolve_bin(home, PET_BIN, "kaiju-pet-shell")?;
    Ok(ServiceSpec {
        label: PET_LABEL.to_string(),
        program,
        run_at_load: true,
        // Quitting a desktop pet should mean quitting. Also avoids a respawn
        // loop racing the pet's own singleton guard on port 52789.
        keep_alive: false,
        stdout_path: None,
        stderr_path: None,
        environment: agent_environment(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kaiju-service-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    #[test]
    fn resolve_in_finds_binary_in_first_matching_dir() {
        let root = tempdir("resolve-first");
        let a = root.join("a");
        let b = root.join("b");
        fs::create_dir_all(&a).expect("mkdir a");
        fs::create_dir_all(&b).expect("mkdir b");
        fs::write(a.join("kaiju-daemon"), b"x").expect("write a");
        fs::write(b.join("kaiju-daemon"), b"x").expect("write b");

        let found = resolve_in(&[a.clone(), b], "kaiju-daemon");
        assert_eq!(found, Some(a.join("kaiju-daemon")));
    }

    #[test]
    fn resolve_in_falls_through_to_later_dir() {
        let root = tempdir("resolve-fallthrough");
        let a = root.join("a");
        let b = root.join("b");
        fs::create_dir_all(&a).expect("mkdir a");
        fs::create_dir_all(&b).expect("mkdir b");
        fs::write(b.join("kaiju-daemon"), b"x").expect("write b");

        let found = resolve_in(&[a, b.clone()], "kaiju-daemon");
        assert_eq!(found, Some(b.join("kaiju-daemon")));
    }

    #[test]
    fn resolve_in_returns_none_when_absent() {
        let root = tempdir("resolve-absent");
        assert_eq!(resolve_in(&[root], "kaiju-daemon"), None);
    }

    #[test]
    fn bin_search_dirs_prefers_cargo_bin_over_local_bin() {
        let dirs = bin_search_dirs(Path::new("/Users/x"));
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/Users/x/.cargo/bin"),
                PathBuf::from("/Users/x/.local/bin"),
            ]
        );
    }

    #[test]
    fn plist_path_lands_in_launch_agents() {
        assert_eq!(
            plist_path(Path::new("/Users/x"), DAEMON_LABEL),
            PathBuf::from("/Users/x/Library/LaunchAgents/dev.kaiju.daemon.plist")
        );
    }

    #[test]
    fn daemon_spec_errors_with_install_hint_when_binary_missing() {
        let home = tempdir("daemon-spec-missing");
        let err = daemon_spec(&home).expect_err("should not resolve");
        match err {
            ServiceError::BinaryNotFound { name, hint, .. } => {
                assert_eq!(name, "kaiju-daemon");
                assert!(hint.contains("cargo install --path crates/kaiju-daemon"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn daemon_spec_keeps_alive_and_logs_to_kaiju_logs() {
        let home = tempdir("daemon-spec-ok");
        let bin = home.join(".cargo/bin");
        fs::create_dir_all(&bin).expect("mkdir bin");
        fs::write(bin.join("kaiju-daemon"), b"x").expect("write bin");

        let spec = daemon_spec(&home).expect("resolves");
        assert_eq!(spec.label, DAEMON_LABEL);
        assert_eq!(spec.program, bin.join("kaiju-daemon"));
        assert!(spec.run_at_load);
        assert!(spec.keep_alive);
        assert_eq!(
            spec.stdout_path,
            Some(home.join(".kaiju/logs/daemon.out.log"))
        );
        assert!(spec.environment.iter().any(|(k, _)| k == "PATH"));
    }

    #[test]
    fn pet_spec_does_not_keep_alive_and_has_no_logs() {
        let home = tempdir("pet-spec-ok");
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).expect("mkdir bin");
        fs::write(bin.join("kaiju-pet-shell"), b"x").expect("write bin");

        let spec = pet_spec(&home).expect("resolves");
        assert_eq!(spec.label, PET_LABEL);
        assert!(spec.run_at_load);
        assert!(!spec.keep_alive);
        assert_eq!(spec.stdout_path, None);
        assert_eq!(spec.stderr_path, None);
    }

    #[test]
    fn specs_carry_the_current_path_so_the_daemon_can_find_tmux() {
        let home = tempdir("path-snapshot");
        let bin = home.join(".cargo/bin");
        fs::create_dir_all(&bin).expect("mkdir bin");
        fs::write(bin.join("kaiju-daemon"), b"x").expect("write bin");

        let spec = daemon_spec(&home).expect("resolves");
        let (_, path) = spec
            .environment
            .iter()
            .find(|(k, _)| k == "PATH")
            .expect("PATH present");
        assert_eq!(*path, std::env::var("PATH").unwrap_or_default());
    }
}
