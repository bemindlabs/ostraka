//! What to do about a problem the browser recognises.
//!
//! "did not finish — git worktree add failed: fatal: not a git repository" is
//! a true sentence and a dead end. It is git's account of the situation,
//! arriving from two layers down, after a run had been started and a vendor
//! had been paid — and it leaves the operator to work out on their own that
//! the answer is `git init`, and then that `git init` alone is not enough
//! either, because a worktree needs a commit to branch from.
//!
//! A remedy is that same knowledge written down as steps: what is wrong, what
//! has to happen, and which of those the browser can do. It does nothing on
//! its own. Every step is taken one at a time and only when asked for, because
//! the second one commits whatever is lying in the operator's directory and
//! that is not a thing to do quietly on somebody's behalf.
//!
//! Diagnosed from the state of the directory rather than from the text of an
//! error. Matching English out of another program is how a fix stops working
//! when that program rewords itself.

use ostraka_runtime::worktree;
use std::path::Path;
use std::process::Command;

/// One thing that has to happen, and whether the browser can do it.
pub struct Step {
    /// What this step is, in a sentence.
    pub said: String,
    /// What it will do that the operator should know before it does it.
    pub warns: Option<String>,
    /// The commands, in order. Empty means only a person can do this.
    pub commands: Vec<Vec<String>>,
}

/// A problem, and the way out of it.
pub struct Remedy {
    pub problem: String,
    pub steps: Vec<Step>,
    /// Which step is next. Equal to `steps.len()` once they are all done.
    pub at: usize,
    /// What the last step said, when it said anything worth showing.
    pub said: Option<String>,
    /// A step failed, and `said` is why. Nothing further is attempted.
    pub failed: bool,
}

impl Remedy {
    /// A workspace with nothing to work on.
    ///
    /// The only step here is one the browser cannot take: it does not know
    /// what you meant to clone, and guessing a URL is not a thing to guess.
    pub fn nothing_cloned(repositories: &Path) -> Remedy {
        Remedy {
            problem: format!(
                "Nothing has been cloned into {} yet, so there is nothing to work on.",
                repositories.display()
            ),
            steps: vec![Step {
                said: format!(
                    "Clone what you want worked on into {}, or start one: ctrl-x w, then n.",
                    repositories.display()
                ),
                warns: Some(
                    "Cloning is yours — nobody here knows which repository you meant. \
                     Starting one needs only a name."
                        .to_string(),
                ),
                commands: Vec::new(),
            }],
            at: 0,
            said: None,
            failed: false,
        }
    }

    /// What is wrong with this directory, if the browser knows.
    ///
    /// Read off the directory, not off an error message.
    pub fn diagnose(project: &Path) -> Option<Remedy> {
        let repository = worktree::is_repository(project);
        if repository && worktree::has_a_commit(project) {
            return None;
        }

        let mut steps = Vec::new();
        if !repository {
            steps.push(Step {
                said: "Make this directory a git repository.".to_string(),
                warns: None,
                commands: vec![words(&["git", "init", "-b", "main"])],
            });
        }
        steps.push(Step {
            said: "Commit what is here, so a run has something to branch from.".to_string(),
            // Said plainly, because this is the step that touches the
            // operator's own files. `git init` creates something; this one
            // takes everything in the directory and writes it into history.
            warns: Some(
                "This commits everything currently in this directory, respecting \
                 .gitignore \u{2014} and makes an empty commit where there is nothing \
                 yet, because a worktree needs one either way."
                    .to_string(),
            ),
            commands: vec![
                words(&["git", "add", "-A"]),
                // `--allow-empty` because a directory somebody has only just
                // made has nothing in it, and refusing to give it a commit
                // would refuse the whole point of the step.
                words(&["git", "commit", "-m", "Initial commit", "--allow-empty"]),
            ],
        });

        Some(Remedy {
            problem: if repository {
                "This repository has no commits, so a run has nothing to branch from.".to_string()
            } else {
                "This directory is not a git repository, so a run has nowhere to work.".to_string()
            },
            steps,
            at: 0,
            said: None,
            failed: false,
        })
    }

