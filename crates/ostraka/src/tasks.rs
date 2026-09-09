//! The workspace's task list.
//!
//! A run has always been one task, typed at the moment it starts. That is the
//! right shape for asking a question and the wrong one for a piece of work with
//! six parts, and it is the reason a second machine, a second terminal or a
//! script has never been able to help: there was nowhere to put the work that
//! had not been started yet.
//!
//! **A list, not a team.** `bwoc` calls this a team's task list, and a team
//! there is a named subset of agents. Ostraka has no agents — it has adapter
//! profiles and it has runs, and its own rules say the agent roster "is not an
//! input to the runtime". Importing the word would import the concept, so what
//! is here is the part that survives translation: work that is written down
//! before somebody is free to do it, and taken by whoever is.
//!
//! **A directory per state, and claiming is a rename.** `rename` is atomic on
//! every platform this ships to, and it fails with `NotFound` for the loser of a
//! race — so two runs draining this list cannot both take the same task, and
//! nothing needs a lock file or a crate to say so. It is also readable: what is
//! waiting, what is going and what is finished are three `ls`s.
//!
//! ```text
//! .ostraka/tasks/
//!   pending/<id>.json     written down, nobody has started it
//!   running/<id>.json     claimed by a run that has not reported back
//!   done/<id>.json        finished, carrying the run that did it
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// One piece of work, before during or after the run that does it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    /// What to ask the agent, verbatim.
    pub prompt: String,
    pub added_at: String,
    /// Which repository it belongs in, where the workspace holds several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The profile that should write it, where somebody has decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    /// The run that took it, once one has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// What that run decided, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Which of the three directories a task is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Running,
    Done,
}

impl State {
    pub fn dir(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Running => "running",
            State::Done => "done",
        }
    }

    pub fn name(self) -> &'static str {
        self.dir()
    }
}

fn root(workspace_ostraka: &Path) -> PathBuf {
    workspace_ostraka.join("tasks")
}

fn place(workspace_ostraka: &Path, state: State, id: &str) -> PathBuf {
    root(workspace_ostraka).join(state.dir()).join(id)
}

/// Writes a task down. Returns its id.
///
/// The id carries the clock so that a listing is chronological without reading
/// every file, and a counter because two `add`s in one second are a thing a
/// script does.
pub fn add(
    workspace_ostraka: &Path,
    prompt: &str,
    repository: Option<&str>,
    adapter: Option<&str>,
) -> Result<Task> {
    let dir = place(workspace_ostraka, State::Pending, "");
    std::fs::create_dir_all(&dir)?;
    let now = ostraka_core::clock::now_rfc3339();
    let stamp = now.replace([':', '-'], "").replace('.', "");
    for n in 0..1000 {
        let id = format!("k{stamp}-{n}");
        let path = dir.join(format!("{id}.json"));
        // `create_new` rather than `exists` then write: two `add`s racing for
        // the same second must not agree on an id.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let task = Task {
                    id,
                    prompt: prompt.to_string(),
                    added_at: now,
                    repository: repository.map(str::to_string),
                    adapter: adapter.map(str::to_string),
                    run_id: None,
                    outcome: None,
                };
                use std::io::Write;
                file.write_all(serde_json::to_string_pretty(&task)?.as_bytes())?;
                return Ok(task);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err("a thousand tasks in one second is not a task list".into())
}

