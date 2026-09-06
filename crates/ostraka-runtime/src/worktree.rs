//! Isolated checkouts, one per task.
//!
//! Work happens in a worktree so that a change is a diff on disk before it is
//! anything else — reviewable by an agent that did not write it, and discardable
//! without touching the branch anyone else is on.

use crate::{Error, Result};
use ostraka_core::identity::ActorId;
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

/// Commits the staged change inside the worktree, as the agent that wrote it.
///
/// The identity is set on the command rather than read from git config, for two
/// reasons. A change an agent wrote should not be attributed to whichever human
/// happened to start the run — the audit trail is the product. And a run that
/// has already done all its work should not be thrown away at the last step
/// because the machine has no `user.email` configured, which is the ordinary
/// state of a CI runner.
///
/// Committing is as far as a run goes. Merging is a separate, human-initiated
/// act: an approved change is ready to merge, not already merged.
pub fn commit(worktree: &Path, message: &str, author: &ActorId) -> Result<()> {
    let out = Command::new("git")
        .arg("-c")
        .arg(format!("user.name={author}"))
        .arg("-c")
        // .invalid is reserved by RFC 2606 and can never resolve, which is the
        // point: this address identifies an agent, it does not reach anyone.
        .arg(format!(
            "user.email={}@ostraka.invalid",
            email_local(author)
        ))
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

/// An actor id reduced to something git will accept left of the `@`.
///
/// Identities are free-form strings; an id containing a space or an angle
/// bracket would produce a malformed address and a commit git refuses.
fn email_local(author: &ActorId) -> String {
    let cleaned: String = author
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "agent".to_string()
    } else {
        cleaned
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_with_spaces_still_yields_a_usable_address() {
        assert_eq!(email_local(&ActorId::new("agent archon")), "agent-archon");
        assert_eq!(email_local(&ActorId::new("archon")), "archon");
    }

    #[test]
    fn an_empty_identity_falls_back_rather_than_producing_an_at_sign_alone() {
        assert_eq!(email_local(&ActorId::new("")), "agent");
    }
}
