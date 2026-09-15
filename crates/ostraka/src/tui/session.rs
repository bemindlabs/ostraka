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

use crate::consult;
use crate::mode::Mode;
use crate::run;
use crate::workspace::Workspace;
use ostraka_core::record::{Event, Outcome};
use ostraka_runtime::progress::{Channel, Phase, Step, Watcher};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Instant;

/// The seat a vendor that could not run was sitting in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    Writes,
    Reviews,
}

/// A run that ended because a vendor could not run, not because of anything
/// about the change. Credit that ran out, a login that expired and a server
/// that is down all end here, and they are the case where another profile is
/// worth offering: the task was never judged, so running it again elsewhere
/// takes nothing back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    pub seat: Seat,
    /// The profile that could not run, where it is known.
    pub failed: Option<String>,
    /// The profile in the other seat, which a replacement cannot also be.
    pub other: Option<String>,
    /// What was typed, to run again.
    pub task: String,
    /// The vendor's own account, as the run described it.
    pub why: String,
}

/// The seat whose vendor could not run, when that is why a run was refused.
///
/// A rejection is a verdict and a failed check is about the change, so
/// neither is one. A timeout and an empty change are not either: a profile
/// that ran out of clock or found nothing to do did run.
pub fn vendor_failure(refusal: &ostraka_runtime::gate::Refusal) -> Option<Seat> {
    use ostraka_runtime::gate::Refusal;
    match refusal {
        Refusal::AuthorFailed { .. } => Some(Seat::Writes),
        // The orchestrator's own words for a reviewer it could not launch or
        // that exited non-zero. They are a rejection so that nothing unjudged
        // is ever approved, and this is where they are told apart from one.
        Refusal::Rejected { reason } if reason.starts_with("reviewer could not") => {
            Some(Seat::Reviews)
        }
        _ => None,
    }
}

/// The seat routing could not fill, from what was named and what exists, so
/// the offer names the right seat without reading routing's wording.
///
/// A named profile that is not here is the seat it was named for. A reviewer
/// named as the author too is the reviewer's seat. An author that was named
/// and exists means the reviewer is what could not be found. With nothing
/// named, it is the author's seat.
pub fn unroutable(args: &run::Args, ids: &[String]) -> (Seat, Option<String>, Option<String>) {
    let here = |id: &String| ids.contains(id);
    if let Some(author) = args.adapter.as_ref().filter(|id| !here(id)) {
        return (
            Seat::Writes,
            Some(author.clone()),
            args.review_adapter.clone(),
        );
    }
    if let Some(reviewer) = args
        .review_adapter
        .as_ref()
        .filter(|id| !here(id) || args.adapter.as_ref() == Some(*id))
    {
        return (Seat::Reviews, Some(reviewer.clone()), args.adapter.clone());
    }
    if args.adapter.is_some() {
        return (
            Seat::Reviews,
            args.review_adapter.clone(),
            args.adapter.clone(),
        );
    }
    (Seat::Writes, None, args.review_adapter.clone())
}

/// How a run ended, said the way the command line says it.
pub struct Finished {
    pub run_id: String,
    pub outcome: Option<Outcome>,
    pub summary: String,
    /// The plan a consultation in plan mode came back with, when it came back
    /// clean. What enter runs next.
    pub plan: Option<String>,
}

