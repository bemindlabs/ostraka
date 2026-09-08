//! `ostraka adapters` — what can actually run on this machine.

use crate::workspace::Workspace;
use ostraka_adapter::process::ProcessAdapter;
use ostraka_adapter::{Availability, VendorAdapter};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &Workspace, json: bool) -> Outcome {
    let mut rows: Vec<(String, Availability)> = workspace
        .profiles()?
        .into_iter()
        .map(|p| {
            let adapter = ProcessAdapter::new(p);
            let availability = adapter.probe();
            (adapter.id().to_string(), availability)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(id, a)| {
                serde_json::json!({
                    "id": id,
                    "ready": a.is_ready(),
                    "detail": describe(a),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for (id, a) in &rows {
            let mark = if a.is_ready() { "ready" } else { "unavailable" };
            println!("{id:<16} {mark:<12} {}", describe(a));
        }
    }

    // Reporting an unavailable adapter is a successful report, not a failure.
    Ok(true)
}

fn describe(a: &Availability) -> String {
    match a {
        Availability::Ready { version } => version.clone().unwrap_or_default(),
        Availability::NotFound { command } => format!("{command} not found on PATH"),
        Availability::Unusable { reason } => reason.clone(),
    }
}
