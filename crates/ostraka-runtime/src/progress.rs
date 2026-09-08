//! Watching a run while it is still running.
//!
//! The run log is the record and is written either way. This is the
//! shoulder-read: somewhere to send what is happening as it happens, so a
//! screen can show a run in progress instead of a blank pane for the minute it
//! takes.
//!
//! A watcher is handed a [`Step`] and returns nothing. That is the whole
//! guarantee, and it is deliberate: watching cannot become steering, and a
//! watcher that panics or blocks cannot change what the run decides. Sending is
//! the watcher's problem — the shipped one drops what it cannot deliver,
//! because a run must not stall on a screen that has gone away.

use ostraka_core::gate::CheckRecord;
use ostraka_core::record::Event;

/// Which part of the pipeline a run is in.
///
/// Named for what is happening rather than numbered, because the numbers in
/// the orchestrator's comments have already changed once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Making the worktree the change will be written in.
    Isolating,
    /// Linking in what git ignores, and running the project's setup.
    Preparing,
    /// The agent that writes the change.
    Authoring,
    /// The project's own checks.
    Gating,
    /// A different vendor, reading the diff.
    Reviewing,
}

impl Phase {
    pub const ALL: [Phase; 5] = [
        Phase::Isolating,
        Phase::Preparing,
        Phase::Authoring,
        Phase::Gating,
        Phase::Reviewing,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Phase::Isolating => "isolate",
            Phase::Preparing => "prepare",
            Phase::Authoring => "author",
            Phase::Gating => "gate",
            Phase::Reviewing => "review",
        }
    }
}

/// One thing that happened, as it happened.
#[derive(Debug, Clone)]
pub enum Step {
    /// A phase began. Everything after this belongs to it, until the next one.
    Entered(Phase),
    /// An agent said something, or the run recorded something about itself.
    Said { phase: Phase, event: Event },
    /// One gate check finished. Carried whole, because a failing check's output
    /// is the thing worth reading and waiting for the record to be written
    /// would mean waiting for the rest of the gate first.
    Checked(CheckRecord),
}

/// Somewhere to send a run's steps while it is still running.
pub trait Watcher: Send {
    fn saw(&mut self, step: Step);
}

/// A watcher over a channel, which is the only kind anything here needs.
///
/// A failed send is dropped rather than reported. The receiver going away means
/// whoever was watching has stopped watching; the run is still writing its log
/// and still has to finish, and stalling it on a closed channel would make a
/// closed window able to break a run.
pub struct Channel(pub std::sync::mpsc::Sender<Step>);

impl Watcher for Channel {
    fn saw(&mut self, step: Step) {
        let _ = self.0.send(step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_channel_watcher_passes_steps_through_in_order() {
        let (tx, rx) = mpsc::channel();
        let mut watcher = Channel(tx);
        watcher.saw(Step::Entered(Phase::Authoring));
        watcher.saw(Step::Entered(Phase::Gating));

        let seen: Vec<Phase> = rx
            .try_iter()
            .filter_map(|s| match s {
                Step::Entered(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(seen, [Phase::Authoring, Phase::Gating]);
    }

    #[test]
    fn a_watcher_whose_receiver_is_gone_does_not_fail_the_run() {
        // A closed window must not be able to break a run that is still
        // writing its log.
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let mut watcher = Channel(tx);
        watcher.saw(Step::Entered(Phase::Isolating));
    }

    #[test]
    fn every_phase_is_named() {
        for phase in Phase::ALL {
            assert!(!phase.title().is_empty(), "{phase:?}");
        }
    }
}
