//! `ostraka task` and `ostraka tasks` — the list, from a shell.
//!
//! Printing only. What a task is and how one is taken lives in
//! [`crate::tasks`], because a second answer to "which task is next" would be
//! a second answer to a question two runs may be asking at the same moment.

use crate::tasks::{self, State};
use crate::workspace::Workspace;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn add(
    workspace: &Workspace,
    prompt: &str,
    repository: Option<&str>,
    adapter: Option<&str>,
    json: bool,
) -> Outcome {
    // Named repositories are checked now rather than when a run picks the task
    // up. A typo found here is a typo somebody can fix; found later it is a run
    // that failed for a reason that happened yesterday.
    if let Some(name) = repository {
        workspace.repository(Some(name))?;
    }
    let task = tasks::add(&workspace.ostraka(), prompt, repository, adapter)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&task)?);
    } else {
        println!("{}  {}", task.id, task.prompt);
    }
    Ok(true)
}

pub fn release(workspace: &Workspace, id: &str, json: bool) -> Outcome {
    tasks::release(&workspace.ostraka(), id)?;
    if json {
        println!("{}", serde_json::json!({ "released": id }));
    } else {
        println!("{id} is waiting again");
    }
    Ok(true)
}

pub fn list(workspace: &Workspace, json: bool) -> Outcome {
    let root = workspace.ostraka();
    let pending = tasks::list(&root, State::Pending);
    let running = tasks::list(&root, State::Running);
    let done = tasks::list(&root, State::Done);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pending": pending,
                "running": running,
                "done": done,
            }))?
        );
        return Ok(true);
    }

    if pending.is_empty() && running.is_empty() && done.is_empty() {
        println!("nothing on the list — `ostraka task add \"...\"` writes one down");
        return Ok(true);
    }

    // Running first: it is the one somebody is waiting on. Then what is next,
    // then what is behind them — the order the question is usually asked in.
    for (state, tasks) in [
        (State::Running, &running),
        (State::Pending, &pending),
        (State::Done, &done),
    ] {
        for task in tasks {
            let tail = match (&task.run_id, &task.outcome) {
                (Some(run), Some(outcome)) => format!("  {run} {outcome}"),
                (Some(run), None) => format!("  {run}"),
                _ => String::new(),
            };
            println!(
                "{:<26} {:<8} {}{tail}",
                task.id,
                state.name(),
                truncate(&task.prompt, 52)
            );
        }
    }
    Ok(true)
}

/// Takes the oldest task and runs it.
///
/// The claim happens before the run and the result is written back after it, so
/// a task is out of the pending list for exactly as long as something is
/// working on it. A run that dies leaves the task in `running/`, where it is
/// visible and `ostraka task release` puts it back — better than a timeout that
/// has to guess whether a long run is a dead one.
///
/// What the task recorded wins over what the flags said, because the task is
/// the thing somebody wrote down on purpose. The flags are the defaults for a
/// task that named nothing.
pub fn run_next(workspace: &Workspace, mut args: crate::run::Args, json: bool) -> Outcome {
    let root = workspace.ostraka();
    let Some(task) = tasks::claim(&root)? else {
        if !json {
            println!("nothing on the list");
        }
        return Ok(true);
    };

    args.prompt = task.prompt.clone();
    if task.repository.is_some() {
        args.repository = task.repository.clone();
    }
    if task.adapter.is_some() {
        args.adapter = task.adapter.clone();
    }

    if !json {
        println!("taking {}  {}", task.id, truncate(&task.prompt, 60));
    }

    match crate::run::run_reporting(workspace, &args, json) {
        Ok((approved, run_id)) => {
            let outcome = if approved { "approved" } else { "refused" };
            tasks::finish(&root, task, &run_id, outcome)?;
            Ok(approved)
        }
        Err(e) => {
            // Left in `running/`. A run that could not start is a thing to look
            // at, and moving the task back would hide that it was ever tried.
            Err(e)
        }
    }
}

fn truncate(text: &str, at: usize) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() <= at {
        return flat;
    }
    flat.chars().take(at.saturating_sub(1)).collect::<String>() + "\u{2026}"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_named_for_a_repository_nobody_has_is_refused_when_it_is_written() {
        // The alternative is a run that fails tomorrow for a typo made today,
        // by which time the run looks like the thing that is broken.
        let dir = std::env::temp_dir().join(format!("ostraka-taskcmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("adapters");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = []\n\n[gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        std::fs::create_dir_all(dir.join("repositories/here")).expect("repository");

        let workspace = Workspace::at(&dir);
        assert!(add(&workspace, "a task", Some("nowhere"), None, false).is_err());
        assert!(add(&workspace, "a task", Some("here"), None, false).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
