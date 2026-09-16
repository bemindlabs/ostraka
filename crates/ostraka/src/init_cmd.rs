//! The `init` command: report the plan, then carry it out.

use crate::drift;
use crate::init::{self, Action};
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(
    project: &Path,
    repositories: Option<&Path>,
    force: bool,
    upgrade: bool,
    json: bool,
) -> Outcome {
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

    // Read from the plan, before anything is carried out, so it describes what
    // was about to happen rather than what is there afterwards. Printed at the
    // end, with the drift, because `--json` is one object.
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

    // "already a project — nothing to write" was the whole of the answer, and
    // it is the sentence a workspace upgraded from an earlier version gets
    // while three of its kept files are missing tables this version ships.
    // Nothing is written here either; what follows it is a report.
    let nothing_to_write = plan.complete() && !force;
    if nothing_to_write {
        if !json {
            println!("already a project — nothing to write");
        }
    } else {
        let written = init::apply(&plan, force)?;

        if !json {
            println!("detected {}", plan.kind.describe());
            // "wrote" for a file that already existed and was edited in place
            // put an operator in the position of not knowing their own
            // `.gitignore` had been touched — every other pre-existing file in
            // the same run is listed as `kept`. The three verbs are the three
            // things that happen.
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
    }

    // Asked of the directory after the plan has been carried out, so a file
    // this run has just written is stock by definition and reports nothing.
    let workspace = crate::workspace::Workspace::at(project);
    let mut drifts = drift::survey(&workspace);

    if upgrade {
        let added = drift::upgrade(&drifts)?;
        if !json {
            for (path, tables) in &added {
                // Named as the report names it: `Workspace::at` resolves its
                // root, so these paths are absolute while `project` is usually
                // `.`, and stripping one from the other gives back the whole
                // absolute path.
                let name = drifts
                    .iter()
                    .find(|d| &d.path == path)
                    .map(|d| d.relative.clone())
                    .unwrap_or_else(|| path.display().to_string());
                let named: Vec<String> = tables.iter().map(|t| format!("[{t}]")).collect();
                println!("  added   {} to {name}", named.join(" "));
            }
            if added.is_empty() {
                println!("nothing to upgrade — no kept file is missing a table");
            }
        }
        // Read again rather than filtered: what is left is what `--upgrade` is
        // not for, and saying so needs the file as it now stands.
        drifts = drift::survey(&workspace);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "project": plan.project.display().to_string(),
                "detected": plan.kind.describe(),
                "complete": plan.complete(),
                "files": rows,
                "drift": drift::as_json(&drifts),
            }))?
        );
    } else if !drifts.is_empty() {
        print!("{}", drift::report(&drifts));
    }

    Ok(true)
}
