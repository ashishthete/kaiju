use kaiju_core::{NexusError, Result};
use std::process::Command;

/// Encode bytes as tmux `send-keys -H` hex arguments (one per byte). Pure.
fn hex_bytes(bytes: &[u8]) -> Vec<String> {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse tmux `#{pane_width}x#{pane_height}` output (e.g. "80x24"). Pure.
fn parse_size(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

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
    pub fn list_kaiju_sessions() -> Result<Vec<String>> {
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

    /// Capture the *visible* pane (current screen) with ANSI escapes preserved
    /// (`-e`), for rendering in a browser terminal. Unlike `capture_pane`, it
    /// omits `-S` so the result is exactly one screen — ideal for repaint.
    pub fn capture_pane_colored(session_name: &str) -> Result<String> {
        let output = Command::new("tmux")
            .args(["capture-pane", "-t", session_name, "-e", "-p"])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to capture pane '{session_name}': {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Inject raw bytes into the session as if typed, using `send-keys -H`
    /// (hex). This passes through control sequences (Ctrl-C, arrows, Esc, …)
    /// without any key-name mapping.
    pub fn send_raw_bytes(session_name: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "send-keys".to_string(),
            "-t".to_string(),
            session_name.to_string(),
            "-H".to_string(),
        ];
        args.extend(hex_bytes(bytes));

        let output = Command::new("tmux").args(&args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to send raw bytes to '{session_name}': {stderr}"
            )));
        }
        Ok(())
    }

    /// Report the pane size as `(cols, rows)` so the browser terminal can match
    /// it (avoids line-wrap mismatch).
    pub fn pane_size(session_name: &str) -> Result<(u16, u16)> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "-t",
                session_name,
                "#{pane_width}x#{pane_height}",
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to read pane size '{session_name}': {stderr}"
            )));
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        parse_size(&raw).ok_or_else(|| NexusError::Tmux(format!("unparseable pane size: {raw:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_encodes_each_byte_as_two_digits() {
        // Ctrl-C, then ESC [ A (up arrow)
        assert_eq!(
            hex_bytes(&[0x03, 0x1b, 0x5b, 0x41]),
            vec!["03", "1b", "5b", "41"]
        );
        assert_eq!(hex_bytes(&[0x00, 0xff]), vec!["00", "ff"]);
        assert!(hex_bytes(&[]).is_empty());
    }

    #[test]
    fn parse_size_reads_width_x_height() {
        assert_eq!(parse_size("80x24\n"), Some((80, 24)));
        assert_eq!(parse_size("  200x50  "), Some((200, 50)));
        assert_eq!(parse_size("nope"), None);
        assert_eq!(parse_size("80xfoo"), None);
    }
}