/// Every task in one state, oldest first.
pub fn list(workspace_ostraka: &Path, state: State) -> Vec<Task> {
    let dir = root(workspace_ostraka).join(state.dir());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    // By name, which is by the clock the id carries. Reading every file to sort
    // would make a listing cost what a listing is for avoiding.
    paths.sort();
    paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// Takes the oldest pending task, if there is one.
///
/// The claim is the rename. Two callers reaching the same file means one of
/// them moves it and the other is told it is not there, which is the answer
/// rather than an error — it tries the next one.
pub fn claim(workspace_ostraka: &Path) -> Result<Option<Task>> {
    let running = root(workspace_ostraka).join(State::Running.dir());
    std::fs::create_dir_all(&running)?;
    for task in list(workspace_ostraka, State::Pending) {
        let from = place(
            workspace_ostraka,
            State::Pending,
            &format!("{}.json", task.id),
        );
        let to = running.join(format!("{}.json", task.id));
        match std::fs::rename(&from, &to) {
            Ok(()) => return Ok(Some(task)),
            // Somebody else got there first. Not a failure: the list is shared,
            // and losing a race is what sharing one looks like.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(None)
}

/// Records what a run decided and moves the task out of the way.
pub fn finish(workspace_ostraka: &Path, mut task: Task, run_id: &str, outcome: &str) -> Result<()> {
    task.run_id = Some(run_id.to_string());
    task.outcome = Some(outcome.to_string());
    let done = root(workspace_ostraka).join(State::Done.dir());
    std::fs::create_dir_all(&done)?;
    std::fs::write(
        done.join(format!("{}.json", task.id)),
        serde_json::to_string_pretty(&task)?,
    )?;
    let running = place(
        workspace_ostraka,
        State::Running,
        &format!("{}.json", task.id),
    );
    let _ = std::fs::remove_file(running);
    Ok(())
}

/// Puts a claimed task back, for a run that never reported.
///
/// A machine that lost power mid-run leaves a task in `running/` that nothing
/// is working on, and no amount of waiting will move it. Deciding that for
/// somebody — by a timeout, say — would mean guessing whether a long run is
/// dead, so this is a thing a person does.
pub fn release(workspace_ostraka: &Path, id: &str) -> Result<()> {
    let from = place(workspace_ostraka, State::Running, &format!("{id}.json"));
    if !from.is_file() {
        return Err(format!("no task {id:?} is running").into());
    }
    let pending = root(workspace_ostraka).join(State::Pending.dir());
    std::fs::create_dir_all(&pending)?;
    std::fs::rename(&from, pending.join(format!("{id}.json")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-tasks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_task_written_down_is_there_to_be_taken() {
        let dir = scratch("add");
        let task = add(&dir, "write a file", None, None).expect("adds");
        assert_eq!(list(&dir, State::Pending).len(), 1);
        assert_eq!(list(&dir, State::Pending)[0].prompt, "write a file");

        let taken = claim(&dir).expect("claims").expect("one was there");
        assert_eq!(taken.id, task.id);
        assert!(list(&dir, State::Pending).is_empty(), "it is still pending");
        assert_eq!(list(&dir, State::Running).len(), 1);

        finish(&dir, taken, "t1-2026", "approved").expect("finishes");
        assert!(list(&dir, State::Running).is_empty());
        let done = list(&dir, State::Done);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].run_id.as_deref(), Some("t1-2026"));
        assert_eq!(done[0].outcome.as_deref(), Some("approved"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_claimants_never_take_the_same_task() {
        // The property the whole directory layout is for, and the one that has
        // to hold before anything drains this list in parallel. Threads rather
        // than a simulated race: the guarantee wanted is the filesystem's.
        let dir = scratch("race");
        for i in 0..8 {
            add(&dir, &format!("task {i}"), None, None).expect("adds");
        }
        let taken: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let dir = dir.clone();
                    s.spawn(move || {
                        let mut mine = Vec::new();
                        while let Ok(Some(task)) = claim(&dir) {
                            mine.push(task.id);
                        }
                        mine
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("thread"))
                .collect()
        });

        assert_eq!(taken.len(), 8, "a task was lost or taken twice: {taken:?}");
        let mut unique = taken.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            8,
            "the same task was claimed twice: {taken:?}"
        );
        assert!(list(&dir, State::Pending).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_task_nobody_reported_on_can_be_put_back() {
        let dir = scratch("release");
        add(&dir, "a task", None, None).expect("adds");
        let taken = claim(&dir).expect("claims").expect("one");
        assert!(release(&dir, &taken.id).is_ok());
        assert_eq!(list(&dir, State::Pending).len(), 1);
        assert!(list(&dir, State::Running).is_empty());
        // And a task that is not running is not one to put back.
        assert!(release(&dir, &taken.id).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_list_is_an_answer_rather_than_an_error() {
        let dir = scratch("empty");
        assert!(list(&dir, State::Pending).is_empty());
        assert!(claim(&dir).expect("claims").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
