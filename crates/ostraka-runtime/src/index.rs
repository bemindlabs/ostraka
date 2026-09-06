//! What runs exist, and how they ended.
//!
//! `replay` reads one run back by id. Nothing enumerated them, so a run id had
//! to be copied off the terminal that produced it — and an id nobody kept was a
//! run nobody could find. An action that leaves no trace did not happen; a trace
//! nobody can list is barely better.

use crate::{Error, Result};
use ostraka_core::identity::ActorId;
use ostraka_core::record::{Outcome, RunRecord};
use std::path::Path;

/// One run, as much of it as its record can say.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub run_id: String,
    pub started_at: String,
    pub prompt: String,
    pub author: ActorId,
    pub adapter: String,
    pub reviewer: Option<ActorId>,
    pub outcome: Option<Outcome>,
    pub checks_passed: usize,
    pub checks_total: usize,
}

impl RunSummary {
    /// A run whose record is missing or unreadable.
    ///
    /// Listed rather than skipped. A run interrupted before it wrote its record
    /// still happened, and hiding it would make the listing agree with itself
    /// by leaving out the runs someone most needs to find.
    fn unfinished(run_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            started_at: String::new(),
            prompt: "(no record — the run did not finish)".to_string(),
            author: ActorId::new(""),
            adapter: String::new(),
            reviewer: None,
            outcome: None,
            checks_passed: 0,
            checks_total: 0,
        }
    }

    fn from_record(record: &RunRecord) -> Self {
        Self {
            run_id: record.run_id.clone(),
            started_at: record.started_at.clone(),
            prompt: record.prompt.clone(),
            author: record.author.clone(),
            adapter: record.adapter.clone(),
            reviewer: record.approval.as_ref().map(|a| a.reviewer.clone()),
            outcome: record.outcome,
            checks_passed: record.checks.iter().filter(|c| c.passed()).count(),
            checks_total: record.checks.len(),
        }
    }

    pub fn approved(&self) -> bool {
        self.outcome == Some(Outcome::Approved)
    }
}

/// Every run under `<records_root>/runs/`, newest first.
///
/// Ordered by the timestamp a run id ends with rather than by the id itself:
/// an id is `<task>-<timestamp>`, and the task part sorts first, which put a
/// listing in process-id order and called it chronological. Reading the time
/// off the directory name rather than out of the record means a run that never
/// wrote one still lands in the right place.
pub fn list(records_root: &Path) -> Result<Vec<RunSummary>> {
    let dir = records_root.join("runs");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut ids: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let when = |id: &str| {
        id.rsplit_once('-')
            .map(|(_, t)| t.to_string())
            .unwrap_or_default()
    };
    ids.sort_by(|a, b| when(b).cmp(&when(a)).then_with(|| b.cmp(a)));

    Ok(ids
        .iter()
        .map(|id| match read(&dir.join(id)) {
            Some(record) => RunSummary::from_record(&record),
            None => RunSummary::unfinished(id),
        })
        .collect())
}

fn read(dir: &Path) -> Option<RunRecord> {
    let text = std::fs::read_to_string(dir.join("record.json")).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::gate::{Approval, CheckRecord, Verdict};
    use std::path::PathBuf;

    fn root() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ostraka-index-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn write_run(records_root: &Path, run_id: &str, outcome: Outcome, passed: bool) {
        let dir = records_root.join("runs").join(run_id);
        std::fs::create_dir_all(&dir).expect("run dir");
        let record = RunRecord {
            run_id: run_id.to_string(),
            task_id: "t".into(),
            prompt: format!("do {run_id}"),
            author: ActorId::new("archon"),
            adapter: "a".into(),
            started_at: "2026-09-07T00:00:00Z".into(),
            finished_at: None,
            checks: vec![CheckRecord {
                name: "test".into(),
                cmd: "true".into(),
                exit_code: Some(if passed { 0 } else { 1 }),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 1,
            }],
            approval: Some(Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Approve,
            }),
            outcome: Some(outcome),
        };
        std::fs::write(
            dir.join("record.json"),
            serde_json::to_string(&record).expect("serializes"),
        )
        .expect("write");
    }

    #[test]
    fn a_project_that_has_never_run_lists_nothing_rather_than_failing() {
        assert!(list(&root()).expect("lists").is_empty());
    }

    #[test]
    fn runs_are_listed_newest_first() {
        let root = root();
        write_run(&root, "t1-20260907T000100Z", Outcome::Approved, true);
        write_run(&root, "t2-20260907T000300Z", Outcome::Rejected, false);
        write_run(&root, "t3-20260907T000200Z", Outcome::Approved, true);

        let runs = list(&root).expect("lists");
        let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "t2-20260907T000300Z",
                "t3-20260907T000200Z",
                "t1-20260907T000100Z"
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_that_never_wrote_a_record_is_still_listed() {
        // It happened. A listing that leaves out the interrupted runs agrees
        // with itself by omitting exactly the ones someone is looking for.
        let root = root();
        write_run(&root, "t1-20260907T000100Z", Outcome::Approved, true);
        std::fs::create_dir_all(root.join("runs").join("t2-20260907T000200Z")).expect("dir");

        let runs = list(&root).expect("lists");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "t2-20260907T000200Z");
        assert!(runs[0].outcome.is_none());
        assert!(runs[0].prompt.contains("did not finish"));
        assert!(runs[1].approved());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_summary_carries_what_the_run_was_for() {
        let root = root();
        write_run(&root, "t1-20260907T000100Z", Outcome::Rejected, false);
        let runs = list(&root).expect("lists");
        let run = &runs[0];
        assert_eq!(run.prompt, "do t1-20260907T000100Z");
        assert_eq!(run.author.as_str(), "archon");
        assert_eq!(run.reviewer.as_ref().map(ActorId::as_str), Some("ephor"));
        assert_eq!((run.checks_passed, run.checks_total), (0, 1));
        assert!(!run.approved());
        let _ = std::fs::remove_dir_all(&root);
    }
}
