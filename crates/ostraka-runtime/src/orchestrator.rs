//! One task, start to finish.
//!
//! Isolate, execute, gate, review, record. The orchestrator drives every step
//! and can approve none of them: the only thing that ends a run favourably is a
//! [`crate::gate::MergeToken`], which this module cannot construct.

use crate::gate::{self, MergeToken, Refusal};
use crate::record::RunLog;
use crate::review;
use crate::route::Routing;
use crate::worktree;
use crate::{Error, Result};
use ostraka_adapter::VendorAdapter;
use ostraka_core::clock::now_rfc3339;
use ostraka_core::config::Config;
use ostraka_core::gate::{Approval, Verdict};
use ostraka_core::identity::ActorId;
use ostraka_core::record::{Event, Outcome, RunRecord};
use ostraka_core::task::TaskSpec;
use std::path::Path;

/// What a finished run produced.
pub struct RunReport {
    pub record: RunRecord,
    /// Present only when the gate minted one. Its absence is the refusal.
    pub token: Option<MergeToken>,
    pub refusal: Option<Refusal>,
    pub diff: String,
}

impl RunReport {
    pub fn approved(&self) -> bool {
        self.token.is_some()
    }
}

/// Runs one task through the whole pipeline.
///
/// `records_root` is where run logs are written — `.ostraka/` by convention.
/// The worktree is left in place on completion so the diff can be inspected;
/// removing it is the caller's decision, not this function's.
pub fn run_task(
    repo: &Path,
    config: &Config,
    routing: &Routing,
    task: &TaskSpec,
    reviewer_identity: &ActorId,
    records_root: &Path,
) -> Result<RunReport> {
    let run_id = format!("{}-{}", task.id, now_rfc3339().replace([':', '-'], ""));
    let mut log = RunLog::create(records_root, &run_id)?;

    let mut record = RunRecord {
        run_id: run_id.clone(),
        task_id: task.id.clone(),
        author: task.author.clone(),
        adapter: routing.author.id().to_string(),
        started_at: now_rfc3339(),
        finished_at: None,
        checks: Vec::new(),
        approval: None,
        outcome: None,
    };

    // 1. Isolate. Work is a diff on disk before it is anything else.
    let wt = worktree::create(
        repo,
        &repo.join(&config.worktree.base),
        &run_id,
        &task.base_ref,
    )?;

    // 2. Execute, streaming events into the log as they arrive so an
    //    interrupted run still leaves an account of how far it got.
    let author_exit = drive(routing.author.as_ref(), task, wt.path(), &mut log)?;

    // 3. Read what was actually touched, from git rather than from the agent.
    let touched = worktree::touched_paths(wt.path())?;
    log.append(&Event::Finished {
        exit_code: author_exit,
        files_touched: touched.clone(),
    })?;

    if !config.policy.permits(&touched) {
        return finish(
            log,
            record,
            Outcome::Rejected,
            None,
            Some(Refusal::Rejected {
                reason: format!("policy forbids writing outside declared paths: {touched:?}"),
            }),
            String::new(),
        );
    }

    // 4. Gate. These commands actually run; their output is captured.
    let passed = match gate::run_checks(&config.gate, wt.path()) {
        Ok(p) => p,
        Err(refusal) => {
            // Keep the evidence. A failed run is the one someone will need to
            // read afterwards, so the records travel from the refusal into the
            // record before anything returns.
            if let Refusal::ChecksFailed { records, .. } = &refusal {
                record.checks.clone_from(records);
            }
            return finish(
                log,
                record,
                Outcome::Rejected,
                None,
                Some(refusal),
                String::new(),
            );
        }
    };
    record.checks = passed.records().to_vec();

    // 5. Review, by an adapter that is not the one that wrote the change.
    let diff = worktree::diff(wt.path())?;
    let verdict = collect_verdict(routing.reviewer.as_ref(), task, &diff, wt.path(), &mut log)?;
    let approval = Approval {
        reviewer: reviewer_identity.clone(),
        verdict: verdict.clone(),
    };
    record.approval = Some(approval.clone());

    // 6. The gate decides. Nothing above this line can mint a token.
    match gate::evaluate(
        passed,
        &task.author,
        &approval,
        config.gate.review.must_differ_from_author,
    ) {
        Ok(token) => {
            // The trailers are the audit trail in the place it survives longest:
            // a commit outlives the run directory it came from.
            let message = format!(
                "{}\n\nRun: {run_id}\nAuthored-by: {} ({})\nReviewed-by: {} ({})",
                task.prompt,
                task.author,
                routing.author.id(),
                reviewer_identity,
                routing.reviewer.id(),
            );
            worktree::commit(wt.path(), &message, &task.author)?;
            finish(log, record, Outcome::Approved, Some(token), None, diff)
        }
        Err(refusal) => finish(log, record, Outcome::Rejected, None, Some(refusal), diff),
    }
}

/// Runs an adapter to completion, logging every event.
fn drive(
    adapter: &dyn VendorAdapter,
    task: &TaskSpec,
    worktree: &Path,
    log: &mut RunLog,
) -> Result<Option<i32>> {
    let mut session = adapter.launch(task, worktree)?;
    while let Some(event) = session.next_event() {
        log.append(&event)?;
    }
    Ok(session.finish().exit_code)
}

/// Runs the reviewer and reads its answer.
///
/// Fail-safe throughout: a reviewer that cannot be launched, or that says
/// nothing usable, has not approved anything.
fn collect_verdict(
    reviewer: &dyn VendorAdapter,
    task: &TaskSpec,
    diff: &str,
    worktree: &Path,
    log: &mut RunLog,
) -> Result<Verdict> {
    let review_task = TaskSpec {
        id: format!("{}-review", task.id),
        prompt: review::review_prompt(&task.prompt, diff),
        adapter: reviewer.id().to_string(),
        author: task.author.clone(),
        base_ref: task.base_ref.clone(),
        model: None,
    };

    let mut session = match reviewer.launch(&review_task, worktree) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Verdict::Reject {
                reason: format!("reviewer could not be launched: {e}"),
            });
        }
    };

    let mut spoken = String::new();
    while let Some(event) = session.next_event() {
        if let Event::Message { text, .. } = &event {
            spoken.push_str(text);
            spoken.push('\n');
        }
        log.append(&event)?;
    }
    let outcome = session.finish();
    if outcome.exit_code != Some(0) {
        return Ok(Verdict::Reject {
            reason: format!("reviewer exited with {:?}", outcome.exit_code),
        });
    }

    Ok(review::parse_verdict(&spoken))
}

fn finish(
    log: RunLog,
    mut record: RunRecord,
    outcome: Outcome,
    token: Option<MergeToken>,
    refusal: Option<Refusal>,
    diff: String,
) -> Result<RunReport> {
    record.finished_at = Some(now_rfc3339());
    record.outcome = Some(outcome);
    log.write_record(&record)?;
    Ok(RunReport {
        record,
        token,
        refusal,
        diff,
    })
}

/// Reads a previous run back for replay.
pub fn replay(records_root: &Path, run_id: &str) -> Result<(RunRecord, Vec<Event>)> {
    let dir = records_root.join("runs").join(run_id);
    let record_text = std::fs::read_to_string(dir.join("record.json"))
        .map_err(|e| Error::Other(format!("no run {run_id:?}: {e}")))?;
    let record: RunRecord = serde_json::from_str(&record_text)
        .map_err(|e| Error::Other(format!("run record is unreadable: {e}")))?;
    let events = crate::record::read_events(&dir)?;
    Ok((record, events))
}
