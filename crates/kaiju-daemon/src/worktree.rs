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

/// Outcome of merging a run's branch into the workspace.
pub enum MergeOutcome {
    Merged,
    AlreadyUpToDate,
    Conflict,
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

    /// Ensure `path` is a git repository, initializing one when it isn't, so an
    /// agent can be isolated there. A no-op if `path` is already inside a working
    /// tree (including a subdirectory of an existing repo). A fresh repo gets an
    /// initial empty commit so worktrees have a base `HEAD` to branch from; a
    /// local identity is set on that one commit so it succeeds even without a
    /// global git config.
    pub fn ensure_repo(path: &Path) -> Result<()> {
        if Self::is_git_repo(path) {
            return Ok(());
        }
        std::fs::create_dir_all(path)?;
        let p = path.display().to_string();
        Self::git_ok(&["-C", &p, "init"], path, "initialize git repository")?;
        Self::git_ok(
            &[
                "-C",
                &p,
                "-c",
                "user.email=kaiju@localhost",
                "-c",
                "user.name=Kaiju",
                "commit",
                "--allow-empty",
                "-m",
                "kaiju: initialize repository",
            ],
            path,
            "create the initial commit",
        )
    }

    /// Run a git command, mapping a non-zero exit to a `Git` error describing
    /// `what` failed at `path`.
    fn git_ok(args: &[&str], path: &Path, what: &str) -> Result<()> {
        let output = Command::new("git").args(args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Git(format!(
                "failed to {what} at '{}': {}",
                path.display(),
                stderr.trim()
            )));
        }
        Ok(())
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
            Command::new("git")
                .args(&full)
                .output()
                .map_err(NexusError::Io)
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

    /// True if the working tree at `dir` has no staged/unstaged/untracked changes.
    pub fn is_clean(dir: &Path) -> Result<bool> {
        let out = Command::new("git")
            .args(["-C", &dir.display().to_string(), "status", "--porcelain"])
            .output()
            .map_err(NexusError::Io)?;
        Ok(out.status.success() && out.stdout.is_empty())
    }

    /// Stage everything and commit in `worktree`. Returns whether a commit was
    /// actually made (`false` when there was nothing to commit — not an error).
    pub fn commit_all(worktree: &Path, message: &str) -> Result<bool> {
        let dir = worktree.display().to_string();
        let add = Command::new("git")
            .args(["-C", &dir, "add", "-A"])
            .output()
            .map_err(NexusError::Io)?;
        if !add.status.success() {
            return Err(NexusError::Git(format!(
                "git add failed: {}",
                String::from_utf8_lossy(&add.stderr).trim()
            )));
        }
        let commit = Command::new("git")
            .args(["-C", &dir, "commit", "-m", message])
            .env("LC_ALL", "C") // the "nothing to commit" check reads git's text
            .output()
            .map_err(NexusError::Io)?;
        if commit.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            let stdout = String::from_utf8_lossy(&commit.stdout);
            if stdout.contains("nothing to commit") || stderr.contains("nothing to commit") {
                Ok(false)
            } else {
                Err(NexusError::Git(format!(
                    "git commit failed: {}",
                    stderr.trim()
                )))
            }
        }
    }

    /// Merge `branch` into the current branch at `workspace` (`--no-ff`). On
    /// conflict, aborts so the workspace is left clean, and reports `Conflict`.
    pub fn merge_branch(workspace: &Path, branch: &str, message: &str) -> Result<MergeOutcome> {
        let dir = workspace.display().to_string();
        let out = Command::new("git")
            .args(["-C", &dir, "merge", "--no-ff", "-m", message, branch])
            .env("LC_ALL", "C") // the "Already up to date" check reads git's text
            .output()
            .map_err(NexusError::Io)?;
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("Already up to date") {
                Ok(MergeOutcome::AlreadyUpToDate)
            } else {
                Ok(MergeOutcome::Merged)
            }
        } else if workspace_has_merge_in_progress(&dir) {
            let _ = Command::new("git")
                .args(["-C", &dir, "merge", "--abort"])
                .output();
            Ok(MergeOutcome::Conflict)
        } else {
            // Git refused before merging (no identity, unknown branch, ...).
            Err(NexusError::Git(format!(
                "git merge failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

/// True while a merge is stopped on conflicts (`MERGE_HEAD` exists).
fn workspace_has_merge_in_progress(dir: &str) -> bool {
    Command::new("git")
        .args(["-C", dir, "rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
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

    #[test]
    fn ensure_repo_initializes_a_plain_dir_with_a_base_commit() {
        let dir = std::env::temp_dir().join(format!("kaiju-ensure-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(!WorktreeManager::is_git_repo(&dir));
        WorktreeManager::ensure_repo(&dir).unwrap();
        assert!(WorktreeManager::is_git_repo(&dir));

        // A base HEAD must exist so worktrees can branch from it.
        let head = Command::new("git")
            .args(["-C", &dir.display().to_string(), "rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(
            head.status.success(),
            "fresh repo should have an initial commit"
        );

        // Idempotent: a second call on an existing repo is a no-op success.
        WorktreeManager::ensure_repo(&dir).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_that_git_refuses_is_an_error_not_a_conflict() {
        let dir = std::env::temp_dir().join(format!("kaiju-merge-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        WorktreeManager::ensure_repo(&dir).unwrap();

        // No such branch: git never starts a merge, so there is nothing to
        // resolve by hand — it must not be reported as a conflict.
        let outcome = WorktreeManager::merge_branch(&dir, "kaiju/missing", "merge");
        assert!(matches!(outcome, Err(NexusError::Git(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
