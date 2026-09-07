//! The `init` command: report the plan, then carry it out.

use crate::init::{self, Action};
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, force: bool, json: bool) -> Outcome {
    let plan = init::plan(project);

    if json {
        let rows: Vec<_> = plan
            .files
            .iter()
            .map(|f| {
                serde_json::json!({
                    "path": f.path.display().to_string(),
                    "action": format!("{:?}", f.action).to_lowercase(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "project": plan.project.display().to_string(),
                "detected": plan.kind.describe(),
                "complete": plan.complete(),
                "files": rows,
            }))?
        );
    }

    if plan.complete() && !force {
        if !json {
            println!("already a project — nothing to write");
        }
        return Ok(true);
    }

    let written = init::apply(&plan, force)?;

    if !json {
        println!("detected {}", plan.kind.describe());
        for path in &written {
            let name = path.strip_prefix(project).unwrap_or(path);
            println!("  wrote  {}", name.display());
        }
        for file in plan
            .files
            .iter()
            .filter(|f| f.action == Action::AlreadyThere)
        {
            let name = file.path.strip_prefix(project).unwrap_or(&file.path);
            println!("  kept   {}", name.display());
        }
        println!("\nNext: `ostraka adapters` to see which CLIs can run here,");
        println!("      then `ostraka run \"...\"`.");
        if plan.kind == init::Kind::Unknown {
            println!(
                "\nostraka.toml declares a check that fails on purpose. Ostraka could not\n\
                 tell how this project is verified, and guessing would have been worse."
            );
        }
    }

    Ok(true)
}
