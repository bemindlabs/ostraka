//! `ostraka runs` — what this project has recorded.
//!
//! The same listing the browser shows, for the places a browser cannot go: a
//! pipe, a CI log, a machine reading `--json`.

use ostraka_runtime::index;
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, json: bool) -> Outcome {
    let runs = index::list(&project.join(".ostraka"))?;

    if json {
        let rows: Vec<_> = runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_id": r.run_id,
                    "started_at": r.started_at,
                    "prompt": r.prompt,
                    "author": r.author.to_string(),
                    "adapter": r.adapter,
                    "reviewer": r.reviewer.as_ref().map(ToString::to_string),
                    "outcome": r.outcome,
                    "checks_passed": r.checks_passed,
                    "checks_total": r.checks_total,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else if runs.is_empty() {
        println!("no runs recorded yet");
    } else {
        for r in &runs {
            let word = match r.outcome {
                Some(ostraka_core::record::Outcome::Approved) => "approved",
                Some(ostraka_core::record::Outcome::Rejected) => "refused",
                Some(ostraka_core::record::Outcome::Failed) => "failed",
                None => "unfinished",
            };
            println!("{:<10}  {:<28}  {}", word, r.run_id, first_line(&r.prompt));
        }
    }

    Ok(true)
}

fn first_line(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        return "(recorded before runs kept the task text)".to_string();
    }
    let line = prompt.lines().next().unwrap_or_default();
    if line.chars().count() > 72 {
        format!("{}…", line.chars().take(71).collect::<String>())
    } else {
        line.to_string()
    }
}
