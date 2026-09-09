//! `ostraka drain` — take the list down, several at a time.
//!
//! The tagline says fleets and the runtime ran one task at a time. What was
//! missing was never the concurrency: a run already spends its life waiting on
//! a subprocess, and the streaming that watches one uses a thread and two
//! channels. What was missing was somewhere to take the second task *from*,
//! which is [`crate::tasks`], and a claim that two workers cannot both win,
//! which is that module's rename.
//!
//! **Threads, not tokio.** `AGENTS.md` says async goes in "when several
//! adapters need to stream at once". They do now, and it still did not need
//! async — the same way showing one run live turned out not to. Each worker is
//! the sequential run that already existed, on a thread of its own. `tokio`
//! buys nothing here: these tasks are subprocesses, not sockets, and there are
//! as many of them as somebody asked for rather than thousands.
//!
//! **One Ctrl-C stops everything, and that is the right answer.** The interrupt
//! flag is global and its own documentation calls that deliberate: there is one
//! process and one Ctrl-C. Stopping *one* run of several is a different
//! question — it is the browser's `s` key, it needs a per-run handle, and it is
//! not answered here.
//!
//! **Output is a line per task, not a transcript per run.** Four transcripts
//! interleaved on one terminal are four transcripts nobody can read, so the
//! per-run reporting is switched off and each worker says what it took and what
//! came of it, under a lock so the lines stay whole.

use crate::tasks;
use crate::workspace::Workspace;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// How many runs at once, when nobody says.
///
/// One is the behaviour that already existed, and it stays the default: a
/// machine spending four vendors at once is a thing to ask for rather than to
/// discover.
pub const WORKERS: usize = 1;

pub fn run(workspace: &Workspace, args: crate::run::Args, workers: usize, json: bool) -> Outcome {
    if workers == 0 {
        return Err("--workers 0 would take nothing from the list".into());
    }

    // Cleared once, here, rather than per run. A stop asked for while the list
    // is draining means stop draining; clearing it between tasks would make
    // Ctrl-C a request to skip one.
    ostraka_adapter::interrupt::clear();
    let interrupts = AtomicUsize::new(0);
    let _ = ctrlc::set_handler(move || {
        if interrupts.fetch_add(1, Ordering::SeqCst) == 0 {
            eprintln!("\nstopping — press Ctrl-C again to give up on what is going");
            ostraka_adapter::interrupt::request();
        } else {
            std::process::exit(130);
        }
    });

    let say = Mutex::new(());
    let taken = AtomicUsize::new(0);
    let approved = AtomicUsize::new(0);
    let refused = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if ostraka_adapter::interrupt::requested() {
                        return;
                    }
                    let claimed = match tasks::claim(&workspace.ostraka()) {
                        Ok(Some(task)) => task,
                        // Empty, or somebody else emptied it. Either way there
                        // is nothing left for this worker to do.
                        Ok(None) => return,
                        Err(e) => {
                            let _guard = say.lock();
                            eprintln!("could not take from the list: {e}");
                            return;
                        }
                    };
                    taken.fetch_add(1, Ordering::SeqCst);

                    let mut mine = args.clone();
                    mine.prompt = claimed.prompt.clone();
                    if claimed.repository.is_some() {
                        mine.repository = claimed.repository.clone();
                    }
                    if claimed.adapter.is_some() {
                        mine.adapter = claimed.adapter.clone();
                    }

                    // `execute` rather than `run`, which prints a transcript.
                    // Four of those at once is four nobody can read.
                    match crate::run::execute(workspace, &mine, None) {
                        Ok(report) => {
                            let outcome = if report.approved() {
                                approved.fetch_add(1, Ordering::SeqCst);
                                "approved"
                            } else {
                                refused.fetch_add(1, Ordering::SeqCst);
                                "refused"
                            };
                            let run_id = report.record.run_id.clone();
                            if let Err(e) =
                                tasks::finish(&workspace.ostraka(), claimed, &run_id, outcome)
                            {
                                let _guard = say.lock();
                                eprintln!("{run_id} finished but the list did not record it: {e}");
                            }
                            if !json {
                                let _guard = say.lock();
                                println!("{outcome:<9} {run_id}");
                            }
                        }
                        Err(e) => {
                            // Left in `running/`, where `ostraka tasks` shows it
                            // and `task release` puts it back. Returning it
                            // automatically would hand the next worker a task
                            // that has just failed to start.
                            failed.fetch_add(1, Ordering::SeqCst);
                            let _guard = say.lock();
                            eprintln!("could not run {}: {e}", claimed.id);
                        }
                    }
                }
            });
        }
    });

    let taken = taken.load(Ordering::SeqCst);
    let approved = approved.load(Ordering::SeqCst);
    let refused = refused.load(Ordering::SeqCst);
    let failed = failed.load(Ordering::SeqCst);
    let stopped = ostraka_adapter::interrupt::requested();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "taken": taken,
                "approved": approved,
                "refused": refused,
                "failed": failed,
                "workers": workers,
                "stopped": stopped,
            }))?
        );
    } else if taken == 0 {
        println!("nothing on the list");
    } else {
        let tail = if stopped { ", stopped" } else { "" };
        println!(
            "{taken} taken — {approved} approved, {refused} refused, {failed} could not run{tail}"
        );
    }

    // A refusal is a correct outcome reported correctly. Only a task that could
    // not be run at all, or a stop, is this command failing at its job.
    Ok(failed == 0 && !stopped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_workers_is_refused_rather_than_quietly_doing_nothing() {
        // `--workers 0` reads as a number somebody meant, and draining nothing
        // while reporting success is the worst answer to it.
        let dir = std::env::temp_dir().join(format!("ostraka-drain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let workspace = Workspace::at(&dir);
        let args = crate::run::Args::for_task(String::new());
        assert!(run(&workspace, args, 0, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
