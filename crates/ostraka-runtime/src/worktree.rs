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
/// Whether this directory is somewhere git can make a worktree.
///
/// The whole runtime stands on `git worktree add`, so a directory that is not
/// a repository cannot run anything — and used to say so for the first time
/// two minutes into a run, in git's own words, after a vendor had been paid.
/// Asked once, cheaply, by whoever is about to promise that a run will work.
pub fn is_repository(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Whether this repository has a commit to branch from.
///
/// `git worktree add <path> HEAD` on a repository nobody has committed to
/// fails with `invalid reference: HEAD`, which is the second half of the same
/// problem `is_repository` catches the first half of: `git init` alone is not
/// enough to run in.
pub fn has_a_commit(repo: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

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

/// Keeps a linked name out of git's sight, in this worktree only.
///
/// A link is not part of the change. Left visible, `git status` reports it as
/// something the agent added, so it counts as a touched path, it is committed
/// with the work, and a reviewer is shown a symlink nobody asked for — which
/// is what happened the first time a real vendor ran against a workspace with
/// notes in it.
///
/// Written to the worktree's own exclude file rather than to a `.gitignore`:
/// that file belongs to the repository and is not this to edit. A failure here
/// is not worth failing the run over — the worst case is the link showing up
/// in a diff, which is where this started.
fn exclude(worktree: &Path, name: &str) {
    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(worktree)
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }
    let path = worktree.join(path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == name) {
        return;
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{name}");
    }
}

/// What went wrong getting a worktree ready to work in.
///
/// Separate from a gate failure on purpose. "The environment was not ready" and
/// "the change was rejected" are different answers, and reporting the first as
/// the second is what made a missing `node_modules` read as a refused change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupProblem {
    pub step: String,
    pub reason: String,
}

/// Makes a fresh checkout usable: links what git ignores, runs what the project
/// says it needs.
///
/// Runs before the agent, not merely before the gate. An agent that cannot run
/// the project's own tools cannot see what it broke, which is how a run ends
/// with the agent having changed nothing and nobody knowing why.
/// Whether the workspace's notes reached this worktree as a link.
///
/// The fact the author prompt turns on, and it cannot be read off the
/// configuration. `[worktree]` naming notes is an intention; a repository that
/// tracks its own `notes/` keeps it, because [`prepare`] leaves what the
/// checkout brought rather than replacing it. Asking the config would then
/// tell an agent that a directory the repository owns is not part of the
/// repository, and invite it to write into the diff it is about to be judged
/// on.
///
/// A symlink is not enough on its own: a repository may track one under that
/// name. The property the sentence rests on is that it points *out* of the
/// checkout, so that is what is checked.
pub fn notes_linked(worktree: &Path) -> bool {
    let path = worktree.join("notes");
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    match (std::fs::read_link(&path), worktree.canonicalize()) {
        (Ok(target), Ok(root)) => !target.starts_with(&root),
        _ => false,
    }
}

pub fn prepare(
    project: &Path,
    worktree: &Path,
    config: &ostraka_core::config::WorktreeConfig,
    notes: Option<&Path>,
    ceiling: Option<std::time::Duration>,
) -> std::result::Result<Vec<String>, SetupProblem> {
    let mut done = Vec::new();

    // The workspace's notes, linked rather than copied and rather than
    // configured. An agent writing here writes into the real directory, so
    // what it worked out survives a refusal — and because the link points out
    // of the checkout, none of it lands in the diff a reviewer judges. Notes
    // are what was learned; the diff is what was changed.
    let linked: Vec<(String, PathBuf)> = notes
        .map(|path| ("notes".to_string(), path.to_path_buf()))
        .into_iter()
        .chain(config.link.iter().map(|n| (n.clone(), project.join(n))))
        .collect();

    for (name, source) in linked {
        let name = &name;
        let target = worktree.join(name);
        if !source.exists() {
            // Said plainly rather than left to surface as an unrunnable check.
            // Declaring it means needing it, so its absence is the answer.
            return Err(SetupProblem {
                step: format!("link {name}"),
                reason: format!(
                    "{} is declared in [worktree] link and is not there; the worktree cannot be \
                     prepared without it",
                    source.display()
                ),
            });
        }
        // Something the repository tracks under that name already arrived with
        // the checkout, and it is not this to replace.
        if target.exists() || std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Absolute, so the link does not depend on how deep `[worktree] base`
        // puts the checkout — the fragility a relative `../../` would carry.
        let source = source.canonicalize().unwrap_or(source);
        if let Err(e) = symlink(&source, &target) {
            return Err(SetupProblem {
                step: format!("link {name}"),
                reason: format!("could not link {} into the worktree: {e}", source.display()),
            });
        }
        exclude(worktree, name);
        done.push(format!("link {name}"));
    }

    if let Some(command) = config.setup.as_deref().filter(|c| !c.trim().is_empty()) {
        let record = crate::gate::run_command(command, worktree, ceiling);
        if record.exit_code != Some(0) {
            let tail: Vec<&str> = record
                .stderr
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return Err(SetupProblem {
                step: "setup".to_string(),
                reason: format!(
                    "`{command}` exited with {}: {}",
                    record
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "no exit code".to_string()),
                    if tail.is_empty() {
                        "and said nothing".to_string()
                    } else {
                        tail.join(" / ")
                    }
                ),
            });
        }
        done.push(format!("setup `{command}`"));
    }

    Ok(done)
}

#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    }
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

