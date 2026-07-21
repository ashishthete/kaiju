# Kaiju Service Autostart Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `kaiju service` subcommand that registers `kaiju-daemon` and `kaiju-pet-shell` as macOS launchd user agents so both start at login.

**Architecture:** A new `service` module in `kaiju-cli` split three ways: `plist.rs` generates plist XML from a `ServiceSpec` struct and does no I/O (this is the only part with logic worth testing); `launchctl.rs` shells out to `launchctl`; `mod.rs` holds the error type, binary resolution, spec builders, and the four command orchestrations. Plists are written to `~/Library/LaunchAgents/` and point at installed binaries, never at `target/`.

**Tech Stack:** Rust 2021, `clap` 4 (derive), `thiserror` 1. No new third-party dependencies beyond `thiserror`, which is already a workspace dependency.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-21-service-autostart-design.md`. Read it before starting.
- **macOS only.** Non-macOS targets must still compile and parse the subcommand, returning `ServiceError::NotMacos` at execution time. Do **not** `#[cfg]` the subcommand out — the CLI surface and `--help` stay uniform across platforms.
- **No `unwrap()` / `expect()` / `panic!()`** outside `#[cfg(test)]` code. Project rule from `CLAUDE.md`.
- **Minimize dependencies.** `thiserror` is the only crate added to `crates/kaiju-cli/Cargo.toml`. Do not add `dirs`, `plist`, `libc`, or `serde_plist`. Get `$HOME` via `std::env::var("HOME")` and the uid via the `id -u` command.
- **Agent labels are exactly** `dev.kaiju.daemon` and `dev.kaiju.pet`.
- **Functions under 30 lines preferred, 50 maximum.** Project rule from `CLAUDE.md`.
- **Pre-merge gate:** `make check` (runs `fmt-check`, `lint` with `-D warnings`, and `test`) must pass before the final commit.
- The CLI's existing error convention is `type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>` (`crates/kaiju-cli/src/client.rs:61`). `ServiceError` converts into that via the blanket `impl<E: Error> From<E> for Box<dyn Error>`, so dispatch needs no manual mapping.

---

## File Structure

| File | Responsibility | Status |
|---|---|---|
| `crates/kaiju-cli/src/service/plist.rs` | `ServiceSpec` struct → plist XML `String`. Pure, no I/O. | Create |
| `crates/kaiju-cli/src/service/mod.rs` | `ServiceError`, binary resolution, spec builders, the four orchestrations. | Create |
| `crates/kaiju-cli/src/service/launchctl.rs` | Thin wrappers over `launchctl bootstrap/bootout/kickstart/print`. | Create |
| `crates/kaiju-cli/src/main.rs` | Register `Commands::Service` and dispatch it. | Modify |
| `crates/kaiju-cli/Cargo.toml` | Add `thiserror`. | Modify |
| `Makefile` | `install-service` target. | Modify |
| `README.md` | Document the subcommand. | Modify |

---

### Task 1: Plist generation

The pure core. `ServiceSpec` describes one launchd agent; `to_plist_xml` renders it. Both agents use this same type and differ only in field values.

`environment` is a `Vec<(String, String)>` rather than a `HashMap` so the rendered output is deterministic and therefore assertable in tests.

