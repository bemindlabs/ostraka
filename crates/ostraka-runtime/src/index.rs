//! What runs exist, and how they ended.
//!
//! `replay` reads one run back by id. Nothing enumerated them, so a run id had
//! to be copied off the terminal that produced it — and an id nobody kept was a
//! run nobody could find. An action that leaves no trace did not happen; a trace
//! nobody can list is barely better.

use crate::{Error, Result};
use ostraka_core::identity::ActorId;
use ostraka_core::record::{Outcome, RunRecord, TokenUsage};
use std::path::Path;

/// One run, as much of it as its record can say.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub run_id: String,
    pub started_at: String,
    pub prompt: String,
    pub author: ActorId,
    pub adapter: String,
    /// Which repository the change was made in.
    pub repository: String,
    pub reviewer: Option<ActorId>,
    pub outcome: Option<Outcome>,
    pub checks_passed: usize,
    pub checks_total: usize,
    /// What each adapter in this run reported spending.
    pub usage: Vec<TokenUsage>,
}

/// What one backend has cost across the runs on record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendUsage {
    pub adapter: String,
    pub input: u64,
    pub output: u64,
    /// Counts from vendors that report one combined figure instead of a split.
    ///
    /// Kept apart from `input` and `output` on purpose. Folding a combined
    /// total into the input side renders as "14.7k in / 0 out", and that zero
    /// is a claim the vendor never made.
    pub total: u64,
    pub runs: usize,
    /// Any part of this total came from a vendor that rounds.
    pub approximate: bool,
}

/// Per-backend totals across every run given, in adapter-id order.
///
/// Sums only what vendors reported. A backend that reports nothing does not
/// appear, which is the honest difference between "spent nothing" and "does not
/// say" — the caller can show the second as a dash rather than as a zero.
pub fn by_backend(runs: &[RunSummary]) -> Vec<BackendUsage> {
    let mut totals: std::collections::BTreeMap<String, BackendUsage> =
        std::collections::BTreeMap::new();
    for usage in runs.iter().flat_map(|run| run.usage.iter()) {
        let entry = totals
            .entry(usage.adapter.clone())
            .or_insert_with(|| BackendUsage {
                adapter: usage.adapter.clone(),
                input: 0,
                output: 0,
                total: 0,
                runs: 0,
                approximate: false,
            });
        entry.input += usage.input.unwrap_or(0);
        entry.output += usage.output.unwrap_or(0);
        entry.total += usage.total.unwrap_or(0);
        entry.runs += 1;
        entry.approximate |= usage.approximate;
    }
    totals.into_values().collect()
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
            repository: String::new(),
            reviewer: None,
            outcome: None,
            checks_passed: 0,
            checks_total: 0,
            usage: Vec::new(),
        }
    }

    fn from_record(record: &RunRecord) -> Self {
        Self {
            run_id: record.run_id.clone(),
            started_at: record.started_at.clone(),
            prompt: record.prompt.clone(),
            author: record.author.clone(),
            adapter: record.adapter.clone(),
            repository: record.repository.clone(),
            reviewer: record.approval.as_ref().map(|a| a.reviewer.clone()),
            outcome: record.outcome,
            checks_passed: record.checks.iter().filter(|c| c.passed()).count(),
            checks_total: record.checks.len(),
            usage: record.usage.clone(),
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

/// The change a finished run produced, read from git.
///
/// The run record does not keep the diff, and should not: it would be a second
/// copy of something git already stores exactly. A run's commit lives on its own
/// branch, or on the branch a promotion named.
///
/// The branch existing is not enough. A refused run has a branch — it was
/// created before the agent started — whose head is simply the base commit it
/// branched from, and showing that would present somebody else's change as this
/// run's output. So the head commit has to say it is this run's, by the trailer
/// the runtime wrote into it. `None` means the run produced no commit, which is
/// a true answer rather than a missing one.
pub fn diff(repo: &Path, run_id: &str) -> Result<Option<String>> {
    for branch in candidates(run_id) {
        if !head_is_this_run(repo, &branch, run_id)? {
            continue;
        }
        let out = std::process::Command::new("git")
            .args(["show", "--format=", "--patch", &branch])
            .current_dir(repo)
            .output()
            .map_err(|e| Error::Other(format!("git show: {e}")))?;
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            if !text.trim().is_empty() {
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}

/// The branch whose head is this run's commit, if it left one here.
///
/// The same two places [`diff`] reads from, and the same reason for checking
/// the head rather than trusting the name: a refused run has a branch too — it
/// was created before the agent started — whose head is the commit it branched
/// from. Building on that would silently start from somewhere else's work.
///
/// `None` is a true answer: the run produced no commit, or produced it in a
/// different repository.
pub fn commit_branch(repo: &Path, run_id: &str) -> Result<Option<String>> {
    for branch in candidates(run_id) {
        if head_is_this_run(repo, &branch, run_id)? {
            return Ok(Some(branch));
        }
    }
    Ok(None)
}

/// Where a run's commit can be, in the order it is looked for. Named once
/// because two lists of branch names drift, and the drift is silent.
fn candidates(run_id: &str) -> [String; 2] {
    [format!("ostraka/{run_id}"), format!("promoted/{run_id}")]
}

fn head_is_this_run(repo: &Path, branch: &str, run_id: &str) -> Result<bool> {
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%B", branch])
        .current_dir(repo)
        .output()
        .map_err(|e| Error::Other(format!("git log: {e}")))?;
    if !out.status.success() {
        return Ok(false);
    }
    let message = String::from_utf8_lossy(&out.stdout);
    Ok(message
        .lines()
        .any(|line| line.trim() == format!("Run: {run_id}")))
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
            repository: "only".into(),
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
            usage: Vec::new(),
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

    fn git(repo: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn a_refused_runs_branch_is_not_mistaken_for_its_change() {
        // A run's branch is created before the agent starts, so a refused run
        // leaves a branch whose head is the commit it branched from. Showing
        // that would present an unrelated change as this run's output — which
        // is worse than showing nothing, because it looks right.
        let repo = root();
        std::fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("seed.txt"), "seed\n").expect("write");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "someone else's change"]);

        // Refused: a branch, no commit of its own.
        git(&repo, &["branch", "ostraka/t-refused"]);
        assert_eq!(diff(&repo, "t-refused").expect("reads"), None);

        // Approved: a branch whose head carries the run trailer.
        git(&repo, &["checkout", "-q", "-b", "ostraka/t-approved"]);
        std::fs::write(repo.join("added.txt"), "new\n").expect("write");
        git(&repo, &["add", "-A"]);
        git(
            &repo,
            &["commit", "-q", "-m", "do a thing\n\nRun: t-approved"],
        );

        let change = diff(&repo, "t-approved")
            .expect("reads")
            .expect("has a diff");
        assert!(change.contains("added.txt"), "{change}");
        assert!(
            !change.contains("seed.txt"),
            "showed the base commit:\n{change}"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_run_with_no_branch_at_all_reads_as_no_change_rather_than_an_error() {
        let repo = root();
        std::fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init", "-q", "-b", "main"]);
        assert_eq!(diff(&repo, "t-never-existed").expect("reads"), None);
        let _ = std::fs::remove_dir_all(&repo);
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
