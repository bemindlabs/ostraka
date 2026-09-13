//! A request to stop, from outside the run.
//!
//! An operator pressing Ctrl-C wants the agent to stop, not the terminal to be
//! abandoned with a subprocess still writing into a worktree. The signal
//! arrives on another thread, so what it can do is set a flag; the loops that
//! wait on a vendor check it and kill the child the same way they would at a
//! deadline.
//!
//! The process-wide flag is still global, because a signal is: there is one
//! process and one Ctrl-C, and it stops everything.
//!
//! What was not true any more is that there is one run. `drain --workers`
//! runs several, and a browser with work in more than one pane wants to stop
//! one of them without stopping the rest — which a single flag cannot express,
//! and which is why the browser used to refuse a second run at all. [`Stop`]
//! is that per-run handle. It answers yes to its own request *or* the global
//! one, so Ctrl-C still stops every run that has a stop of its own, and code
//! that only ever asked the global flag did not have to change.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Asks every waiting adapter to stop. Safe to call from a signal handler.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Clears the request. For tests, which share one process.
pub fn clear() {
    REQUESTED.store(false, Ordering::SeqCst);
}

/// A request to stop one run.
///
/// Cloned into everything that run launches — its author, its reviewer, its
/// checks — so a request made through any clone reaches all of them and
/// reaches nothing else.
#[derive(Debug, Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    /// A stop nobody has asked for yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks this run, and only this run, to stop.
    pub fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether this run has been asked to stop — by its own request or by the
    /// process-wide one, which still means everything.
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst) || requested()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_runs_stop_is_not_anothers() {
        // Only the per-run flags are exercised here: the process-wide one is
        // shared with every other test in this binary, and its effect on a
        // `Stop` is asserted where those tests serialise on it.
        let mine = Stop::new();
        let theirs = Stop::new();
        let clone = mine.clone();
        clone.request();
        assert!(mine.own(), "a request through a clone did not arrive");
        assert!(!theirs.own(), "one run's stop reached another");
    }

    impl Stop {
        /// The per-run half alone, so this test does not race the global flag.
        fn own(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }
}
