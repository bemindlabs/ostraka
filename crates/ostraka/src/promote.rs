//! `ostraka promote` — give an approved run a branch someone can merge.
//!
//! Deliberately stops at the branch. Merging, pushing and opening a pull
//! request are acts a person takes; this command's job is to make sure that
//! what they take them on actually passed the gate.

use crate::workspace::{Repository, Workspace};
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{index, orchestrator};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &Workspace, run_id: &str, branch: Option<&str>, json: bool) -> Outcome {
    let records_root = workspace.records();
    let repo = repository_of(workspace, &records_root, run_id)?;
    let config = workspace.config_for(&repo)?;

    let result = promote::promote(&repo.path, &records_root, run_id, &config, branch)?;

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

/// The repository a recorded run was made in.
///
/// Read from the record, because a workspace can hold several and promoting
/// into the wrong one would be promoting a commit that is not there. A record
/// written before a workspace could hold more than one names none, and the
/// answer for those is the only repository there was.
pub fn repository_of(
    workspace: &Workspace,
    records_root: &std::path::Path,
    run_id: &str,
) -> Result<Repository, Box<dyn std::error::Error>> {
    let named = orchestrator::replay(records_root, run_id)
        .ok()
        .map(|(record, _)| record.repository)
        .filter(|name| !name.is_empty());
    workspace.repository(named.as_deref())
}

/// The repository a listed run was made in, for the callers that already have
/// the summary and should not read the record again to learn one field.
pub fn repository_for(
    workspace: &Workspace,
    run: &index::RunSummary,
) -> Result<Repository, Box<dyn std::error::Error>> {
    workspace.repository(Some(run.repository.as_str()).filter(|n| !n.is_empty()))
}
