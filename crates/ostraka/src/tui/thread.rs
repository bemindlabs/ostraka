//! A chain of runs, each starting where the last one left off.
//!
//! One run is isolated by design: a worktree branched from `HEAD`, gated,
//! reviewed, committed, and left as a branch. That is the right unit for
//! reviewing a change and the wrong unit for doing a piece of work, because
//! the second task in any real piece of work starts by looking at what the
//! first one wrote — and off `HEAD` it cannot see it.
//!
//! A thread is the missing unit. Each run branches from the run before it, so
//! the chain is a stack of commits that each passed the gate and an independent
//! review on their own. Nothing about the gate is relaxed to get it: what
//! changes is one argument, `base_ref`.
//!
//! **A refused run is not built on.** The chain advances only where a run was
//! approved; after a refusal the next task starts from where the refused one
//! did. Continuing from a change the gate would not take would be a way of
//! taking it.

use crate::run;
use crate::tui::session::{Finished, Session};
use crate::workspace::Workspace;
use ostraka_core::record::Outcome;
use ostraka_runtime::progress::Step;
/// One run in the chain, once it is over.
pub struct Turn {
    pub prompt: String,
    pub steps: Vec<Step>,
    pub finished: Option<Finished>,
    /// The run could not be started, or stopped without saying why.
    pub failed: Option<String>,
}

impl Turn {
    pub fn approved(&self) -> bool {
        self.finished
            .as_ref()
            .is_some_and(|f| f.outcome == Some(Outcome::Approved))
    }
}

pub struct Thread {
    /// Runs that are over, oldest first.
    pub turns: Vec<Turn>,
    /// The run happening now.
    pub live: Option<Session>,
    /// Where the next run starts. `HEAD` until something has been approved.
    pub base_ref: String,
    /// What has been asked here, for the box to offer back.
    pub history: Vec<String>,
    /// The profile that writes, when somebody has picked one. `None` leaves
    /// the choice to routing, which is the default and usually right.
    pub adapter: Option<String>,
    /// The profile that reviews. Routing still refuses to let it be the same
    /// one as the author: choosing is not the same as being allowed to.
    pub review_adapter: Option<String>,
    /// A model hint, passed through to whichever profile takes it.
    pub model: Option<String>,
    /// The identity a run is attributed to, and the one that reviews it. Both
    /// end up in the commit trailers, so they are worth being able to set.
    pub author: String,
    pub reviewer: String,
}

impl Default for Thread {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            live: None,
            base_ref: run::BASE_REF.to_string(),
            history: Vec::new(),
            adapter: None,
            review_adapter: None,
            model: None,
            author: run::AUTHOR.to_string(),
            reviewer: run::REVIEWER.to_string(),
        }
    }
}

impl Thread {
    pub fn running(&self) -> bool {
        self.live.as_ref().is_some_and(|s| s.live())
    }

    /// True when nothing has happened here yet.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty() && self.live.is_none()
    }

    /// True once the chain is standing on a run rather than on `HEAD`.
    pub fn continuing(&self) -> bool {
        self.base_ref != run::BASE_REF
    }

    /// Starts the next run, from wherever the chain has got to.
    pub fn start(&mut self, workspace: Workspace, repository: Option<String>, prompt: String) {
        self.history.push(prompt.clone());
        let mut args = run::Args::for_task(prompt);
        args.repository = repository;
        args.base_ref = self.base_ref.clone();
        args.adapter = self.adapter.clone();
        args.review_adapter = self.review_adapter.clone();
        args.model = self.model.clone();
        args.author = self.author.clone();
        args.reviewer = self.reviewer.clone();
        self.live = Some(Session::start(workspace, args));
    }

    /// Takes what the run has said, and closes it out when it is over.
    ///
    /// Returns the run id of a run that has just ended, so the caller can go
    /// and read the record that now exists.
    pub fn settle(&mut self) -> Option<String> {
        let session = self.live.as_mut()?;
        let was_live = session.live();
        session.drain();
        if was_live && session.live() {
            return None;
        }
        if was_live {
            return self.close_out();
        }
        None
    }

    /// Moves the finished run into the chain.
    fn close_out(&mut self) -> Option<String> {
        let session = self.live.take()?;
        let turn = Turn {
            prompt: session.prompt.clone(),
            steps: session.steps,
            finished: session.finished,
            failed: session.failed,
        };
        let ended = turn.finished.as_ref().map(|f| f.run_id.clone());

        // The chain advances only through the gate. A refused run leaves the
        // next task starting where this one did, because building on a change
        // the gate would not take is a way of taking it.
        if turn.approved() {
            if let Some(id) = &ended {
                self.base_ref = format!("ostraka/{id}");
            }
        }
        self.turns.push(turn);
        ended
    }

    /// Asks a running run to stop.
    pub fn stop(&mut self) {
        if let Some(session) = self.live.as_mut() {
            session.stop();
        }
    }

    /// Waits for the worker, for a browser on its way out.
    pub fn settle_worker(&mut self) {
        if let Some(session) = self.live.as_mut() {
            session.stop();
            session.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(run_id: &str, outcome: Outcome) -> Finished {
        Finished {
            run_id: run_id.to_string(),
            outcome: Some(outcome),
            summary: format!("{outcome:?}"),
        }
    }

    fn ended(thread: &mut Thread, prompt: &str, finished: Option<Finished>) {
        thread.live = Some(Session::recorded(prompt, Vec::new(), finished));
        thread.close_out();
    }

    #[test]
    fn a_thread_starts_at_head_and_moves_onto_the_run_it_approved() {
        let mut thread = Thread::default();
        assert_eq!(thread.base_ref, "HEAD");
        assert!(!thread.continuing());

        ended(
            &mut thread,
            "write a file",
            Some(finished("t1-20260908T000100Z", Outcome::Approved)),
        );
        assert_eq!(thread.base_ref, "ostraka/t1-20260908T000100Z");
        assert!(thread.continuing());
        assert_eq!(thread.turns.len(), 1);
    }

    #[test]
    fn a_refused_run_does_not_become_the_ground_the_next_one_stands_on() {
        // Building on a change the gate would not take is a way of taking it.
        let mut thread = Thread::default();
        ended(
            &mut thread,
            "write a file",
            Some(finished("t1-20260908T000100Z", Outcome::Approved)),
        );
        ended(
            &mut thread,
            "break the build",
            Some(finished("t2-20260908T000200Z", Outcome::Rejected)),
        );

        assert_eq!(
            thread.base_ref, "ostraka/t1-20260908T000100Z",
            "the chain advanced through a refusal"
        );
        assert_eq!(thread.turns.len(), 2);
    }

    #[test]
    fn a_run_that_never_started_leaves_the_chain_where_it_was() {
        let mut thread = Thread::default();
        let mut session = Session::recorded("a task", Vec::new(), None);
        session.failed = Some("no adapter profile can run here".into());
        thread.live = Some(session);
        thread.close_out();

        assert_eq!(thread.base_ref, "HEAD");
        assert_eq!(thread.turns.len(), 1);
        assert!(!thread.turns[0].approved());
    }

    #[test]
    fn what_was_asked_is_kept_whether_or_not_it_worked() {
        // The box offers it back, and a task worth retrying is usually one
        // that just failed.
        let mut thread = Thread::default();
        thread.history.push("first".into());
        thread.history.push("second".into());
        assert_eq!(thread.history, ["first", "second"]);
    }
}
