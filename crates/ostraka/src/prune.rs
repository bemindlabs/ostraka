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

pub fn run(workspace: &Workspace, apply: bool, json: bool) -> Outcome {
    let config = workspace.config()?;
    let base = workspace.worktrees(&config);
    // Worktrees are the workspace's, and every repository that has any is
    // asked: a checkout belongs to the repository it was made from, and git
    // will only list it from there.
    let mut worktrees: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for repo in workspace.repositories() {
        for path in worktree::list(&repo.path, &base)? {
            worktrees.push((repo.path.clone(), path));
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

    let removable: Vec<&(std::path::PathBuf, std::path::PathBuf)> = worktrees
        .iter()
        .filter(|(_, path)| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            !unfinished.iter().any(|id| *id == name)
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "worktrees": worktrees.len(),
                "removable": removable.iter().map(|(_, p)| p.display().to_string()).collect::<Vec<_>>(),
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
            for (_, path) in &removable {
                let name = path.strip_prefix(&workspace.root).unwrap_or(path);
                println!("  {}", name.display());
            }
            println!("\nBranches and run records are untouched either way.");
            println!("Re-run with --apply to remove them.");
        }
        return Ok(true);
    }

    let mut removed = 0usize;
    for (repo, path) in &removable {
        match worktree::release_path(repo, path) {
            Ok(()) => removed += 1,
            Err(e) => eprintln!("could not remove {}: {e}", path.display()),
        }
    }
    if !json {
        println!("removed {removed} worktree(s); branches and records untouched");
    }
    Ok(true)
}