**Files:**
- Create: `crates/kaiju-cli/src/service/plist.rs`
- Test: same file, `#[cfg(test)] mod tests` (project convention — see `crates/kaiju-pet-core/src/prefs.rs`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct ServiceSpec { pub label: String, pub program: PathBuf, pub run_at_load: bool, pub keep_alive: bool, pub stdout_path: Option<PathBuf>, pub stderr_path: Option<PathBuf>, pub environment: Vec<(String, String)> }`
  - `pub fn to_plist_xml(spec: &ServiceSpec) -> String`

- [ ] **Step 1: Create the module file with the struct and stub, plus the failing tests**

Create `crates/kaiju-cli/src/service/plist.rs`:

```rust
//! Renders a launchd agent description into plist XML.
//!
//! Pure: no filesystem, no process, no launchd. This is the only part of the
//! service module with enough logic to be worth unit-testing on its own.

use std::path::PathBuf;

/// One launchd user agent.
pub struct ServiceSpec {
    pub label: String,
    pub program: PathBuf,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    /// Ordered so the rendered plist is deterministic.
    pub environment: Vec<(String, String)>,
}

pub fn to_plist_xml(_spec: &ServiceSpec) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon_fixture() -> ServiceSpec {
        ServiceSpec {
            label: "dev.kaiju.daemon".to_string(),
            program: PathBuf::from("/Users/x/.cargo/bin/kaiju-daemon"),
            run_at_load: true,
            keep_alive: true,
            stdout_path: Some(PathBuf::from("/Users/x/.kaiju/logs/daemon.out.log")),
            stderr_path: Some(PathBuf::from("/Users/x/.kaiju/logs/daemon.err.log")),
            environment: vec![("PATH".to_string(), "/opt/homebrew/bin:/usr/bin".to_string())],
        }
    }

    fn pet_fixture() -> ServiceSpec {
        ServiceSpec {
            label: "dev.kaiju.pet".to_string(),
            program: PathBuf::from("/Users/x/.cargo/bin/kaiju-pet-shell"),
            run_at_load: true,
            keep_alive: false,
            stdout_path: None,
            stderr_path: None,
            environment: vec![],
        }
    }

    #[test]
    fn plist_has_xml_header_and_root() {
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<!DOCTYPE plist PUBLIC"));
        assert!(xml.contains("<plist version=\"1.0\">"));
        assert!(xml.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn plist_contains_label_and_program() {
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.contains("<key>Label</key>\n\t<string>dev.kaiju.daemon</string>"));
        assert!(xml.contains("<key>Program</key>\n\t<string>/Users/x/.cargo/bin/kaiju-daemon</string>"));
    }

    #[test]
    fn daemon_keeps_alive_and_pet_does_not() {
        let daemon = to_plist_xml(&daemon_fixture());
        assert!(daemon.contains("<key>KeepAlive</key>\n\t<true/>"));

        let pet = to_plist_xml(&pet_fixture());
        assert!(pet.contains("<key>KeepAlive</key>\n\t<false/>"));
    }

    #[test]
    fn both_run_at_load() {
        assert!(to_plist_xml(&daemon_fixture()).contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(to_plist_xml(&pet_fixture()).contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn path_environment_is_embedded() {
        // The whole point of the feature: launchd's default PATH omits
        // /opt/homebrew/bin, so the daemon could not find tmux or the agent CLIs.
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.contains("<key>EnvironmentVariables</key>"));
        assert!(xml.contains("<key>PATH</key>\n\t\t<string>/opt/homebrew/bin:/usr/bin</string>"));
    }

    #[test]
    fn empty_environment_omits_the_dict_entirely() {
        let xml = to_plist_xml(&pet_fixture());
        assert!(!xml.contains("EnvironmentVariables"));
    }

    #[test]
    fn log_paths_present_for_daemon_absent_for_pet() {
        let daemon = to_plist_xml(&daemon_fixture());
        assert!(daemon.contains("<key>StandardOutPath</key>"));
        assert!(daemon.contains("/Users/x/.kaiju/logs/daemon.out.log"));
        assert!(daemon.contains("<key>StandardErrorPath</key>"));
        assert!(daemon.contains("/Users/x/.kaiju/logs/daemon.err.log"));

        let pet = to_plist_xml(&pet_fixture());
        assert!(!pet.contains("StandardOutPath"));
        assert!(!pet.contains("StandardErrorPath"));
    }

    #[test]
    fn special_characters_are_escaped() {
        let spec = ServiceSpec {
            label: "dev.kaiju.daemon".to_string(),
            program: PathBuf::from("/Users/a & b/bin/<kaiju>"),
            run_at_load: true,
            keep_alive: false,
            stdout_path: None,
            stderr_path: None,
            environment: vec![],
        };
        let xml = to_plist_xml(&spec);
        assert!(xml.contains("/Users/a &amp; b/bin/&lt;kaiju&gt;"));
        assert!(!xml.contains("<kaiju>"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kaiju-cli plist`

Expected: compile error — `service` is not yet declared as a module in `main.rs`, so the file is not compiled at all. Add `mod service;` to `crates/kaiju-cli/src/main.rs` (line 1, above `mod client;`) and create `crates/kaiju-cli/src/service/mod.rs` containing only `pub mod plist;` for now. Re-run; the tests should now compile and FAIL on assertions (`to_plist_xml` returns an empty string).

- [ ] **Step 3: Implement the renderer**

Replace the `to_plist_xml` stub in `crates/kaiju-cli/src/service/plist.rs`:

```rust
pub fn to_plist_xml(spec: &ServiceSpec) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );

    push_string(&mut out, "Label", &spec.label);
    push_string(&mut out, "Program", &spec.program.display().to_string());
    push_bool(&mut out, "RunAtLoad", spec.run_at_load);
    push_bool(&mut out, "KeepAlive", spec.keep_alive);

    if let Some(path) = &spec.stdout_path {
        push_string(&mut out, "StandardOutPath", &path.display().to_string());
    }
    if let Some(path) = &spec.stderr_path {
        push_string(&mut out, "StandardErrorPath", &path.display().to_string());
    }

    if !spec.environment.is_empty() {
        out.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (key, value) in &spec.environment {
            out.push_str(&format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml_escape(key),
                xml_escape(value)
            ));
        }
        out.push_str("\t</dict>\n");
    }

    out.push_str("</dict>\n</plist>\n");
    out
}

fn push_string(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!(
        "\t<key>{}</key>\n\t<string>{}</string>\n",
        xml_escape(key),
        xml_escape(value)
    ));
}

fn push_bool(out: &mut String, key: &str, value: bool) {
    let tag = if value { "<true/>" } else { "<false/>" };
    out.push_str(&format!("\t<key>{}</key>\n\t{}\n", xml_escape(key), tag));
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
```

Note: `&` must be replaced first, otherwise the `&` in `&lt;` gets double-escaped.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kaiju-cli plist`

Expected: PASS — 8 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-cli/src/service/plist.rs crates/kaiju-cli/src/service/mod.rs crates/kaiju-cli/src/main.rs
git commit -m "feat(cli): render launchd agent plists from a ServiceSpec"
```

---

### Task 2: Error type, paths, and spec builders

Everything `install` needs before it can touch launchd: the error enum, binary resolution against a search path, and the two `ServiceSpec` builders.

`resolve_in` takes its search roots as an argument rather than reading `$HOME` itself. That is what makes it testable against a tempdir.

**Files:**
- Modify: `crates/kaiju-cli/src/service/mod.rs`
- Modify: `crates/kaiju-cli/Cargo.toml`
- Test: `crates/kaiju-cli/src/service/mod.rs`, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `plist::ServiceSpec` from Task 1.
- Produces:
  - `pub enum ServiceError` with variants `BinaryNotFound { name: String, searched: Vec<PathBuf>, hint: String }`, `LaunchctlFailed { command: String, code: i32, stderr: String }`, `PlistWrite { path: PathBuf, source: std::io::Error }`, `LogDirCreate { path: PathBuf, source: std::io::Error }`, `NoHomeDir`, `NotMacos`
  - `pub const DAEMON_LABEL: &str = "dev.kaiju.daemon";`
  - `pub const PET_LABEL: &str = "dev.kaiju.pet";`
  - `fn resolve_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf>`
  - `fn bin_search_dirs(home: &Path) -> Vec<PathBuf>`
  - `fn home_dir() -> Result<PathBuf, ServiceError>`
  - `fn plist_path(home: &Path, label: &str) -> PathBuf`
  - `fn daemon_spec(home: &Path) -> Result<ServiceSpec, ServiceError>`
  - `fn pet_spec(home: &Path) -> Result<ServiceSpec, ServiceError>`

- [ ] **Step 1: Add the `thiserror` dependency**

In `crates/kaiju-cli/Cargo.toml`, add to the `[dependencies]` block (after `tokio`):

```toml
thiserror = { workspace = true }
```

`thiserror = "1"` is already declared in the root `Cargo.toml` `[workspace.dependencies]`, so no version goes here.

- [ ] **Step 2: Write the failing tests**

Replace the contents of `crates/kaiju-cli/src/service/mod.rs`:

```rust
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

fn to_stub() {}

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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p kaiju-cli service::tests`

Expected: FAIL to compile — `resolve_in`, `bin_search_dirs`, `plist_path`, `daemon_spec`, and `pet_spec` are not defined.

- [ ] **Step 4: Implement**

In `crates/kaiju-cli/src/service/mod.rs`, delete the `fn to_stub() {}` placeholder and add above the `#[cfg(test)]` block:

```rust
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
    home.join("Library/LaunchAgents").join(format!("{label}.plist"))
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
    let mut env = vec![("PATH".to_string(), std::env::var("PATH").unwrap_or_default())];
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kaiju-cli service::tests`

Expected: PASS — 9 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-cli/Cargo.toml crates/kaiju-cli/src/service/mod.rs
git commit -m "feat(cli): resolve service binaries and build daemon/pet launchd specs"
```

---

### Task 3: The launchctl wrapper

A thin shell over the four `launchctl` verbs. No branching logic beyond error mapping, so it carries no unit tests of its own — it is covered by the manual verification in Task 6.

**Files:**
- Create: `crates/kaiju-cli/src/service/launchctl.rs`
- Modify: `crates/kaiju-cli/src/service/mod.rs` (add `mod launchctl;`)

**Interfaces:**
- Consumes: `ServiceError` from Task 2.
- Produces:
  - `pub fn bootstrap(uid: &str, plist: &Path) -> Result<(), ServiceError>`
  - `pub fn bootout(uid: &str, label: &str) -> Result<(), ServiceError>`
  - `pub fn kickstart(uid: &str, label: &str) -> Result<(), ServiceError>`
  - `pub fn print(uid: &str, label: &str) -> Option<String>` — `None` when the label is not loaded
  - `pub fn current_uid() -> Result<String, ServiceError>`
  - `pub fn pid_from_print(output: &str) -> Option<u32>`

- [ ] **Step 1: Add the module declaration**

In `crates/kaiju-cli/src/service/mod.rs`, below `pub mod plist;`:

```rust
mod launchctl;
```

- [ ] **Step 2: Write the failing test for the one piece with real logic**

Create `crates/kaiju-cli/src/service/launchctl.rs` with the tests and stubs:

```rust
//! Thin wrappers over the `launchctl` verbs used to manage kaiju's agents.
//!
//! Deliberately logic-free: everything worth asserting lives in `plist.rs` and
//! `mod.rs`. Only the pid parser is unit-tested here.

use super::ServiceError;
use std::path::Path;
use std::process::Command;

pub fn pid_from_print(_output: &str) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real `launchctl print gui/501/dev.kaiju.daemon` output.
    const RUNNING: &str = "\
dev.kaiju.daemon = {
\tactive count = 1
\tpath = /Users/x/Library/LaunchAgents/dev.kaiju.daemon.plist
\tstate = running
\tpid = 4821
\tprogram = /Users/x/.cargo/bin/kaiju-daemon
}";

    const LOADED_NOT_RUNNING: &str = "\
dev.kaiju.pet = {
\tactive count = 0
\tstate = not running
}";

    #[test]
    fn pid_is_parsed_from_print_output() {
        assert_eq!(pid_from_print(RUNNING), Some(4821));
    }

    #[test]
    fn no_pid_when_loaded_but_not_running() {
        assert_eq!(pid_from_print(LOADED_NOT_RUNNING), None);
    }

    #[test]
    fn no_pid_from_empty_output() {
        assert_eq!(pid_from_print(""), None);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p kaiju-cli launchctl`

Expected: FAIL — `pid_is_parsed_from_print_output` asserts `Some(4821)`, gets `None`.

- [ ] **Step 4: Implement the wrapper**

Replace the `pid_from_print` stub in `crates/kaiju-cli/src/service/launchctl.rs` and add the rest:

```rust
/// `launchctl print` reports the pid on its own indented `pid = N` line.
pub fn pid_from_print(output: &str) -> Option<u32> {
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("pid = ") {
            return value.trim().parse().ok();
        }
    }
    None
}

pub fn current_uid() -> Result<String, ServiceError> {
    let output = run(&["id"], &["-u"])?;
    Ok(output.trim().to_string())
}

pub fn bootstrap(uid: &str, plist: &Path) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}");
    let path = plist.display().to_string();
    run(&["launchctl"], &["bootstrap", &target, &path])?;
    Ok(())
}

