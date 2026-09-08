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
        })
    }

    /// Sends everything this log is told to whoever is watching.
    pub fn watched_by(mut self, watcher: Option<Box<dyn Watcher>>) -> Self {
        self.watcher = watcher;
        self
    }

    /// Moves the run into a phase, and says so.
    pub fn enter(&mut self, phase: Phase) {
        self.phase = phase;
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
