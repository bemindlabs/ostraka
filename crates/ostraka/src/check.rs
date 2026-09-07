//! `ostraka check` — validate configuration before anything runs.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, json: bool) -> Outcome {
    let config_path = project.join("ostraka.toml");
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;

    let config = Config::parse(&text)?;
    let mut problems: Vec<String> = Vec::new();
    if let Err(e) = config.validate() {
        problems.push(e.to_string());
    }

    let mut profiles = 0usize;
    let adapters_dir = project.join("adapters");
    if adapters_dir.is_dir() {
        for entry in std::fs::read_dir(&adapters_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            profiles += 1;
            let text = std::fs::read_to_string(&path)?;
            if let Err(e) = Profile::parse(&text) {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
    }

    if json {
        let report = serde_json::json!({
            "checks": config.gate.checks.len(),
            "profiles": profiles,
            "problems": problems,
            "ok": problems.is_empty(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if problems.is_empty() {
        println!(
            "ok — {} gate check(s), {profiles} adapter profile(s)",
            config.gate.checks.len()
        );
    } else {
        for p in &problems {
            println!("problem: {p}");
        }
    }

    Ok(problems.is_empty())
}
