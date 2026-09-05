//! `ostraka adapters` — what can actually run on this machine.

use ostraka_adapter::process::ProcessAdapter;
use ostraka_adapter::{Availability, Profile, VendorAdapter};
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, json: bool) -> Outcome {
    let dir = project.join("adapters");
    if !dir.is_dir() {
        return Err(format!("no adapters directory at {}", dir.display()).into());
    }

    let mut rows = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let profile = Profile::parse(&std::fs::read_to_string(&path)?)?;
        let adapter = ProcessAdapter::new(profile);
        let availability = adapter.probe();
        rows.push((adapter.id().to_string(), availability));
    }

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
