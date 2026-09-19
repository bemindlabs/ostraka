//! Writing the run log.
//!
//! Events are appended as they happen rather than assembled at the end, so an
//! interrupted run still leaves an account of how far it got.

use crate::progress::{Phase, Step, Watcher};
use crate::{Error, Result};
use ostraka_core::gate::CheckRecord;
use ostraka_core::record::{Event, RunRecord};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct RunLog {
    dir: PathBuf,
    events: File,
    /// Told about everything the log is told about, when anyone is looking.
    ///
    /// It lives here rather than being threaded through the orchestrator
    /// because every `append` is already the sentence "something happened",
    /// and a second parameter carried through six functions to say the same
    /// thing twice is how the two drift apart.
    watcher: Option<Box<dyn Watcher>>,
    /// Which part of the pipeline the run is in, so an event can be attributed
    /// to the agent that produced it rather than arriving unlabelled.
    phase: Phase,
    /// The profiles this run is using, once routing has chosen them. Written
    /// out as `live.json` whenever the phase moves, so that what a run is
    /// doing can be seen from outside the process running it.
    running_as: Option<(String, String)>,
    /// `live.lock`, held for as long as this log exists. The OS lets go of it
    /// when the process ends, however it ends, which is how a reader tells a
    /// run that is going from one whose process was killed and left its
    /// `live.json` behind.
    _owner: Option<File>,
}

/// What a run in progress is doing, as `live.json` beside its event log.
///
/// The event log says what was said and the record says how it ended, and
/// neither says, while a run is going, which profile is writing it and which
/// will review it. A screen in the same process could ask its own session; a
/// screen watching a `drain` in another process had nothing to read, so it
/// could list a run as unfinished and not say who was working on it. This is
/// that fact, written by the runtime that knows it.
///
/// It exists only while the run does: it is removed once the record is
/// written. A process killed outright — a signal, a crash, a reboot — cannot
/// remove it, so the run also holds an OS lock on `live.lock` beside it, and
/// `index::live` leaves out a `live.json` nobody holds that lock for. The OS
/// releases the lock however the process ends, and a lock cannot outlive its
/// process the way a recorded pid can be reused by another one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Live {
    /// The profile writing the change.
    pub author: String,
    /// The profile that will review it.
    pub reviewer: String,
    pub phase: Phase,
}

/// Where a run's `live.json` is.
pub fn live_path(run_dir: &Path) -> PathBuf {
    run_dir.join("live.json")
}

/// Where the lock a running run holds is.
pub fn live_lock_path(run_dir: &Path) -> PathBuf {
    run_dir.join("live.lock")
}

/// Whether the process that wrote a run's `live.json` still holds its lock.
///
/// No lock file means no owner: the lock is taken before `live.json` is first
/// written. Where the file system cannot lock at all, the run is taken to be
/// going, which is what every run looked like before there was a lock.
pub fn live_owner_running(run_dir: &Path) -> bool {
    let Ok(file) = File::open(live_lock_path(run_dir)) else {
        return false;
    };
    match file.try_lock_shared() {
        // Nobody holds it. Released at once as `file` drops.
        Ok(()) => false,
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(_)) => true,
    }
}

