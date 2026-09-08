//! `ostraka run` — one task through the whole pipeline.

use crate::workspace::Workspace;
use ostraka_core::identity::ActorId;
use ostraka_core::task::TaskSpec;
use ostraka_runtime::orchestrator::{Places, RunReport};
use ostraka_runtime::progress::Watcher;
use ostraka_runtime::{orchestrator, route};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Which run of this process the next one is.
static NEXT_RUN: AtomicUsize = AtomicUsize::new(1);

/// The identity a run is attributed to when nobody names one.
pub const AUTHOR: &str = "author";
/// The identity that reviews it when nobody names one.
pub const REVIEWER: &str = "reviewer";
/// What a run branches from when nobody names anything.
pub const BASE_REF: &str = "HEAD";

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
    pub model: Option<String>,
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
            model: None,
        }
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
) -> Result<RunReport, Box<dyn std::error::Error>> {
    let repo = workspace.repository(args.repository.as_deref())?;
    let config = workspace.config_for(&repo)?;
    config.validate()?;
    let profiles = workspace.profiles()?;

    // Vendors that can only be isolated by relocating their home directory get
    // one here, beside the run records and ignored by git for the same reason.
    let vendor_home = workspace.ostraka().join("vendor-home");
    let routing = route::select(
        &profiles,
        args.adapter.as_deref(),
        args.review_adapter.as_deref(),
        &vendor_home,
        config
            .policy
            .timeout_secs
            .map(std::time::Duration::from_secs),
    )?;

    let task = TaskSpec {
        // The process, and which run of it. Two runs sharing a task id collide
        // in the records directory and in every branch name derived from it,
        // and both halves are needed to rule that out: the process id alone
        // repeats now that a browser starts several runs without restarting,
        // and the clock alone repeats because it counts in seconds and two
        // runs fit comfortably inside one. Caught by a flow test that started
        // two runs and found one record.
        id: format!(
            "t{}-{}",
            std::process::id(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ),
        prompt: args.prompt.clone(),
        adapter: routing_author_id(&routing),
        author: ActorId::new(&args.author),
        base_ref: args.base_ref.clone(),
        model: args.model.clone(),
    };

    let worktrees = workspace.worktrees(&config);
    let notes = workspace.notes_if_present();
    let records = workspace.records();
    let places = Places {
        repo: &repo.path,
        worktrees: &worktrees,
        records: &records,
        name: &repo.name,
        notes: notes.as_deref(),
    };

    Ok(orchestrator::run_task(
        &places,
        &config,
        &routing,
        &task,
        &ActorId::new(&args.reviewer),
        watcher,
    )?)
}

pub fn run(workspace: &Workspace, args: &Args, json: bool) -> Outcome {
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

    let report = execute(workspace, args, None)?;

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
            (None, Some(refusal)) => println!("refused — {}", describe(refusal)),
            (None, None) => println!("refused"),
        }
        println!("record: {}", workspace.records().join("runs").display());
    }

    Ok(report.approved())
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
