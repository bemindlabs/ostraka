//! `ostraka adapters` — what can actually run on this machine.

use crate::discover;
use crate::workspace::Workspace;
use ostraka_adapter::process::ProcessAdapter;
use ostraka_adapter::{Availability, VendorAdapter};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &Workspace, json: bool) -> Outcome {
    // A workspace with no `adapters/` reports that it has no `adapters/`, which
    // whoever is standing there can already see. What they cannot see is that
    // CLIs this binary knows how to drive are installed and one command away,
    // so an empty listing is answered rather than refused.
    let configured = workspace.profiles().unwrap_or_default();
    let mut rows: Vec<(String, Availability)> = configured
        .clone()
        .into_iter()
        .map(|p| {
            let adapter = ProcessAdapter::new(p);
            let availability = adapter.probe();
            (adapter.id().to_string(), availability)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    // What this binary could write a profile for and this workspace has not.
    // Probed, because "installed" is the only part of it worth saying.
    let ids: Vec<String> = configured.iter().map(|p| p.id.clone()).collect();
    let found = discover::unconfigured(&ids);

    if json {
        let out = serde_json::json!({
            "configured": rows
                .iter()
                .map(|(id, a)| serde_json::json!({
                    "id": id,
                    "ready": a.is_ready(),
                    "detail": describe(a),
                }))
                .collect::<Vec<_>>(),
            "available": found
                .iter()
                .filter(|f| f.ready())
                .map(|f| serde_json::json!({
                    "id": f.id,
                    "detail": describe(&f.availability),
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for (id, a) in &rows {
            let mark = if a.is_ready() { "ready" } else { "unavailable" };
            println!("{id:<16} {mark:<12} {}", describe(a));
        }
        if rows.is_empty() {
            println!("no adapter profiles are configured here");
        }
        // Only the ones that answered. Listing an absent CLI as a possibility
        // would send somebody to `init` for a profile they cannot run.
        for f in found.iter().filter(|f| f.ready()) {
            println!(
                "{:<16} {:<12} {}",
                f.id,
                "available",
                describe(&f.availability)
            );
        }
        if let Some(said) = discover::suggestion(&found) {
            println!();
            println!("{said}");
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