impl RunLog {
    /// Opens `<root>/runs/<run_id>/`, creating it if needed.
    pub fn create(root: &Path, run_id: &str) -> Result<Self> {
        let dir = root.join("runs").join(run_id);
        fs::create_dir_all(&dir)?;
        let events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("events.jsonl"))?;
        Ok(Self {
            dir,
            events,
            watcher: None,
            phase: Phase::Isolating,
            running_as: None,
            _owner: None,
        })
    }

    /// Names the profiles this run is using, and starts reporting what it is
    /// doing where another process can read it.
    pub fn running_as(mut self, author: &str, reviewer: &str) -> Self {
        // The lock first, so no reader ever finds a `live.json` whose owner
        // has not taken it yet and calls the run dead. Best effort, like
        // `live.json` itself: without it the run still runs, and reads as dead
        // to a screen in another process.
        self._owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(live_lock_path(&self.dir))
            .ok()
            .filter(|file| file.lock().is_ok());
        self.running_as = Some((author.to_string(), reviewer.to_string()));
        self.report();
        self
    }

    /// Rewrites `live.json`. Best effort: a run is not failed because a file
    /// meant for somebody watching could not be written. Written to a
    /// temporary file and renamed, so a reader never sees half of one.
    fn report(&self) {
        let Some((author, reviewer)) = &self.running_as else {
            return;
        };
        let live = Live {
            author: author.clone(),
            reviewer: reviewer.clone(),
            phase: self.phase,
        };
        let Ok(json) = serde_json::to_string(&live) else {
            return;
        };
        let tmp = self.dir.join("live.json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, live_path(&self.dir));
        }
    }

    /// Sends everything this log is told to whoever is watching.
    pub fn watched_by(mut self, watcher: Option<Box<dyn Watcher>>) -> Self {
        self.watcher = watcher;
        self
    }

    /// Moves the run into a phase, and says so.
    pub fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.report();
        self.tell(Step::Entered(phase));
    }

    /// Reports one finished gate check.
    ///
    /// Not written here — the checks travel into the record whole, at the end,
    /// where they belong. This is only so a screen does not have to wait for
    /// the rest of the gate to learn that the first check passed.
    pub fn checked(&mut self, record: &CheckRecord) {
        self.tell(Step::Checked(record.clone()));
    }

    fn tell(&mut self, step: Step) {
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.saw(step);
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append(&mut self, event: &Event) -> Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|e| Error::Other(format!("serializing event: {e}")))?;
        writeln!(self.events, "{line}")?;
        self.events.flush()?;
        // Told after it is written, not before. The file is the record; a
        // watcher that saw an event the log then failed to write would be
        // showing something that did not happen.
        let phase = self.phase;
        self.tell(Step::Said {
            phase,
            event: event.clone(),
        });
        Ok(())
    }

    pub fn write_record(&self, record: &RunRecord) -> Result<()> {
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| Error::Other(format!("serializing record: {e}")))?;
        fs::write(self.dir.join("record.json"), json)?;
        // The record says how it ended, so there is nothing live left to say.
        // The lock file goes too; the lock itself is released as the log drops.
        let _ = fs::remove_file(live_path(&self.dir));
        let _ = fs::remove_file(live_lock_path(&self.dir));
        Ok(())
    }
}

/// Reads back an event log for replay.
pub fn read_events(dir: &Path) -> Result<Vec<Event>> {
    let text = fs::read_to_string(dir.join("events.jsonl"))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| Error::Other(format!("replay: {e}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watcher_is_told_what_the_log_is_told_and_in_which_phase() {
        use crate::progress::{Phase, Step};
        let root = std::env::temp_dir().join(format!("ostraka-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut log = RunLog::create(&root, "run-w")
            .expect("creates")
            .watched_by(Some(Box::new(crate::progress::Channel(tx))));

        log.enter(Phase::Authoring);
        log.append(&Event::Message {
            text: "working".into(),
            raw: None,
        })
        .expect("appends");
        log.enter(Phase::Gating);
        log.checked(&ostraka_core::gate::CheckRecord {
            name: "format".into(),
            cmd: "fmt".into(),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 3,
        });

        let steps: Vec<Step> = rx.try_iter().collect();
        assert_eq!(steps.len(), 4, "{steps:?}");
        assert!(matches!(steps[0], Step::Entered(Phase::Authoring)));
        // The phase travels with the event, so a transcript can say which agent
        // spoke rather than listing every line under one heading.
        assert!(matches!(
            steps[1],
            Step::Said {
                phase: Phase::Authoring,
                ..
            }
        ));
        assert!(matches!(steps[2], Step::Entered(Phase::Gating)));
        assert!(matches!(steps[3], Step::Checked(_)));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unwatched_log_writes_exactly_as_it_did_before() {
        let root = std::env::temp_dir().join(format!("ostraka-unwatched-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut log = RunLog::create(&root, "run-u").expect("creates");
        log.enter(crate::progress::Phase::Gating);
        log.append(&Event::Message {
            text: "only".into(),
            raw: None,
        })
        .expect("appends");
        assert_eq!(read_events(log.dir()).expect("reads").len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn events_survive_a_write_and_read_round_trip() {
        let root = std::env::temp_dir().join(format!("ostraka-test-{}", std::process::id()));
        let mut log = RunLog::create(&root, "run-1").expect("creates");
        log.append(&Event::Message {
            text: "first".into(),
            raw: None,
        })
        .expect("appends");
        log.append(&Event::Finished {
            exit_code: Some(0),
            files_touched: vec![],
        })
        .expect("appends");

        let events = read_events(log.dir()).expect("reads back");
        assert_eq!(events.len(), 2);

        fs::remove_dir_all(&root).ok();
    }
}
