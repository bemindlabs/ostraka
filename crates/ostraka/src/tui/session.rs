//! A run started from the browser, and watched while it happens.
//!
//! The run itself is on a thread of its own and reaches the screen over two
//! channels: the steps as they happen, and one final word about how it ended.
//! A thread and a channel, not an async runtime — the pipeline is sequential
//! and staying sequential is what keeps this to twenty lines of `std`.
//!
//! What runs is [`crate::run::execute`], the same function `ostraka run` calls.
//! Pressing a key and typing a command reach one pipeline, so the gate cannot
//! be different on one of them.

use crate::run;
use ostraka_core::record::Outcome;
use ostraka_runtime::progress::{Channel, Phase, Step};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Instant;

/// How a run ended, said the way the command line says it.
pub struct Finished {
    pub run_id: String,
    pub outcome: Option<Outcome>,
    pub summary: String,
}

pub struct Session {
    /// What was asked. Kept because the record does not exist until the end.
    pub prompt: String,
    pub steps: Vec<Step>,
    /// The phase the run is in, or `None` once it is over.
    pub phase: Option<Phase>,
    /// Read off the clock by the event loop, not by the drawing: a screen that
    /// asked the time while rendering could not be asserted against a buffer.
    pub elapsed_secs: u64,
    pub finished: Option<Finished>,
    /// The run could not be started, or stopped without saying why.
    pub failed: Option<String>,
    /// A stop has been asked for and the vendors have not noticed yet.
    pub stopping: bool,
    started: Instant,
    steps_in: Option<Receiver<Step>>,
    done_in: Option<Receiver<Result<Finished, String>>>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts a run on a thread and returns something to watch it with.
    pub fn start(project: PathBuf, args: run::Args) -> Self {
        let prompt = args.prompt.clone();
        let (steps_out, steps_in) = mpsc::channel();
        let (done_out, done_in) = mpsc::channel();

        let worker = std::thread::spawn(move || {
            let result = run::execute(&project, &args, Some(Box::new(Channel(steps_out))))
                .map(|report| Finished {
                    run_id: report.record.run_id.clone(),
                    outcome: report.record.outcome,
                    summary: match (&report.token, &report.refusal) {
                        (Some(_), _) => "approved — nothing merged".to_string(),
                        (None, Some(refusal)) => format!("refused — {}", run::describe(refusal)),
                        (None, None) => "refused".to_string(),
                    },
                })
                // Flattened to a string here, on the thread that produced it.
                // A boxed error is not `Send`, and the screen has no use for
                // one that a sentence does not serve better.
                .map_err(|e| e.to_string());
            let _ = done_out.send(result);
        });

        Self {
            prompt,
            steps: Vec::new(),
            phase: None,
            elapsed_secs: 0,
            finished: None,
            failed: None,
            stopping: false,
            started: Instant::now(),
            steps_in: Some(steps_in),
            done_in: Some(done_in),
            worker: Some(worker),
        }
    }

    /// True while the run is still going.
    pub fn live(&self) -> bool {
        self.finished.is_none() && self.failed.is_none()
    }

    /// Takes everything the worker has said since last time.
    ///
    /// Never blocks. It is called from the same loop that polls the keyboard,
    /// and a browser that stopped answering keys while a run was quiet would
    /// be a browser nobody could stop.
    pub fn drain(&mut self) {
        let incoming: Vec<Step> = match &self.steps_in {
            Some(rx) => rx.try_iter().collect(),
            None => Vec::new(),
        };
        for step in incoming {
            if let Step::Entered(phase) = step {
                self.phase = Some(phase);
            }
            self.steps.push(step);
        }

        let ended = match &self.done_in {
            Some(rx) => match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                // The worker is gone without a word, which means it panicked.
                // A run that vanished is not a run still going: saying so is
                // the difference between a screen someone waits at forever and
                // one they can act on.
                Err(TryRecvError::Disconnected) => {
                    Some(Err("the run stopped without saying why".to_string()))
                }
            },
            None => None,
        };

        if let Some(result) = ended {
            match result {
                Ok(finished) => self.finished = Some(finished),
                Err(why) => self.failed = Some(why),
            }
            self.phase = None;
            self.stopping = false;
            self.settle();
        }

        if self.live() {
            self.elapsed_secs = self.started.elapsed().as_secs();
        }
    }

    /// Asks the run to stop, the same way Ctrl-C asks the command to.
    ///
    /// The flag is global because a signal is global; the adapters notice
    /// within a poll and kill what they launched. The run still finishes — as
    /// `Interrupted`, which is its own outcome and not a verdict on the change.
    pub fn stop(&mut self) {
        if self.live() {
            ostraka_adapter::interrupt::request();
            self.stopping = true;
        }
    }

    /// Waits for the worker to finish, dropping the channels.
    ///
    /// Called when the run has ended, and again before the browser exits: a
    /// process that walked away from this thread would leave a vendor still
    /// writing into a worktree, which is the whole reason Ctrl-C was ever made
    /// to do anything more than exit.
    pub fn settle(&mut self) {
        self.steps_in = None;
        self.done_in = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    #[cfg(test)]
    /// A session with no thread behind it, for asserting on what it draws.
    pub fn recorded(prompt: &str, steps: Vec<Step>, finished: Option<Finished>) -> Self {
        Self {
            prompt: prompt.to_string(),
            phase: steps.iter().rev().find_map(|s| match s {
                Step::Entered(p) if finished.is_none() => Some(*p),
                _ => None,
            }),
            steps,
            elapsed_secs: 41,
            finished,
            failed: None,
            stopping: false,
            started: Instant::now(),
            steps_in: None,
            done_in: None,
            worker: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::record::Event;

    #[test]
    fn a_session_reads_the_phase_off_the_steps_it_is_given() {
        let session = Session::recorded(
            "do a thing",
            vec![
                Step::Entered(Phase::Authoring),
                Step::Said {
                    phase: Phase::Authoring,
                    event: Event::Message {
                        text: "working".into(),
                        raw: None,
                    },
                },
                Step::Entered(Phase::Gating),
            ],
            None,
        );
        assert_eq!(session.phase, Some(Phase::Gating));
        assert!(session.live());
    }

    #[test]
    fn a_finished_session_is_not_live_and_is_in_no_phase() {
        let session = Session::recorded(
            "do a thing",
            vec![Step::Entered(Phase::Reviewing)],
            Some(Finished {
                run_id: "t1-20260907T000300Z".into(),
                outcome: Some(Outcome::Approved),
                summary: "approved — nothing merged".into(),
            }),
        );
        assert!(!session.live());
        assert_eq!(session.phase, None);
    }
}
