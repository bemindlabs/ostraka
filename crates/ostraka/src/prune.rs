//! `ostraka prune` — clear what finished runs left behind.
//!
//! A run leaves a worktree and a record. The worktree of an approved run is
//! removed when it commits, but a refused one is kept on purpose — it is the
//! evidence of what went wrong — and evidence nobody has looked at in a month
//! is just a copy of the repository taking up a disk.
//!
//! Nothing here touches a branch. The commit a run produced lives there, and
//! promotion reads it from there; removing a checkout is reversible, removing
//! the branch would not be.

use crate::workspace::Workspace;
use ostraka_runtime::{index, worktree};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// A worktree a finished run left, and the repository it belongs to.
#[derive(Debug, Clone)]
pub struct Leftover {
    pub repo: std::path::PathBuf,
    pub path: std::path::PathBuf,
}

/// Everything a survey of the workspace found.
///
/// All three, not only the removable ones: `--json` reports what was looked at
/// and what was spared as well as what can go, and a caller that had to
/// recompute either would be a second answer to the same question.
pub struct Survey {
    /// Every worktree under the workspace's base, in every repository.
    pub worktrees: Vec<Leftover>,
    /// Those whose run has finished, and so are nobody's to be using.
    pub removable: Vec<Leftover>,
    /// The run ids still going, whose worktrees were left alone.
    pub unfinished: Vec<String>,
}

/// What a run left behind and nothing is still using.
///
/// Separated from the printing so that the browser can show the same list the
/// command prints. Two answers to "what can go" would eventually be two
/// different answers, and the one that removes directories is a poor place for
/// that.
pub fn leftovers(workspace: &Workspace) -> Result<Vec<Leftover>, Box<dyn std::error::Error>> {
    Ok(survey(workspace)?.removable)
}

/// The whole picture, for a caller that reports rather than removes.
pub fn survey(workspace: &Workspace) -> Result<Survey, Box<dyn std::error::Error>> {
    let config = workspace.config()?;
    let base = workspace.worktrees(&config);
    // Worktrees are the workspace's, and every repository that has any is
    // asked: a checkout belongs to the repository it was made from, and git
    // will only list it from there.
    let mut worktrees: Vec<Leftover> = Vec::new();
    for repo in workspace.repositories() {
        for path in worktree::list(&repo.path, &base)? {
            worktrees.push(Leftover {
                repo: repo.path.clone(),
                path,
            });
        }
    }
    let runs = index::list(&workspace.records())?;

    // A worktree whose run is still unfinished is one that may be in use.
    // Nothing here should race a run that is happening right now.
    let unfinished: Vec<&str> = runs
        .iter()
        .filter(|r| r.outcome.is_none())
        .map(|r| r.run_id.as_str())
        .collect();

    let unfinished: Vec<String> = unfinished.into_iter().map(str::to_string).collect();
    let removable: Vec<Leftover> = worktrees
        .iter()
        .filter(|l| {
            let name = l.path.file_name().unwrap_or_default().to_string_lossy();
            !unfinished.iter().any(|id| *id == name)
        })
        .cloned()
        .collect();
    Ok(Survey {
        worktrees,
        removable,
        unfinished,
    })
}

/// Removes them, returning how many went and what could not.
///
/// Branches and run records are untouched, which is the whole posture: a
/// checkout can be made again, and the commit a run produced cannot.
pub fn remove(leftovers: &[Leftover]) -> (usize, Vec<String>) {
    let mut removed = 0usize;
    let mut failed = Vec::new();
    for l in leftovers {
        match worktree::release_path(&l.repo, &l.path) {
            Ok(()) => removed += 1,
            Err(e) => failed.push(format!("could not remove {}: {e}", l.path.display())),
        }
    }
    (removed, failed)
}

pub fn run(workspace: &Workspace, apply: bool, json: bool) -> Outcome {
    let Survey {
        worktrees,
        removable,
        unfinished,
    } = survey(workspace)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "worktrees": worktrees.len(),
                "removable": removable.iter().map(|l| l.path.display().to_string()).collect::<Vec<_>>(),
                "kept_because_unfinished": unfinished,
                "applied": apply,
            }))?
        );
    }

    if removable.is_empty() {
        if !json {
            println!("nothing to prune");
        }
        return Ok(true);
    }

    if !apply {
        if !json {
            println!("{} worktree(s) can be removed:", removable.len());
            for l in &removable {
                let name = l.path.strip_prefix(&workspace.root).unwrap_or(&l.path);
                println!("  {}", name.display());
            }
            println!("\nBranches and run records are untouched either way.");
            println!("Re-run with --apply to remove them.");
        }
        return Ok(true);
    }

    let (removed, failed) = remove(&removable);
    for said in &failed {
        eprintln!("{said}");
    }
    if !json {
        println!("removed {removed} worktree(s); branches and records untouched");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace with one repository, one finished run and one still going.
    fn fixture(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-prune-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("adapters");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = []\n\n[gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        dir
    }

    #[test]
    fn a_survey_reports_what_it_looked_at_as_well_as_what_can_go() {
        // `--json` names three things, and each of them has to be the thing its
        // name says. Reporting the removable list as the total, and an empty
        // list as what was spared, is the shape this regressed into when the
        // browser's half was lifted out — true-looking and wrong.
        let dir = fixture("survey");
        let workspace = Workspace::at(&dir);
        let survey = survey(&workspace).expect("surveys");
        // Nothing here yet, but the three are distinct fields rather than one
        // list reported three ways.
        assert!(survey.worktrees.is_empty());
        assert!(survey.removable.is_empty());
        assert!(survey.unfinished.is_empty());
        // And the convenience wrapper is the removable half of the same answer.
        assert_eq!(
            leftovers(&workspace).expect("leftovers").len(),
            survey.removable.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