/// Removes a worktree, keeping the branch it was on.
///
/// The distinction matters: the branch holds the commit a run produced, and
/// promotion, replay and the diff pane all read it from there. Taking the
/// branch would take the change.
pub fn release(repo: &Path, wt: &Worktree) -> Result<()> {
    remove_checkout(repo, &wt.path)
}

/// Removes a worktree by path, for one that has outlived its run.
pub fn release_path(repo: &Path, path: &Path) -> Result<()> {
    remove_checkout(repo, path)
}

fn remove_checkout(repo: &Path, path: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(path)
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

/// Every worktree this project has created, by path.
pub fn list(repo: &Path, base: &Path) -> Result<Vec<PathBuf>> {
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let _ = repo;
    let mut found: Vec<PathBuf> = std::fs::read_dir(base)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::config::WorktreeConfig;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ostraka-prep-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("project")).expect("project");
        std::fs::create_dir_all(path.join("wt")).expect("worktree");
        path
    }

    fn prep_config(link: &[&str], setup: Option<&str>) -> WorktreeConfig {
        WorktreeConfig {
            base: "worktrees".into(),
            link: link.iter().map(|s| (*s).to_string()).collect(),
            setup: setup.map(str::to_string),
        }
    }

    #[test]
    fn what_git_ignores_is_linked_into_the_checkout() {
        // The reported defect: a worktree is a fresh checkout, so node_modules
        // is absent and every check needing the toolchain fails for a reason
        // that has nothing to do with the change.
        let dir = scratch("link");
        std::fs::create_dir_all(dir.join("project/node_modules")).expect("deps");
        std::fs::write(dir.join("project/node_modules/marker"), "here").expect("write");

        let done = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&["node_modules"], None),
            None,
            None,
        )
        .expect("prepares");

        assert_eq!(done, ["link node_modules"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("wt/node_modules/marker")).expect("reads"),
            "here"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_workspaces_notes_reach_every_worktree_without_being_configured() {
        // What an agent works out along the way should survive the run that
        // worked it out — including a refused one, which is the run whose
        // notes are worth the most.
        let dir = scratch("notes");
        std::fs::create_dir_all(dir.join("notes")).expect("notes");
        std::fs::create_dir_all(dir.join("project")).expect("project");
        std::fs::write(dir.join("notes/earlier.md"), "what was worked out").expect("write");

        let done = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&[], None),
            Some(&dir.join("notes")),
            None,
        )
        .expect("prepares");

        assert_eq!(done, ["link notes"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("wt/notes/earlier.md")).expect("reads"),
            "what was worked out"
        );

        // Written through the link, so it lands in the workspace rather than
        // in a checkout that is about to be thrown away.
        std::fs::write(dir.join("wt/notes/during.md"), "what was learned").expect("write");
        assert!(
            dir.join("notes/during.md").is_file(),
            "the note stayed in the worktree"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_worktree_without_notes_is_prepared_anyway() {
        // A workspace nobody has taken notes in is not a broken workspace.
        let dir = scratch("no-notes");
        std::fs::create_dir_all(dir.join("project")).expect("project");
        let done = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&[], None),
            None,
            None,
        )
        .expect("prepares");
        assert!(done.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_link_is_absolute_so_the_worktree_depth_does_not_matter() {
        // A relative `../../` breaks the moment [worktree] base changes.
        let dir = scratch("absolute");
        std::fs::create_dir_all(dir.join("project/node_modules")).expect("deps");
        std::fs::create_dir_all(dir.join("wt/deep/deeper")).expect("deep");
        prepare(
            &dir.join("project"),
            &dir.join("wt/deep/deeper"),
            &prep_config(&["node_modules"], None),
            None,
            None,
        )
        .expect("prepares");
        let link = std::fs::read_link(dir.join("wt/deep/deeper/node_modules")).expect("a link");
        assert!(link.is_absolute(), "{link:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_declared_link_that_is_absent_is_said_plainly() {
        let dir = scratch("missing");
        let problem = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&["node_modules"], None),
            None,
            None,
        )
        .expect_err("must refuse");
        assert_eq!(problem.step, "link node_modules");
        assert!(problem.reason.contains("is not there"), "{problem:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn something_the_repository_tracks_is_not_replaced_by_a_link() {
        let dir = scratch("tracked");
        std::fs::create_dir_all(dir.join("project/vendor")).expect("source");
        std::fs::create_dir_all(dir.join("wt/vendor")).expect("checked out");
        std::fs::write(dir.join("wt/vendor/theirs"), "tracked").expect("write");

        prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&["vendor"], None),
            None,
            None,
        )
        .expect("prepares");
        assert!(
            dir.join("wt/vendor/theirs").is_file(),
            "the checkout lost a tracked file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_setup_command_that_fails_reports_the_environment_not_the_change() {
        let dir = scratch("setup-fails");
        let problem = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&[], Some("echo no registry >&2; exit 1")),
            None,
            None,
        )
        .expect_err("must refuse");
        assert_eq!(problem.step, "setup");
        assert!(problem.reason.contains("no registry"), "{problem:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_setup_command_runs_inside_the_worktree() {
        let dir = scratch("setup-cwd");
        prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&[], Some("pwd > where")),
            None,
            None,
        )
        .expect("prepares");
        let ran_in = std::fs::read_to_string(dir.join("wt/where")).expect("reads");
        assert!(ran_in.trim().ends_with("wt"), "{ran_in}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_declared_means_nothing_done() {
        let dir = scratch("nothing");
        let done = prepare(
            &dir.join("project"),
            &dir.join("wt"),
            &prep_config(&[], None),
            None,
            None,
        )
        .expect("prepares");
        assert!(done.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
