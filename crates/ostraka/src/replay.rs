//! `ostraka replay` — read a finished run back.

use ostraka_runtime::orchestrator;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &crate::workspace::Workspace, run_id: &str, json: bool) -> Outcome {
    let records_root = workspace.records();
    let (record, events) = orchestrator::replay(&records_root, run_id)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "record": record,
                "events": events,
            }))?
        );
    } else {
        println!("run {}  ({})", record.run_id, record.started_at);
        println!(
            "task {}  by {}  via {}",
            record.task_id, record.author, record.adapter
        );
        for c in &record.checks {
            let mark = if c.passed() { "pass" } else { "FAIL" };
            println!("  {mark}  {}", c.name);
        }
        if let Some(a) = &record.approval {
            println!("  reviewed by {}: {:?}", a.reviewer, a.verdict);
        }
        // Not `{:?}`. That printed `Some(Rejected)` — a Rust value, in a
        // command whose reader is a person, spelled differently from the JSON.
        println!(
            "  outcome: {}",
            record.outcome.map_or("unfinished", |o| o.as_str())
        );
        println!("{} event(s)", events.len());
    }

    Ok(true)
}
