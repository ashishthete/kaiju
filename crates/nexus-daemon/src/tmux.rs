use nexus_core::{NexusError, Result};
use std::process::Command;

/// Manages tmux sessions for agent processes.
pub struct TmuxManager;

impl TmuxManager {
    /// Create a detached tmux session that runs `command` (via `sh -c`) as its
    /// main process.
    ///
    /// Because the agent is the session's process, the session ends when the
    /// agent exits — a clean "completed" signal. Input can still be delivered
    /// with `send_keys` while it runs, and `capture_pane` reads its output.
    pub fn create_session(session_name: &str, working_dir: &str, command: &str) -> Result<()> {
        let output = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                session_name,
                "-c",
                working_dir,
                "sh",
                "-c",
                command,
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to create session '{session_name}': {stderr}"
            )));
        }

        Ok(())
    }

    /// Send a command to a tmux session (simulates typing + Enter).
    pub fn send_keys(session_name: &str, command: &str) -> Result<()> {
        let output = Command::new("tmux")
            .args(["send-keys", "-t", session_name, command, "Enter"])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to send keys to '{session_name}': {stderr}"
            )));
        }

        Ok(())
    }

    /// Capture the current pane output from a tmux session.
    ///
    /// Returns the last `lines` lines of output.
    pub fn capture_pane(session_name: &str, lines: u32) -> Result<String> {
        let start = -(lines as i64);
        let output = Command::new("tmux")
            .args([
                "capture-pane",
                "-t",
                session_name,
                "-p", // print to stdout
                "-S",
                &start.to_string(),
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to capture pane '{session_name}': {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Kill a tmux session.
    pub fn kill_session(session_name: &str) -> Result<()> {
        let output = Command::new("tmux")
            .args(["kill-session", "-t", session_name])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to kill session '{session_name}': {stderr}"
            )));
        }

        Ok(())
    }

    /// Check if a tmux session exists.
    pub fn session_exists(session_name: &str) -> bool {
        Command::new("tmux")
            .args(["has-session", "-t", session_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// List all active tmux sessions with the "kaiju-" prefix.
    pub fn list_nexus_sessions() -> Result<Vec<String>> {
        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()?;

        if !output.status.success() {
            // No server running = no sessions, not an error
            return Ok(vec![]);
        }

        let sessions = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|s| s.starts_with("kaiju-"))
            .map(|s| s.to_string())
            .collect();

        Ok(sessions)
    }

    /// Send an interrupt (Ctrl-C) to a tmux session.
    pub fn send_interrupt(session_name: &str) -> Result<()> {
        let output = Command::new("tmux")
            .args(["send-keys", "-t", session_name, "C-c", ""])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to send interrupt to '{session_name}': {stderr}"
            )));
        }

        Ok(())
    }
}