pub fn bootout(uid: &str, label: &str) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}/{label}");
    run(&["launchctl"], &["bootout", &target])?;
    Ok(())
}

pub fn kickstart(uid: &str, label: &str) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}/{label}");
    run(&["launchctl"], &["kickstart", "-k", &target])?;
    Ok(())
}

/// `None` when the label is not loaded — launchctl exits non-zero for that,
/// which is a normal state here rather than a failure.
pub fn print(uid: &str, label: &str) -> Option<String> {
    let target = format!("gui/{uid}/{label}");
    let output = Command::new("launchctl").args(["print", &target]).output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn run(program: &[&str], args: &[&str]) -> Result<String, ServiceError> {
    let bin = program.first().copied().unwrap_or_default();
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| ServiceError::LaunchctlFailed {
            command: format!("{bin} {}", args.join(" ")),
            code: -1,
            stderr: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(ServiceError::LaunchctlFailed {
            command: format!("{bin} {}", args.join(" ")),
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kaiju-cli launchctl`

Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-cli/src/service/launchctl.rs crates/kaiju-cli/src/service/mod.rs
git commit -m "feat(cli): wrap the launchctl verbs used to manage kaiju agents"
```

---

### Task 4: `install` and `uninstall`, wired into the CLI

The core deliverable. After this task, `kaiju service install` makes both agents start at login.

`install` is idempotent: it boots out any existing label before bootstrapping, so a re-run upgrades cleanly instead of erroring on an already-loaded label. It does **not** build — it verifies the binaries exist and errors with the exact `cargo install` command otherwise.

Partial-failure policy: if the daemon registers and the pet then fails, report the failure and leave the daemon registered. Do not roll back. A half-installed service the user is told about beats a silent revert, and `uninstall` cleans up either state.

**Files:**
- Modify: `crates/kaiju-cli/src/service/mod.rs`
- Modify: `crates/kaiju-cli/src/main.rs`

**Interfaces:**
- Consumes: `daemon_spec`, `pet_spec`, `plist_path`, `home_dir`, `log_dir`, `ServiceError` (Task 2); `plist::to_plist_xml` (Task 1); `launchctl::{bootstrap, bootout, current_uid}` (Task 3).
- Produces:
  - `pub fn install() -> Result<(), ServiceError>`
  - `pub fn uninstall() -> Result<(), ServiceError>`

- [ ] **Step 1: Implement `install` and `uninstall`**

In `crates/kaiju-cli/src/service/mod.rs`, add above the `#[cfg(test)]` block:

```rust
fn require_macos() -> Result<(), ServiceError> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(ServiceError::NotMacos)
    }
}

/// Write the plist and (re)load it. Bootout first so a re-run upgrades in
/// place rather than failing on an already-loaded label.
fn install_one(uid: &str, home: &Path, spec: &ServiceSpec) -> Result<(), ServiceError> {
    let path = plist_path(home, &spec.label);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ServiceError::PlistWrite {
            path: path.clone(),
            source,
        })?;
    }

    std::fs::write(&path, plist::to_plist_xml(spec)).map_err(|source| {
        ServiceError::PlistWrite {
            path: path.clone(),
            source,
        }
    })?;

    // Not loaded yet is the normal case on a first install, so ignore this.
    let _ = launchctl::bootout(uid, &spec.label);
    launchctl::bootstrap(uid, &path)?;

    println!("installed {} -> {}", spec.label, spec.program.display());
    Ok(())
}

pub fn install() -> Result<(), ServiceError> {
    require_macos()?;
    let home = home_dir()?;
    let uid = launchctl::current_uid()?;

    // Resolve both binaries before touching launchd, so a missing pet binary
    // does not leave a half-registered daemon.
    let daemon = daemon_spec(&home)?;
    let pet = pet_spec(&home)?;

    let logs = log_dir(&home);
    std::fs::create_dir_all(&logs).map_err(|source| ServiceError::LogDirCreate {
        path: logs.clone(),
        source,
    })?;

    install_one(&uid, &home, &daemon)?;
    install_one(&uid, &home, &pet)?;

    println!("daemon logs: {}", logs.join("daemon.out.log").display());
    Ok(())
}

fn uninstall_one(uid: &str, home: &Path, label: &str) -> Result<(), ServiceError> {
    // Both steps tolerate absence: uninstall must succeed from any state.
    let _ = launchctl::bootout(uid, label);
    let path = plist_path(home, label);
    let _ = std::fs::remove_file(&path);
    println!("uninstalled {label}");
    Ok(())
}

pub fn uninstall() -> Result<(), ServiceError> {
    require_macos()?;
    let home = home_dir()?;
    let uid = launchctl::current_uid()?;
    uninstall_one(&uid, &home, DAEMON_LABEL)?;
    uninstall_one(&uid, &home, PET_LABEL)?;
    Ok(())
}
```

- [ ] **Step 2: Add the subcommand to the CLI**

In `crates/kaiju-cli/src/main.rs`, add to the `Commands` enum immediately after the `Pet` variant (around line 159):

```rust
    /// Manage the launchd agents that start kaiju at login (macOS)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
```

Then add this enum directly below the `Commands` enum's closing brace:

```rust
#[derive(Subcommand)]
enum ServiceAction {
    /// Register the daemon and pet to start at login
    Install,
    /// Remove both launchd agents
    Uninstall,
    /// Show whether each agent is running
    Status,
    /// Restart an agent (both, if unspecified)
    Restart {
        /// Which agent: daemon or pet
        target: Option<String>,
    },
}
```

- [ ] **Step 3: Dispatch it**

In `crates/kaiju-cli/src/main.rs`, add to the `match cli.command` block immediately after the `Commands::Pet => { ... }` arm:

```rust
        Commands::Service { action } => match action {
            ServiceAction::Install => service::install().map_err(Into::into),
            ServiceAction::Uninstall => service::uninstall().map_err(Into::into),
            ServiceAction::Status => Err("not implemented".into()),
            ServiceAction::Restart { .. } => Err("not implemented".into()),
        },
```

`Status` and `Restart` land in Task 5.

- [ ] **Step 4: Verify it compiles and the existing tests still pass**

Run: `cargo test -p kaiju-cli && cargo run -p kaiju-cli -- service --help`

Expected: tests PASS, and the help output lists `install`, `uninstall`, `status`, `restart`.

- [ ] **Step 5: Verify install end to end**

Run:

```bash
cargo install --path crates/kaiju-daemon
cargo install --path crates/kaiju-pet-shell
cargo run -p kaiju-cli -- service install
launchctl print gui/$(id -u)/dev.kaiju.daemon | head -20
curl -s http://127.0.0.1:7800/health
```

Expected: `install` prints two `installed ...` lines; `launchctl print` shows `state = running` with a pid; `curl` returns a healthy response. Confirm the baked `PATH` is right:

```bash
grep -A2 '<key>PATH</key>' ~/Library/LaunchAgents/dev.kaiju.daemon.plist
```

Expected: your interactive `PATH`, including `/opt/homebrew/bin`.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-cli/src/service/mod.rs crates/kaiju-cli/src/main.rs
git commit -m "feat(cli): add kaiju service install/uninstall for login autostart"
```

---

### Task 5: `status` and `restart`

**Files:**
- Modify: `crates/kaiju-cli/src/service/mod.rs`
- Modify: `crates/kaiju-cli/src/main.rs`

**Interfaces:**
- Consumes: `launchctl::{print, pid_from_print, kickstart, current_uid}` (Task 3); `DAEMON_LABEL`, `PET_LABEL`, `plist_path`, `home_dir` (Task 2).
- Produces:
  - `pub fn status() -> Result<(), ServiceError>`
  - `pub fn restart(target: Option<&str>) -> Result<(), ServiceError>`

- [ ] **Step 1: Write the failing test for label selection**

Add to the `#[cfg(test)] mod tests` block in `crates/kaiju-cli/src/service/mod.rs`:

```rust
    #[test]
    fn restart_target_selects_the_right_labels() {
        assert_eq!(labels_for(None), vec![DAEMON_LABEL, PET_LABEL]);
        assert_eq!(labels_for(Some("daemon")), vec![DAEMON_LABEL]);
        assert_eq!(labels_for(Some("pet")), vec![PET_LABEL]);
        assert_eq!(labels_for(Some("nonsense")), Vec::<&str>::new());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p kaiju-cli restart_target`

Expected: FAIL to compile — `labels_for` is not defined.

- [ ] **Step 3: Implement**

In `crates/kaiju-cli/src/service/mod.rs`, add above the `#[cfg(test)]` block:

```rust
/// Empty result means the caller passed an unrecognised target.
fn labels_for(target: Option<&str>) -> Vec<&'static str> {
    match target {
        None => vec![DAEMON_LABEL, PET_LABEL],
        Some("daemon") => vec![DAEMON_LABEL],
        Some("pet") => vec![PET_LABEL],
        Some(_) => vec![],
    }
}

fn describe(uid: &str, home: &Path, label: &str) -> String {
    match launchctl::print(uid, label) {
        Some(output) => match launchctl::pid_from_print(&output) {
            Some(pid) => format!("running  pid {pid}"),
            None => "stopped".to_string(),
        },
        None if plist_path(home, label).exists() => "stopped".to_string(),
        None => "not installed".to_string(),
    }
}

pub fn status() -> Result<(), ServiceError> {
    require_macos()?;
    let home = home_dir()?;
    let uid = launchctl::current_uid()?;
    println!("daemon  {}", describe(&uid, &home, DAEMON_LABEL));
    println!("pet     {}", describe(&uid, &home, PET_LABEL));
    Ok(())
}

pub fn restart(target: Option<&str>) -> Result<(), ServiceError> {
    require_macos()?;
    let labels = labels_for(target);
    if labels.is_empty() {
        return Err(ServiceError::UnknownTarget {
            target: target.unwrap_or_default().to_string(),
        });
    }

    let uid = launchctl::current_uid()?;
    for label in labels {
        launchctl::kickstart(&uid, label)?;
        println!("restarted {label}");
    }
    Ok(())
}
```

Add the new variant to `ServiceError`:

```rust
    #[error("unknown service `{target}` — expected `daemon` or `pet`")]
    UnknownTarget { target: String },
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kaiju-cli`

Expected: PASS — all tests across `plist`, `service::tests`, and `launchctl`.

- [ ] **Step 5: Replace the placeholder dispatch arms**

In `crates/kaiju-cli/src/main.rs`, replace the two `Err("not implemented".into())` arms added in Task 4:

```rust
            ServiceAction::Status => service::status().map_err(Into::into),
            ServiceAction::Restart { target } => {
                service::restart(target.as_deref()).map_err(Into::into)
            }
```

- [ ] **Step 6: Verify against the live agents**

Run:

```bash
cargo run -p kaiju-cli -- service status
cargo run -p kaiju-cli -- service restart pet
cargo run -p kaiju-cli -- service restart nonsense
```

Expected: `status` prints `daemon  running  pid N` and `pet  running  pid N`; `restart pet` prints `restarted dev.kaiju.pet` and the pet window reappears; `restart nonsense` prints `error: unknown service `nonsense` — expected `daemon` or `pet`` and exits 1.

- [ ] **Step 7: Commit**

```bash
git add crates/kaiju-cli/src/service/mod.rs crates/kaiju-cli/src/main.rs
git commit -m "feat(cli): add kaiju service status and restart"
```

---

### Task 6: Docs, make target, and the login verification

The one test that actually proves the feature: log out and back in.

**Files:**
- Modify: `Makefile`
- Modify: `README.md`

- [ ] **Step 1: Add the make target**

In `Makefile`, add to the `.PHONY` line (line 5) the name `install-service`, then add after the existing `install` target (line 39):

```make
install-service: ## Install daemon + pet and register them to start at login (macOS)
	cargo install --path crates/kaiju-daemon
	cargo install --path crates/kaiju-pet-shell
	cargo install --path crates/kaiju-cli
	kaiju service install
```

Unlike `kaiju service install` itself, the make target *does* build — that is the point of a convenience target, and `make` output makes the wait legible.

- [ ] **Step 2: Document it in the README**

In `README.md`, add a section after the install instructions (near line 53):

````markdown
### Start at login (macOS)

```bash
make install-service
```

This installs the daemon, pet, and CLI, then registers two launchd user agents
in `~/Library/LaunchAgents/`:

| Label | Binary | Behaviour |
|---|---|---|
| `dev.kaiju.daemon` | `kaiju-daemon` | Starts at login, restarts if it crashes. Logs to `~/.kaiju/logs/daemon.{out,err}.log`. |
| `dev.kaiju.pet` | `kaiju-pet-shell` | Starts at login. If you quit it, it stays quit until next login or `kaiju service restart pet`. |

```bash
kaiju service status              # daemon  running  pid 4821
kaiju service restart [daemon|pet]
kaiju service uninstall
```

Your shell's `PATH` is captured at install time and written into the plists.
The daemon spawns `tmux` and the agent CLIs by name, and launchd would
otherwise give it a minimal `PATH` that omits `/opt/homebrew/bin`. Re-run
`kaiju service install` after changing your `PATH` or reinstalling the binaries.
````

- [ ] **Step 3: Run the full gate**

Run: `make check`

Expected: `fmt-check`, `clippy -D warnings`, and all tests PASS. Fix anything clippy flags.

- [ ] **Step 4: The login test**

This is the one verification that cannot be automated.

```bash
kaiju service uninstall
make install-service
kaiju service status          # both running
```

Log out and log back in. Then:

```bash
kaiju service status          # both running again
curl -s http://127.0.0.1:7800/health
kaiju here "say hello"        # proves PATH reached tmux and the agent CLI
tail -5 ~/.kaiju/logs/daemon.err.log
```

Expected: both agents running after login; health OK; the agent actually spawns (this is the check that the `PATH` snapshot worked — a broken `PATH` shows up here and nowhere earlier); no errors in the log tail.

Finally confirm uninstall is clean:

```bash
kaiju service uninstall
ls ~/Library/LaunchAgents/ | grep kaiju    # no output
kaiju service status                        # both "not installed"
```

- [ ] **Step 5: Commit**

```bash
git add Makefile README.md
git commit -m "docs: document kaiju service autostart and add install-service target"
```

---

## Self-Review Notes

Checked against `docs/superpowers/specs/2026-07-21-service-autostart-design.md`:

- Every spec decision maps to a task: launchd mechanism (T1, T3), installed-binary resolution with `~/.cargo/bin` then `~/.local/bin` (T2), pet-shell only (T2), `kaiju service` surface (T4, T5), pet `KeepAlive` off (T2), `PATH` snapshot (T2, verified T4/T6), daemon log redirection (T2), idempotent install (T4), partial-failure policy (T4 — implemented as resolve-both-before-touching-launchd, which is strictly better than the spec's report-and-leave since it prevents the half-registered state arising from a missing binary), `install` does not build (T4), all six error variants (T2) plus `UnknownTarget` (T5, an addition the spec did not anticipate), `NotMacos` at runtime not `#[cfg]` (T2, T4).
- Spec's out-of-scope items are absent from the plan: no `.app` bundle, no `SMAppService`, no repointing of `kaiju pet`, no systemd, no daemon lifecycle CLI.
- Naming is consistent across tasks: `resolve_in`, `bin_search_dirs`, `plist_path`, `daemon_spec`, `pet_spec`, `install_one`, `uninstall_one`, `labels_for`, `describe`, `pid_from_print`, `current_uid`.
- No placeholders. Task 4 Step 3 introduces two deliberate `not implemented` arms, replaced in Task 5 Step 5, so each task compiles and commits green on its own.
