//! `ostraka run` — one task through the whole pipeline.

use crate::project;
use ostraka_core::identity::ActorId;
use ostraka_core::task::TaskSpec;
use ostraka_runtime::{orchestrator, route};
use std::path::{Path, PathBuf};

pub struct Args {
    pub prompt: String,
    pub author: String,
    pub reviewer: String,
    pub adapter: Option<String>,
    pub review_adapter: Option<String>,
    pub base_ref: String,
    pub model: Option<String>,
}

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, args: &Args, json: bool) -> Outcome {
    let config = project::load_config(project)?;
    config.validate()?;
    let profiles = project::load_profiles(project)?;

    // Vendors that can only be isolated by relocating their home directory get
    // one here, beside the run records and ignored by git for the same reason.
    let records_root: PathBuf = project.join(".ostraka");
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

    let routing = route::select(
        &profiles,
        args.adapter.as_deref(),
        args.review_adapter.as_deref(),
        &records_root.join("vendor-home"),
        config
            .policy
            .timeout_secs
            .map(std::time::Duration::from_secs),
    )?;

    let task = TaskSpec {
        id: format!("t{}", std::process::id()),
        prompt: args.prompt.clone(),
        adapter: routing_author_id(&routing),
        author: ActorId::new(&args.author),
        base_ref: args.base_ref.clone(),
        model: args.model.clone(),
    };

    let report = orchestrator::run_task(
        project,
        &config,
        &routing,
        &task,
        &ActorId::new(&args.reviewer),
        &records_root,
    )?;

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
        println!("record: {}", records_root.join("runs").display());
    }

    Ok(report.approved())
}

fn routing_author_id(routing: &route::Routing) -> String {
    ostraka_adapter::VendorAdapter::id(routing.author.as_ref()).to_string()
}

fn describe(refusal: &ostraka_runtime::gate::Refusal) -> String {
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
