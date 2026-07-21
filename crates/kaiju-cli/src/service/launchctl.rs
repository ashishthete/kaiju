//! Thin wrappers over the `launchctl` verbs used to manage kaiju's agents.
//!
//! Deliberately logic-free: everything worth asserting lives in `plist.rs` and
//! `mod.rs`. Only the pid parser is unit-tested here.

use super::ServiceError;
use std::path::Path;
use std::process::Command;

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
    let output = run("id", &["-u"])?;
    Ok(output.trim().to_string())
}

pub fn bootstrap(uid: &str, plist: &Path) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}");
    let path = plist.display().to_string();
    run("launchctl", &["bootstrap", &target, &path])?;
    Ok(())
}

pub fn bootout(uid: &str, label: &str) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}/{label}");
    run("launchctl", &["bootout", &target])?;
    Ok(())
}

pub fn kickstart(uid: &str, label: &str) -> Result<(), ServiceError> {
    let target = format!("gui/{uid}/{label}");
    run("launchctl", &["kickstart", "-k", &target])?;
    Ok(())
}

/// `None` when the label is not loaded — launchctl exits non-zero for that,
/// which is a normal state here rather than a failure.
pub fn print(uid: &str, label: &str) -> Option<String> {
    let target = format!("gui/{uid}/{label}");
    let output = Command::new("launchctl")
        .args(["print", &target])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn run(bin: &str, args: &[&str]) -> Result<String, ServiceError> {
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
