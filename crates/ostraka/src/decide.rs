//! `ostraka decide` — what a person says about a run a reviewer handed them.
//!
//! A reviewer may decline to judge: the task is ambiguous, or the change is
//! defensible but consequential, or judging it needs something the reviewer was
//! not given. The run is recorded refused, because nothing may merge on a
//! review that did not judge, and an escalation is written beside it saying
//! what a person has to decide.
//!
//! This is that person answering. The answer is an approval like any other —
//! the gate has always asked *who* approved, not what kind of thing they are —
//! so promoting on it still needs every required check to have passed, and
//! still needs the approver to differ from the author.

use crate::workspace::Workspace;
use ostraka_runtime::record::{Decision, read_decision, read_escalation, write_decision};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// Who is deciding, when nobody said.
///
/// The login name, because that is the only thing here that is about a person
/// rather than about a machine, and `--as` is how somebody says otherwise. Not
/// a claim about identity: a decision is a record of what was done on this
/// machine, and the machine's own account of who did it is what it has.
fn operator() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "operator".to_string())
}

pub fn run(
    workspace: &Workspace,
    run_id: &str,
    approve: bool,
    reason: Option<&str>,
    who: Option<&str>,
    json: bool,
) -> Outcome {
    let run_dir = workspace.records().join("runs").join(run_id);
    if !run_dir.is_dir() {
        return Err(format!("no run {run_id:?} in {}", workspace.records().display()).into());
    }

    // Only an escalated run is waiting on anybody. Deciding about a run that
    // was judged would be overruling a review, which is a different act and
    // not one this command performs: the gate reads a decision only where a
    // reviewer asked for one.
    let Some(escalation) = read_escalation(&run_dir) else {
        return Err(format!(
            "run {run_id:?} was not handed to anybody \u{2014} nothing here is waiting on a \
             decision"
        )
        .into());
    };

    if let Some(already) = read_decision(&run_dir) {
        return Err(format!(
            "{} already decided this run: {} \u{2014} a decision is not taken back, and a \
             change judged again is a new run",
            already.by,
            if already.approved {
                "approved"
            } else {
                "refused"
            }
        )
        .into());
    }

    let by = who.map(str::to_string).unwrap_or_else(operator);
    let decision = Decision {
        by: by.clone(),
        approved: approve,
        reason: reason.map(str::to_string),
        at: ostraka_core::clock::now_rfc3339(),
    };
    write_decision(&run_dir, &decision)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "run_id": run_id,
                "by": decision.by,
                "approved": decision.approved,
                "reason": decision.reason,
                "at": decision.at,
                "asked": escalation.said,
            }))?
        );
        return Ok(true);
    }

    println!("{} was asked: {}", run_id, escalation.said);
    match approve {
        true => {
            println!("{by} approved it");
            println!("\nNothing is merged. To take it further:");
            println!("  ostraka promote {run_id}");
        }
        false => println!("{by} refused it \u{2014} nothing will promote"),
    }
    Ok(true)
}