/// The worker's last word: how the run ended, and whether a vendor failing
/// was why.
type Ended = (Result<Finished, String>, Option<Fallback>);

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
    /// Set when it ended because a vendor could not run.
    pub fallback: Option<Fallback>,
    /// A stop has been asked for and the vendors have not noticed yet.
    pub stopping: bool,
    /// This run's own stop. Requesting it stops this run and nothing else;
    /// Ctrl-C, which is the process-wide request, still stops it too.
    stop: ostraka_adapter::interrupt::Stop,
    started: Instant,
    steps_in: Option<Receiver<Step>>,
    done_in: Option<Receiver<Ended>>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts a run, or a consultation, on a thread and returns something to
    /// watch it with.
    pub fn start(workspace: Workspace, args: run::Args, mode: Mode) -> Self {
        let prompt = args.prompt.clone();
        // Made before the thread and owned by this session, not shared. A stop
        // asked for in the same breath as the run — n, a task, enter, s — lands
        // on a flag the run is already holding, and asking this pane to stop
        // cannot reach a run in any other pane.
        let stop = ostraka_adapter::interrupt::Stop::new();
        let worker_stop = stop.clone();
        let (steps_out, steps_in) = mpsc::channel();
        let (done_out, done_in) = mpsc::channel();

        let worker = std::thread::spawn(move || {
            let mut fallback = None;
            let result = if mode.consults() {
                consult::consult(
                    &workspace,
                    &args,
                    mode,
                    Some(Box::new(Channel(steps_out))),
                    &worker_stop,
                )
                .map(|consulted| Finished {
                    run_id: consulted.id.clone(),
                    outcome: None,
                    summary: consulted.summary(),
                    plan: (mode == Mode::Plan && consulted.clean())
                        .then(|| consulted.answer.clone()),
                })
            } else {
                let attempts = args.attempts;
                run::execute_looping(
                    &workspace,
                    &args,
                    || Some(Box::new(Channel(steps_out.clone())) as Box<dyn Watcher>),
                    &worker_stop,
                    |n, refusal| {
                        // Said in the transcript, where the attempt it explains
                        // is about to appear.
                        let _ = steps_out.send(Step::Said {
                            phase: Phase::Authoring,
                            event: Event::Message {
                                text: format!(
                                    "attempt {n} of {attempts} \u{2014} the last one was not kept: {}",
                                    run::describe(refusal)
                                ),
                                raw: None,
                            },
                        });
                    },
                )
                .map(|report| {
                    fallback = report.refusal.as_ref().and_then(|refusal| {
                        let seat = vendor_failure(refusal)?;
                        let author = Some(report.record.adapter.clone());
                        Some(Fallback {
                            seat,
                            failed: match seat {
                                Seat::Writes => author.clone(),
                                // The record names the author and not the
                                // reviewer; a reviewer somebody named is known.
                                Seat::Reviews => args.review_adapter.clone(),
                            },
                            other: match seat {
                                // The reviewer never ran, so only one somebody
                                // named is known.
                                Seat::Writes => args.review_adapter.clone(),
                                Seat::Reviews => author,
                            },
                            task: String::new(),
                            why: run::describe(refusal),
                        })
                    });
                    Finished {
                        run_id: report.record.run_id.clone(),
                        outcome: report.record.outcome,
                        summary: match (&report.token, &report.refusal) {
                            (Some(_), _) => "approved — nothing merged".to_string(),
                            (None, Some(refusal)) => {
                                format!("rejected — {}", run::describe(refusal))
                            }
                            (None, None) => "rejected".to_string(),
                        },
                        plan: None,
                    }
                })
            }
            // Flattened to a string here, on the thread that produced it.
            // A boxed error is not `Send`, and the screen has no use for
            // one that a sentence does not serve better.
            .map_err(|e| {
                // No profile could take the seat at all: a named one that is
                // not installed, or none that answers.
                if e.downcast_ref::<crate::discover::NoAdapter>().is_some() {
                    let ids: Vec<String> = workspace
                        .profiles()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|profile| profile.id)
                        .collect();
                    let (seat, failed, other) = unroutable(&args, &ids);
                    fallback = Some(Fallback {
                        seat,
                        failed,
                        other,
                        task: String::new(),
                        why: e.to_string().lines().next().unwrap_or_default().to_string(),
                    });
                }
                e.to_string()
            });
            let _ = done_out.send((result, fallback));
        });

        Self {
            stop,
            prompt,
            steps: Vec::new(),
            phase: None,
            elapsed_secs: 0,
            finished: None,
            failed: None,
            fallback: None,
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
                Ok((result, fallback)) => {
                    self.fallback = fallback;
                    Some(result)
                }
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

    /// Asks this run, and only this run, to stop.
    ///
    /// Through the run's own stop rather than the process-wide flag, so a run
    /// in another pane carries on. The adapters and the gate notice within a
    /// poll and kill what they launched. The run still finishes — as
    /// `Interrupted`, which is its own outcome and not a verdict on the change.
    pub fn stop(&mut self) {
        if self.live() {
            self.stop.request();
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
            fallback: None,
            stopping: false,
            started: Instant::now(),
            steps_in: None,
            done_in: None,
            worker: None,
            stop: ostraka_adapter::interrupt::Stop::new(),
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
    fn a_vendor_that_could_not_run_is_told_apart_from_a_verdict() {
        use ostraka_runtime::gate::Refusal;
        let author = Refusal::AuthorFailed {
            code: "1".into(),
            diagnostics: Some("insufficient credit".into()),
        };
        assert_eq!(vendor_failure(&author), Some(Seat::Writes));
        let reviewer = Refusal::Rejected {
            reason: "reviewer could not run (exit 1): quota exceeded".into(),
        };
        assert_eq!(vendor_failure(&reviewer), Some(Seat::Reviews));
        let verdict = Refusal::Rejected {
            reason: "the parser now accepts an empty id".into(),
        };
        assert_eq!(
            vendor_failure(&verdict),
            None,
            "a verdict was offered a retry"
        );
        assert_eq!(vendor_failure(&Refusal::NoChange), None);
        assert_eq!(vendor_failure(&Refusal::TimedOut { after_secs: 5 }), None);
        assert_eq!(vendor_failure(&Refusal::Interrupted), None);
    }

    #[test]
    fn a_seat_routing_could_not_fill_is_read_from_what_was_named() {
        let ids = vec!["alpha".to_string(), "beta".to_string()];
        let named = |author: Option<&str>, reviewer: Option<&str>| {
            let mut args = run::Args::for_task("a task".into());
            args.adapter = author.map(str::to_string);
            args.review_adapter = reviewer.map(str::to_string);
            unroutable(&args, &ids)
        };
        assert_eq!(
            named(Some("gone"), Some("beta")),
            (Seat::Writes, Some("gone".into()), Some("beta".into()))
        );
        assert_eq!(
            named(Some("alpha"), Some("gone")),
            (Seat::Reviews, Some("gone".into()), Some("alpha".into()))
        );
        assert_eq!(
            named(Some("alpha"), Some("alpha")),
            (Seat::Reviews, Some("alpha".into()), Some("alpha".into()))
        );
        // The author was found, so the reviewer is what routing could not find.
        assert_eq!(
            named(Some("alpha"), None),
            (Seat::Reviews, None, Some("alpha".into()))
        );
        assert_eq!(named(None, None), (Seat::Writes, None, None));
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
                plan: None,
            }),
        );
        assert!(!session.live());
        assert_eq!(session.phase, None);
    }
}