    pub fn done(&self) -> bool {
        self.at >= self.steps.len()
    }

    /// Takes the step that is next, and stops on the first thing that fails.
    ///
    /// Git's own words are kept on a failure. A commit with no author
    /// configured is the common one, and no sentence written here would be as
    /// useful as the one git prints about it.
    pub fn take_step(&mut self, project: &Path) {
        if self.failed {
            return;
        }
        let Some(step) = self.steps.get(self.at) else {
            return;
        };
        for argv in &step.commands {
            let (program, args) = argv.split_first().expect("a command has a program");
            let out = Command::new(program)
                .args(args)
                .current_dir(project)
                .output();
            match out {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let said = String::from_utf8_lossy(&out.stderr);
                    let said = said.trim();
                    self.said = Some(if said.is_empty() {
                        format!("{} failed", argv.join(" "))
                    } else {
                        said.to_string()
                    });
                    self.failed = true;
                    return;
                }
                Err(e) => {
                    self.said = Some(format!("{} could not be run: {e}", argv.join(" ")));
                    self.failed = true;
                    return;
                }
            }
        }
        self.at += 1;
        self.said = None;
    }
}

fn words(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|w| (*w).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ostraka-remedy-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_directory_git_has_never_heard_of_needs_two_steps() {
        let dir = scratch("bare");
        let remedy = Remedy::diagnose(&dir).expect("a problem");
        assert!(remedy.problem.contains("not a git repository"));
        assert_eq!(remedy.steps.len(), 2, "git init alone is not enough");
        assert!(
            remedy.steps.iter().all(|s| !s.commands.is_empty()),
            "both are the browser's to take"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repository_nobody_has_committed_to_needs_one() {
        let dir = scratch("empty-repo");
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&dir)
                .status()
                .expect("git runs")
                .success()
        );
        let remedy = Remedy::diagnose(&dir).expect("a problem");
        assert!(remedy.problem.contains("no commits"), "{}", remedy.problem);
        assert_eq!(remedy.steps.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn taking_every_step_leaves_a_directory_with_nothing_wrong_with_it() {
        let dir = scratch("fixed");
        std::fs::write(dir.join("seed.txt"), "seed\n").expect("write");
        let mut remedy = Remedy::diagnose(&dir).expect("a problem");

        while !remedy.done() && !remedy.failed {
            remedy.take_step(&dir);
            // The commit step needs an identity, and a machine may have none —
            // which is every runner this has. Giving the repository one as soon
            // as it exists is the difference between a test about the steps and
            // a test about the machine.
            //
            // It used to assert that a failure mentioned the author and then
            // return, so on any machine without a git identity everything below
            // never ran. A test that passes by leaving is a test that passes.
            if dir.join(".git").is_dir() {
                for (key, value) in [
                    ("user.email", "test@example.invalid"),
                    ("user.name", "test"),
                ] {
                    let _ = std::process::Command::new("git")
                        .args(["config", key, value])
                        .current_dir(&dir)
                        .output();
                }
            }
        }

        assert!(
            !remedy.failed,
            "a step failed: {:?}",
            remedy.said.as_deref()
        );
        assert!(remedy.done());
        assert!(
            Remedy::diagnose(&dir).is_none(),
            "the steps did not fix what they were for"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_step_that_fails_keeps_the_reason_and_stops() {
        // Nothing further is attempted, because the next step usually depends
        // on the one that just failed.
        let dir = scratch("fails");
        let mut remedy = Remedy::diagnose(&dir).expect("a problem");
        remedy.steps[0].commands = vec![words(&["git", "definitely-not-a-command"])];
        remedy.take_step(&dir);

        assert!(remedy.failed);
        assert!(remedy.said.is_some(), "git's own words were discarded");
        assert_eq!(remedy.at, 0, "a failed step counted as taken");
        std::fs::remove_dir_all(&dir).ok();
    }
}
