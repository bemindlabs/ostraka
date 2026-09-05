//! Isolated checkouts, one per task.
//!
//! Work happens in a worktree so that a change is a diff on disk before it is
//! anything else — reviewable by an agent that did not write it, and discardable
//! without touching the branch anyone else is on.

use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Worktree {
    path: PathBuf,
    branch: String,
}

impl Worktree {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }
}

/// Creates a worktree for a run, branching from `base_ref`.
pub fn create(repo: &Path, base: &Path, run_id: &str, base_ref: &str) -> Result<Worktree> {
    let path = base.join(run_id);
    let branch = format!("ostraka/{run_id}");

    let out = Command::new("git")
        .args(["worktree", "add", "-b", &branch])
        .arg(&path)
        .arg(base_ref)
        .current_dir(repo)
        .output()?;

    if !out.status.success() {
        return Err(Error::Other(format!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(Worktree { path, branch })
}

/// Lists paths modified inside a worktree.
///
/// Read from git, never from the agent's own account of what it did — an agent
/// that misreports its edits should not be able to shrink its own diff.
pub fn touched_paths(worktree: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree)
        .output()?;

    if !out.status.success() {
        return Err(Error::Other(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            // Porcelain v1: two status columns, a space, then the path.
            line.get(3..).map(|p| p.trim().to_string())
        })
        .filter(|p| !p.is_empty())
        .collect())
}

/// The change a run produced, as a diff against the base ref.
///
/// This is what a reviewer sees. It is read from git rather than from the
/// agent, so an agent cannot narrow its own diff by under-reporting.
pub fn diff(worktree: &Path) -> Result<String> {
    // Stage everything first so that new files appear in the diff at all;
    // untracked files are invisible to `git diff` otherwise.
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree)
        .output()?;
    if !add.status.success() {
        return Err(Error::Other(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        )));
    }

    let out = Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(worktree)
        .output()?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Commits the staged change inside the worktree.
///
/// Committing is as far as a run goes. Merging is a separate, human-initiated
/// act: an approved change is ready to merge, not already merged.
pub fn commit(worktree: &Path, message: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(worktree)
        .output()?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Removes a worktree and its branch.
pub fn remove(repo: &Path, wt: &Worktree) -> Result<()> {
    let out = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&wt.path)
        .current_dir(repo)
        .output()?;

    if !out.status.success() {
        return Err(Error::Other(format!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}
