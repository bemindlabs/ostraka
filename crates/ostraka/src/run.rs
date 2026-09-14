//! `ostraka run` — one task through the whole pipeline.

use crate::workspace::Workspace;
use ostraka_core::identity::ActorId;
use ostraka_core::task::TaskSpec;
use ostraka_runtime::index::{self, RunSummary};
use ostraka_runtime::orchestrator::{Places, RunReport};
use ostraka_runtime::progress::Watcher;
use ostraka_runtime::{orchestrator, route};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Which run of this process the next one is.
static NEXT_RUN: AtomicUsize = AtomicUsize::new(1);

/// The identity a run is attributed to when nobody names one.
pub const AUTHOR: &str = "author";
/// The identity that reviews it when nobody names one.
pub const REVIEWER: &str = "reviewer";
/// What a run branches from when nobody names anything.
pub const BASE_REF: &str = "HEAD";
/// How many attempts a loop gets when nobody says.
pub const ATTEMPTS: usize = 3;

/// The process, and which run of it. Two runs sharing a task id collide in the
/// records directory and in every branch name derived from it, and both halves
/// are needed to rule that out: the process id alone repeats now that a browser
/// starts several runs without restarting, and the clock alone repeats because
/// it counts in seconds and two runs fit comfortably inside one. Caught by a
/// flow test that started two runs and found one record.
pub fn task_id() -> String {
    format!(
        "t{}-{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone)]
pub struct Args {
    pub prompt: String,
    /// Which repository the change is made in. `None` where the workspace
    /// holds one and there is nothing to choose between.
    pub repository: Option<String>,
    pub author: String,
    pub reviewer: String,
    pub adapter: Option<String>,
    pub review_adapter: Option<String>,
    pub base_ref: String,
    /// A finished run to continue, by run id. Resolves to the branch that run's
    /// commit landed on, and to the repository it was made in.
    pub from: Option<String>,
    pub model: Option<String>,
    /// How many attempts a refused run gets. One is a run; more is a loop.
    pub attempts: usize,
}

impl Args {
    /// A run with nothing chosen but the task.
    ///
    /// The same defaults the command line's flags carry, taken from the same
    /// three constants, so a run started by typing a task into the browser is
    /// the run `ostraka run "..."` starts.
    pub fn for_task(prompt: String) -> Self {
        Self {
            prompt,
            repository: None,
            author: AUTHOR.to_string(),
            reviewer: REVIEWER.to_string(),
            adapter: None,
            review_adapter: None,
            base_ref: BASE_REF.to_string(),
            from: None,
            model: None,
            attempts: 1,
        }
    }
}

/// The run `--from` names, once it is known to be one worth continuing.
///
/// Two refusals, and both are the browser's rule written down where the command
/// line can be held to it. A thread advances only where a run was approved: a
/// change the gate would not take is not a base to build on, and continuing
/// from one would be a way of taking it after all.
///
/// The second refusal is the one that would not announce itself. A refused run
/// *has* a branch — created before the agent started — whose head is the commit
/// it branched from. `--base-ref ostraka/<refused>` therefore succeeds and
/// silently starts from somewhere else's work, which is the failure this exists
/// to make impossible to reach by accident.
fn finished_run(
    workspace: &Workspace,
    run_id: &str,
) -> Result<RunSummary, Box<dyn std::error::Error>> {
    let runs = index::list(&workspace.records())?;
    let Some(run) = runs.into_iter().find(|r| r.run_id == run_id) else {
        return Err(format!("no run {run_id:?} is recorded in this workspace").into());
    };
    if !run.approved() {
        let said = match &run.outcome {
            Some(outcome) => format!("{outcome:?}").to_lowercase(),
            None => "never finished".to_string(),
        };
        return Err(format!(
            "run {run_id} was {said}, so there is nothing to continue from — \
             a change the gate would not take is not a base to build on"
        )
        .into());
    }
    Ok(run)
}

/// The ref a continued run branches from.
///
/// Separate from [`finished_run`] because it needs the repository, and the
/// repository is what that function is consulted to find.
///
/// It is also the second of two independent guards, which was measured rather
/// than assumed: with the approval check removed, a refused run still cannot be
/// continued, because it never made a commit for a branch head to carry. The
/// message here can say "approved" because nothing reaches it that was not.
fn continue_from(repo: &Path, run_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    match index::commit_branch(repo, run_id)? {
        Some(branch) => Ok(branch),
        None => Err(format!(
            "run {run_id} was approved but its commit is not on a branch here; \
             the branch it was on has been deleted"
        )
        .into()),
    }
}

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// Everything `ostraka run` does except say so.
///
/// The command prints and the browser draws, and both arrive here: a run
/// started by pressing a key is the same run started from a shell — the same
/// routing, the same gate, the same record. Two paths to one pipeline would be
/// two pipelines within a release.
///
/// `watcher` is handed to the orchestrator and can only be told things. A
/// caller with a screen passes one; the command passes `None`.
///
/// The interrupt flag belongs to the caller. It is global because a signal is
/// global, so a caller that starts more than one run in a process clears it
/// between them — and clears it *before* starting the next, not from inside
/// it: a stop asked for in the same breath as a run would otherwise be wiped
/// by the run clearing the flag a moment later, and the run nobody wanted
/// would carry on.
pub fn execute(
    workspace: &Workspace,
    args: &Args,
    watcher: Option<Box<dyn Watcher>>,
    stop: &ostraka_adapter::interrupt::Stop,
) -> Result<RunReport, Box<dyn std::error::Error>> {
    // `--from` names a run, and a run is not a ref. The branch a run's commit
    // lands on is `ostraka/<id>` — internal knowledge that was reachable only
    // by reading the source, so continuing a piece of work meant knowing a
    // naming convention nobody had been told.
    let continued = args.from.as_deref().map(|id| finished_run(workspace, id));
    let continued = continued.transpose()?;

    // A run belongs to a repository. Continuing it in a different one would
    // branch from a ref that repository has never heard of.
    let named = continued
        .as_ref()
        .map(|r| r.repository.clone())
        .or_else(|| args.repository.clone());
    if let (Some(run), Some(asked)) = (continued.as_ref(), args.repository.as_deref()) {
        if run.repository != asked {
            return Err(format!(
                "run {} was made in {:?}, not in {asked:?}",
                run.run_id, run.repository
            )
            .into());
        }
    }

    let repo = workspace.repository(named.as_deref())?;
    let config = workspace.config_for(&repo)?;
    config.validate()?;
    let profiles = workspace.profiles()?;

    // Vendors that can only be isolated by relocating their home directory get
    // one here, beside the run records and ignored by git for the same reason.
    let vendor_home = workspace.ostraka().join("vendor-home");
    let routing = route::select_until(
        &profiles,
        args.adapter.as_deref(),
        args.review_adapter.as_deref(),
        &vendor_home,
        config
            .policy
            .timeout_secs
            .map(std::time::Duration::from_secs),
        stop,
    );
    // A routing failure is the moment somebody most needs to know that a CLI
    // this binary can drive is installed and simply has no profile here. The
    // error already says what it checked; what it could not say is what it
    // never looked at.
    let routing = match routing {
        Ok(routing) => routing,
        Err(e) => {
            let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
            return Err(Box::new(crate::discover::NoAdapter {
                said: e.to_string(),
                found: crate::discover::unconfigured(&ids),
            }));
        }
    };

    let task = TaskSpec {
        id: task_id(),
        prompt: args.prompt.clone(),
        adapter: routing_author_id(&routing),
        author: ActorId::new(&args.author),
        base_ref: match continued.as_ref() {
            Some(run) => continue_from(&repo.path, &run.run_id)?,
            None => args.base_ref.clone(),
        },
        model: args.model.clone(),
    };

    let worktrees = workspace.worktrees(&config);
    let notes = workspace.notes_if_present();
    let skills = workspace.skills_if_present();
    let records = workspace.records();
    let places = Places {
        repo: &repo.path,
        worktrees: &worktrees,
        records: &records,
        name: &repo.name,
        notes: notes.as_deref(),
        skills: skills.as_deref(),
    };

    Ok(orchestrator::run_task_until(
        &places,
        &config,
        &routing,
        &task,
        &ActorId::new(&args.reviewer),
        watcher,
        stop,
    )?)
}

/// Refusals a loop tries again after: the ones that are about the change.
///
/// A failed check, a sent-back change and no change at all are all things a
/// second attempt can do better. A vendor that could not start, a clock that ran
/// out, an operator who stopped it and a worktree that could not be prepared are
/// not about the change, and trying again would spend another attempt on the
/// same wall. A policy violation is never retried: it means something wrote
/// where it had no business writing, and a loop that tried again would be
/// asking it to find another way in.
pub fn worth_retrying(refusal: &ostraka_runtime::gate::Refusal) -> bool {
    use ostraka_runtime::gate::Refusal;
    matches!(
        refusal,
        Refusal::ChecksFailed { .. } | Refusal::Rejected { .. } | Refusal::NoChange
    )
}

/// How much of a failing check's output the next attempt is shown. The end of
/// it, which is where test runners and compilers say what went wrong.
const FEEDBACK_TAIL: usize = 1500;

/// The task a loop's next attempt is given: the task, and why the last attempt
/// was not kept.
///
/// This is the one place an author learns a verdict exists, which the author
/// prompt otherwise keeps from it on purpose. It is told the reason, never the
/// marker: the marker is derived after authoring, per run, so nothing written
/// here can match the next one.
pub fn feedback(task: &str, refusal: &ostraka_runtime::gate::Refusal) -> String {
    use ostraka_runtime::gate::Refusal;
    let mut said = format!("{task}\n\n----- the last attempt at this was not kept -----\n\n");
    match refusal {
        Refusal::ChecksFailed { failed, records } => {
            said.push_str(&format!(
                "The project's checks failed: {}.\n",
                failed.join(", ")
            ));
            for record in records.iter().filter(|r| !r.passed()) {
                let output = format!("{}{}", record.stdout, record.stderr);
                let mut from = output.len().saturating_sub(FEEDBACK_TAIL);
                while !output.is_char_boundary(from) {
                    from += 1;
                }
                said.push_str(&format!(
                    "\n`{}` ended with:\n{}\n",
                    record.cmd,
                    output[from..].trim_end()
                ));
            }
        }
        Refusal::Rejected { reason } => {
            said.push_str(&format!("It was sent back, and this is why: {reason}\n"));
        }
        Refusal::NoChange => {
            said.push_str("It finished without changing any file, so there was nothing to keep.\n");
        }
        other => said.push_str(&format!("{}\n", describe(other))),
    }
    said.push_str(
        "\nNothing from that attempt is in this worktree. Start from the task again, \
         and deal with what went wrong.\n",
    );
    said
}

/// [`execute`], again, while the refusal is one worth another attempt and
/// attempts remain.
///
/// Every attempt is a whole run: its own worktree, its own gate, its own review
/// and its own record. Nothing is relaxed to get a loop — a refused attempt has
/// no commit, so the next one branches from where the first did, and the only
/// thing carried across is the reason. `watcher` is asked for a fresh watcher
/// per attempt, and `again` is told before each retry which attempt it is and
/// what the last one was refused for.
pub fn execute_looping(
    workspace: &Workspace,
    args: &Args,
    mut watcher: impl FnMut() -> Option<Box<dyn Watcher>>,
    stop: &ostraka_adapter::interrupt::Stop,
    mut again: impl FnMut(usize, &ostraka_runtime::gate::Refusal),
) -> Result<RunReport, Box<dyn std::error::Error>> {
    let mut attempt = args.clone();
    let mut report = execute(workspace, &attempt, watcher(), stop)?;
    for n in 2..=args.attempts.max(1) {
        let Some(refusal) = report.refusal.as_ref().filter(|r| worth_retrying(r)) else {
            break;
        };
        if stop.requested() {
            break;
        }
        again(n, refusal);
        attempt.prompt = feedback(&args.prompt, refusal);
        report = execute(workspace, &attempt, watcher(), stop)?;
    }
    Ok(report)
}

pub fn run(workspace: &Workspace, args: &Args, json: bool) -> Outcome {
    run_with(workspace, args, json, |_| {})
}

/// A run from the command line: as many attempts as it was given, each one
/// announced on stderr before it starts.
fn attempted(workspace: &Workspace, args: &Args) -> Result<RunReport, Box<dyn std::error::Error>> {
    execute_looping(
        workspace,
        args,
        || None,
        &ostraka_adapter::interrupt::Stop::new(),
        |n, refusal| {
            eprintln!(
                "attempt {n} of {}: the last one was not kept \u{2014} {}",
                args.attempts,
                describe(refusal)
            );
        },
    )
}

/// Everything `run` does, with a look at the report on the way past.
fn run_with(
    workspace: &Workspace,
    args: &Args,
    json: bool,
    seen: impl FnOnce(&crate::run::RunReport),
) -> Outcome {
    // One Ctrl-C asks the run to stop; the adapters notice within a poll and
    // kill what they launched. A second is the operator saying they meant it,
    // and the default handler takes over.
    let interrupts = std::sync::atomic::AtomicUsize::new(0);
    let _ = ctrlc::set_handler(move || {
        if interrupts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            eprintln!("\nstopping the run — press Ctrl-C again to give up on it");
            ostraka_adapter::interrupt::request();
        } else {
            std::process::exit(130);
        }
    });

    // The command line's own step, and not the run's: `execute` is shared with
    // the browser, which has a selector of its own and no stdin anybody is
    // typing into. Where a run failed for want of a profile and one is
    // installed, the operator is asked; accepting writes it and the run is
    // tried once more, on a workspace that now declares what it uses.
    let report = match attempted(workspace, args) {
        Ok(report) => report,
        Err(e) => {
            let Some(problem) = e.downcast_ref::<crate::discover::NoAdapter>() else {
                return Err(e);
            };
            let choice = crate::offer::profiles(
                workspace,
                problem,
                &mut std::io::stdin().lock(),
                &mut std::io::stderr(),
                !json && crate::offer::at_a_terminal(),
            )?;
            match choice {
                crate::offer::Choice::Wrote => attempted(workspace, args)?,
                // Once. A second failure is the answer, not another question.
                crate::offer::Choice::Declined | crate::offer::Choice::NotAsked => return Err(e),
            }
        }
    };

    if json {
        let out = serde_json::json!({
            "run_id": report.record.run_id,
            "approved": report.approved(),
            "outcome": report.record.outcome,
            "checks": report.record.checks.iter().map(|c| serde_json::json!({
                "name": c.name,
                "passed": c.passed(),
                "exit_code": c.exit_code,
                "duration_ms": c.duration_ms,
            })).collect::<Vec<_>>(),
            "refusal": report.refusal.as_ref().map(|r| format!("{r:?}")),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("run {}", report.record.run_id);
        for c in &report.record.checks {
            let mark = if c.passed() { "pass" } else { "FAIL" };
            println!("  {mark}  {:<8} {}ms", c.name, c.duration_ms);
        }
        match (&report.token, &report.refusal) {
            (Some(token), _) => println!(
                "approved — written by {}, reviewed by {}",
                token.author(),
                token.reviewer()
            ),
            (None, Some(refusal)) => println!("rejected — {}", describe(refusal)),
            (None, None) => println!("rejected"),
        }
        println!("record: {}", workspace.records().join("runs").display());
    }

    seen(&report);
    Ok(report.approved())
}

/// The run, and which run it was.
///
/// `run` answers the command line's question — did it pass — and the task list
/// needs the other half: which record to write against the task it took. Read
/// from the report rather than looked up afterwards, because "the newest run"
/// stops being "the run I just started" the moment two of them are going.
pub fn run_reporting(
    workspace: &Workspace,
    args: &Args,
    json: bool,
) -> Result<(bool, String), Box<dyn std::error::Error>> {
    // A plain local. `seen` is `FnOnce`, called synchronously on this thread
    // before `run_with` returns, so there is nothing for a lock to make safe —
    // and the `Arc<Mutex<_>>` this replaces could panic on a poisoned mutex in
    // the one path whose whole job is to report what happened.
    let mut id = String::new();
    let approved = run_with(workspace, args, json, |report| {
        id = report.record.run_id.clone();
    })?;
    Ok((approved, id))
}

fn routing_author_id(routing: &route::Routing) -> String {
    ostraka_adapter::VendorAdapter::id(routing.author.as_ref()).to_string()
}

pub fn describe(refusal: &ostraka_runtime::gate::Refusal) -> String {
    use ostraka_runtime::gate::Refusal;
    match refusal {
        Refusal::ChecksFailed { failed, .. } => {
            format!("checks failed: {}", failed.join(", "))
        }
        Refusal::Rejected { reason } => format!("reviewer rejected: {reason}"),
        Refusal::AuthorFailed { code, diagnostics } => match diagnostics {
            Some(d) => format!("the author could not run (exit {code}): {d}"),
            None => format!("the author could not run (exit {code}), and said nothing"),
        },
        Refusal::PolicyViolation { reason } => reason.clone(),
        Refusal::SetupFailed { step, reason } => {
            format!("the worktree could not be prepared ({step}): {reason}")
        }
        Refusal::Interrupted => "stopped by the operator".to_string(),
        Refusal::TimedOut { after_secs } => {
            format!("the author was still running after {after_secs}s and was stopped")
        }
        Refusal::NoChange => {
            "the author ran cleanly and changed nothing; there is nothing to review".to_string()
        }
        Refusal::SelfApproval { actor } => {
            format!("{actor} cannot approve a change {actor} wrote")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::gate::CheckRecord;
    use ostraka_runtime::gate::Refusal;

    #[test]
    fn only_what_is_about_the_change_is_tried_again() {
        assert!(worth_retrying(&Refusal::NoChange));
        assert!(worth_retrying(&Refusal::Rejected {
            reason: "no tests".into()
        }));
        assert!(!worth_retrying(&Refusal::Interrupted));
        assert!(!worth_retrying(&Refusal::TimedOut { after_secs: 60 }));
        assert!(!worth_retrying(&Refusal::PolicyViolation {
            reason: "wrote outside src".into()
        }));
        assert!(!worth_retrying(&Refusal::AuthorFailed {
            code: "1".into(),
            diagnostics: None
        }));
    }

    #[test]
    fn the_next_attempt_is_told_the_task_and_why_the_last_was_not_kept() {
        let failing = CheckRecord {
            name: "test".into(),
            cmd: "cargo test".into(),
            exit_code: Some(101),
            stdout: format!("{}assertion failed: left == right", "x".repeat(4000)),
            stderr: String::new(),
            duration_ms: 3,
        };
        let said = feedback(
            "add a flag",
            &Refusal::ChecksFailed {
                failed: vec!["test".into()],
                records: vec![failing],
            },
        );
        assert!(said.starts_with("add a flag"));
        assert!(said.contains("`cargo test` ended with"));
        assert!(said.contains("assertion failed: left == right"));
        assert!(
            said.len() < 2500,
            "the whole output was pasted: {}",
            said.len()
        );

        let sent_back = feedback(
            "add a flag",
            &Refusal::Rejected {
                reason: "it has no test".into(),
            },
        );
        assert!(sent_back.contains("it has no test"));
        assert!(!sent_back.contains("VERDICT"));
    }
}
