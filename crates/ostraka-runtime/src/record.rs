//! Writing the run log.
//!
//! Events are appended as they happen rather than assembled at the end, so an
//! interrupted run still leaves an account of how far it got.

use crate::{Error, Result};
use ostraka_core::record::{Event, RunRecord};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct RunLog {
    dir: PathBuf,
    events: File,
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
        Ok(Self { dir, events })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append(&mut self, event: &Event) -> Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|e| Error::Other(format!("serializing event: {e}")))?;
        writeln!(self.events, "{line}")?;
        self.events.flush()?;
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
