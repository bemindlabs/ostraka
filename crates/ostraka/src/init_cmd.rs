//! The `init` command: report the plan, then carry it out.

use crate::init::{self, Action};
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, repositories: Option<&Path>, force: bool, json: bool) -> Outcome {
    // Refused by name before anything is planned. Taken, the workspace would
    // list its own `.ostraka` and `notes` as repositories; written down, it
    // would be `repositories = ""`, which the next command refuses with a
    // sentence about emptiness instead of this one. Raised in review.
    if let Some(given) = repositories {
        let dir = crate::workspace::resolve_flag(given)?;
        let root = project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf());
        if dir == root {
            return Err(format!(
                "--repositories {} is the workspace itself; its repositories need a \
                 directory of their own",
                given.display()
            )
            .into());
        }
    }

    let plan = init::plan_with(project, repositories);

    // `init` never overwrites a file, so a workspace that already has its
    // config keeps it — and the directory just named on the command line would
    // be forgotten by the next command without anybody being told why.
    let config_kept = plan
        .files
        .iter()
        .any(|f| f.role == init::Role::Config && f.action == Action::AlreadyThere);
    if repositories.is_some() && config_kept && !json {
        eprintln!(
            "note: .ostraka/ostraka.toml already exists and was left alone, so --repositories \
             lasts for this command only — add `repositories = \"…\"` under a [workspace] \
             table there to keep it"
        );
    }

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
        // "wrote" for a file that already existed and was edited in place put
        // an operator in the position of not knowing their own `.gitignore`
        // had been touched — every other pre-existing file in the same run is
        // listed as `kept`. The three verbs are the three things that happen.
        for (path, action) in &written {
            let name = path.strip_prefix(project).unwrap_or(path);
            let verb = match action {
                Action::Append => "updated",
                _ => "wrote  ",
            };
            println!("  {verb} {}", name.display());
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
