//! `ostraka replay` — read a finished run back.

use ostraka_runtime::orchestrator;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &crate::workspace::Workspace, run_id: &str, json: bool) -> Outcome {
    let records_root = workspace.records();
    let (record, events) = orchestrator::replay(&records_root, run_id)?;
    // Which agents took the seats. A run made before this was recorded has
    // none, and says so rather than having one guessed for it.
    let provenance =
        ostraka_runtime::record::read_provenance(&records_root.join("runs").join(run_id));

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "record": record,
                "provenance": provenance,
                "events": events,
            }))?
        );
    } else {
        println!("run {}  ({})", record.run_id, record.started_at);
        println!(
            "task {}  by {}  via {}",
            record.task_id, record.author, record.adapter
        );
        match &provenance {
            Some(p) => {
                for (seat, who) in [("author", &p.author), ("reviewer", &p.reviewer)] {
                    println!(
                        "  {seat:<8} {}  cli {}  model {}",
                        who.profile,
                        who.cli_trailer(),
                        who.model_trailer()
                    );
                }
            }
            None => println!("  no provenance recorded for this run"),
        }
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
