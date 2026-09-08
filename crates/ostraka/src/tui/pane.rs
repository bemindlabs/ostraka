//! One line of work, and the several of them a browser can hold.
//!
//! A pane is a thread, the repository it runs in, and the task being written
//! into it. Everything else — the runs recorded here, the record being read,
//! whatever dialog is open — belongs to the workspace rather than to a pane,
//! and stays where it is.
//!
//! **Panes are switched between, not shown side by side.** Two transcripts on
//! an eighty-column terminal are two transcripts nobody can read, and the width
//! is what this screen spends on the thing being read. Which one you are in is
//! a row at the top; getting to another is one keystroke.
//!
//! **One run at a time across all of them.** Not a property of panes: the
//! request to stop is a single flag, because a signal is single, so two runs
//! going at once would both stop when either was asked to. Running several is
//! the parallel-execution question and this does not answer it — what panes
//! buy is keeping several lines of work open, not running them together.

use crate::tui::thread::Thread;
use crate::workspace::Repository;

pub struct Pane {
    pub thread: Thread,
    /// The repository this line of work is in. `None` until one is chosen.
    pub repository: Option<Repository>,
    /// The task being written here.
    pub prompt: String,
    /// How far back into what has been asked here the box has been walked.
    pub history_at: Option<usize>,
    /// Whether the transcript sticks to the bottom as the run writes to it.
    pub follow: bool,
}

// Scrolling is not here. It belongs to the screen rather than to the line of
// work — the record screen scrolls too, and it is the same screen whichever
// pane is in front — and moving between panes puts it back to the bottom,
// which is where a transcript somebody has just come back to should be.

impl Default for Pane {
    fn default() -> Self {
        Self {
            thread: Thread::default(),
            repository: None,
            prompt: String::new(),
            history_at: None,
            follow: true,
        }
    }
}

impl Pane {
    pub fn new(repository: Option<Repository>) -> Self {
        Self {
            repository,
            ..Self::default()
        }
    }

    /// What the pane is called on the bar: the repository it works in, or the
    /// task it is working on where that says more.
    pub fn title(&self) -> String {
        if let Some(repo) = &self.repository {
            return repo.name.clone();
        }
        match self.thread.turns.first() {
            Some(turn) => turn.prompt.clone(),
            None => "new".to_string(),
        }
    }

    pub fn running(&self) -> bool {
        self.thread.running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repository(name: &str) -> Repository {
        Repository {
            name: name.to_string(),
            path: PathBuf::from("/p").join(name),
        }
    }

    #[test]
    fn a_pane_is_named_for_the_repository_it_works_in() {
        assert_eq!(Pane::new(Some(repository("scratch"))).title(), "scratch");
    }

    #[test]
    fn a_pane_with_nowhere_to_work_yet_says_so_rather_than_being_blank() {
        // A bar of unnamed panes is a bar nobody can navigate.
        assert_eq!(Pane::new(None).title(), "new");
    }
}
