//! `ostraka replay` — read a finished run back.

use ostraka_runtime::orchestrator;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &crate::workspace::Workspace, run_id: &str, json: bool) -> Outcome {
    let records_root = workspace.records();
    let (record, events) = orchestrator::replay(&records_root, run_id)?;
    // Which agents took the seats. A run made before this was recorded has
    // none, and says so rather than having one guessed for it.
    let run_dir = records_root.join("runs").join(run_id);
    let provenance = ostraka_runtime::record::read_provenance(&run_dir);
    // What the review reported, whether it approved or not, and — where it
    // declined to judge — what it asked a person and what they said.
    let findings = ostraka_runtime::record::read_findings(&run_dir);
    let escalation = ostraka_runtime::record::read_escalation(&run_dir);
    let decision = ostraka_runtime::record::read_decision(&run_dir);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "record": record,
                "provenance": provenance,
                "findings": findings,
                "escalation": escalation,
                "decision": decision,
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
        for finding in &findings {
            println!(
                "  finding  {} [{}, {}] {}",
                finding.at, finding.severity, finding.disposition, finding.said
            );
        }
        if let Some(escalation) = &escalation {
            println!(
                "  escalated by {}: {}",
                escalation.reviewer, escalation.said
            );
            match &decision {
                Some(decision) => println!(
                    "  {} {} it{}",
                    decision.by,
                    if decision.approved {
                        "approved"
                    } else {
                        "refused"
                    },
                    decision
                        .reason
                        .as_deref()
                        .map(|why| format!(": {why}"))
                        .unwrap_or_default()
                ),
                None => println!("  waiting on a person \u{2014} `ostraka decide {run_id} …`"),
            }
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
