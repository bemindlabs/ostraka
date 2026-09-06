//! `ostraka promote` — give an approved run a branch someone can merge.
//!
//! Deliberately stops at the branch. Merging, pushing and opening a pull
//! request are acts a person takes; this command's job is to make sure that
//! what they take them on actually passed the gate.

use crate::project;
use ostraka_runtime::promote::{self, NotPromoted};
use std::path::{Path, PathBuf};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project_dir: &Path, run_id: &str, branch: Option<&str>, json: bool) -> Outcome {
    let config = project::load_config(project_dir)?;
    let records_root: PathBuf = project_dir.join(".ostraka");

    let result = promote::promote(project_dir, &records_root, run_id, &config, branch)?;

    match result {
        Ok(p) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "promoted": true,
                        "run_id": run_id,
                        "branch": p.branch,
                        "commit": p.commit,
                        "author": p.token.author().to_string(),
                        "reviewer": p.token.reviewer().to_string(),
                    }))?
                );
            } else {
                println!(
                    "promoted {run_id} to {} ({})",
                    p.branch,
                    &p.commit[..p.commit.len().min(12)]
                );
                println!(
                    "  written by {}, reviewed by {}",
                    p.token.author(),
                    p.token.reviewer()
                );
                // Named, not run. Merging is the person's decision, and a tool
                // that offers to take it for them is the thing this project
                // exists not to be.
                println!("\nnothing has been merged. To take it further:");
                println!("  git merge --no-ff {}", p.branch);
                println!("  gh pr create --head {}", p.branch);
            }
            Ok(true)
        }
        Err(why) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "promoted": false,
                        "run_id": run_id,
                        "reason": why.to_string(),
                    }))?
                );
            } else {
                println!("not promoted — {why}");
                if let NotPromoted::Refused(_) = why {
                    println!("\nA run that did not pass the gate cannot be promoted by promoting.");
                }
            }
            Ok(false)
        }
    }
}
