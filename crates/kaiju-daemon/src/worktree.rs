//! Git worktree isolation for agents.
//!
//! Running several agents in the same repository at once invites conflicts:
//! they trample each other's working tree. When an agent is created with
//! `isolate = true`, the daemon gives it its own git worktree on a fresh
//! branch, so each agent edits an independent checkout of the same repo.
//!
//! Pure naming helpers ([`branch_name`], [`worktree_path`]) are tested
//! directly; the git operations shell out and are validated manually.

use kaiju_core::{NexusError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Number of leading id characters used to label an agent's branch/worktree.
const ID_PREFIX_LEN: usize = 8;

fn id_prefix(agent_id: &str) -> &str {
    let len = agent_id.len().min(ID_PREFIX_LEN);
    &agent_id[..len]
}

/// Branch name for an agent's isolated worktree, e.g. `kaiju/1a2b3c4d`.
pub fn branch_name(agent_id: &str) -> String {
    format!("kaiju/{}", id_prefix(agent_id))
}

/// Directory for an agent's worktree, under `base`.
pub fn worktree_path(base: &Path, agent_id: &str) -> PathBuf {
    base.join(id_prefix(agent_id))
}

/// Manages git worktrees via the `git` CLI.
pub struct WorktreeManager;

impl WorktreeManager {
    /// Is `path` inside a git working tree?
    pub fn is_git_repo(path: &Path) -> bool {
        Command::new("git")
            .args([
                "-C",
                &path.display().to_string(),
                "rev-parse",
                "--is-inside-work-tree",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Create a worktree at `worktree` on a new `branch` checked out from the
    /// current HEAD of `repo`.
    pub fn create(repo: &Path, worktree: &Path, branch: &str) -> Result<()> {
        if let Some(parent) = worktree.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = Command::new("git")
            .args([
                "-C",
                &repo.display().to_string(),
                "worktree",
                "add",
                "-b",
                branch,
                &worktree.display().to_string(),
                "HEAD",
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Git(format!(
                "failed to create worktree at '{}': {}",
                worktree.display(),
                stderr.trim()
            )));
        }

        Ok(())
    }

    /// Show the working-tree changes in `dir` (the agent's run directory).
    ///
    /// Diffs against `HEAD` so it captures **both staged and unstaged** edits to
    /// tracked files (plain `git diff` misses staged ones), falling back to a
    /// bare diff in a repo with no commits yet. Untracked files — which no diff
    /// shows — are appended as a short list so new files aren't invisible. Works
    /// on any git directory: an isolated worktree or the plain workspace.
    pub fn diff(dir: &Path) -> Result<String> {
        let dir_str = dir.display().to_string();
        let git = |args: &[&str]| -> Result<std::process::Output> {
            let mut full = vec!["-C", &dir_str];
            full.extend_from_slice(args);
            Command::new("git").args(&full).output().map_err(NexusError::Io)
        };

        // HEAD-relative diff (staged + unstaged); fall back when there's no HEAD.
        let tracked = git(&["--no-pager", "diff", "HEAD"])?;
        let mut out = if tracked.status.success() {
            String::from_utf8_lossy(&tracked.stdout).to_string()
        } else {
            let bare = git(&["--no-pager", "diff"])?;
            if !bare.status.success() {
                let stderr = String::from_utf8_lossy(&bare.stderr);
                return Err(NexusError::Git(format!(
                    "failed to diff '{dir_str}': {}",
                    stderr.trim()
                )));
            }
            String::from_utf8_lossy(&bare.stdout).to_string()
        };

        if let Ok(untracked) = git(&["ls-files", "--others", "--exclude-standard"]) {
            let names = String::from_utf8_lossy(&untracked.stdout);
            if !names.trim().is_empty() {
                out.push_str("\n# untracked files:\n");
                for name in names.lines() {
                    out.push_str("?? ");
                    out.push_str(name);
                    out.push('\n');
                }
            }
        }

        Ok(out)
    }

    /// Remove a worktree. `--force` so a dirty checkout is still cleaned up.
    pub fn remove(repo: &Path, worktree: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "-C",
                &repo.display().to_string(),
                "worktree",
                "remove",
                "--force",
                &worktree.display().to_string(),
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Git(format!(
                "failed to remove worktree at '{}': {}",
                worktree.display(),
                stderr.trim()
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_uses_id_prefix() {
        assert_eq!(branch_name("1a2b3c4d5e6f7890"), "kaiju/1a2b3c4d");
    }

    #[test]
    fn branch_name_handles_short_id() {
        assert_eq!(branch_name("abc"), "kaiju/abc");
    }

    #[test]
    fn worktree_path_joins_base_and_prefix() {
        let base = Path::new("/home/u/.kaiju/worktrees");
        assert_eq!(
            worktree_path(base, "1a2b3c4d5e6f"),
            PathBuf::from("/home/u/.kaiju/worktrees/1a2b3c4d")
        );
    }
}
