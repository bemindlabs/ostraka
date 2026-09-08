//! Promoting an approved run to a branch someone can merge.
//!
//! A run commits inside its worktree and stops. Nothing here merges anything:
//! promotion gives an approved change a branch of its own in the repository, so
//! the worktree can go away and a person can merge or open a pull request as a
//! deliberate act of their own.
//!
//! The invariant that has to survive the process boundary is that a change which
//! never obtained a [`crate::gate::MergeToken`] cannot be promoted. It survives
//! by asking the gate again rather than by storing an answer: a serialized token
//! would be a token anyone could write. See [`crate::gate::reaffirm`].
//!
//! Two independent records have to agree before anything is named. The run
//! record says what the gate decided; the commit's own trailers say who wrote
//! and who reviewed it, and those are in git history where the runtime put them.
//! Rule 6 applies here as much as anywhere: what happened is read from git, not
//! from a file describing git.

use crate::gate::{self, MergeToken, Refusal};
use crate::{Error, Result};
use ostraka_core::config::Config;
use ostraka_core::record::{Outcome, RunRecord};
use std::path::Path;
use std::process::Command;

/// A promoted run: what was named, and what it was named after.
#[derive(Debug)]
pub struct Promotion {
    pub branch: String,
    pub commit: String,
    pub run_branch: String,
    pub token: MergeToken,
}

/// Why a promotion did not happen.
#[derive(Debug)]
pub enum NotPromoted {
    /// The gate would not mint a token from the run's evidence.
    Refused(Refusal),
    /// The record and the commit disagree, or the commit is not there.
    Unverifiable(String),
}

impl std::fmt::Display for NotPromoted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "the gate refuses to promote this run: {r:?}"),
            Self::Unverifiable(why) => write!(f, "{why}"),
        }
    }
}

/// The branch a run is promoted to when the caller names none.
pub fn default_branch(run_id: &str) -> String {
    format!("promoted/{run_id}")
}

/// Gives an approved run's commit a branch of its own.
///
/// Refuses, without touching the repository, unless every one of these holds:
/// the record says the run was approved, the gate mints a token from its
/// evidence against the project's *current* checks, the run's branch exists,
/// and its head commit's trailers name the same run, author and reviewer the
/// record does.
pub fn promote(
    repo: &Path,
    records_root: &Path,
    run_id: &str,
    config: &Config,
    branch: Option<&str>,
) -> Result<std::result::Result<Promotion, NotPromoted>> {
    let record = read_record(records_root, run_id)?;

    // The recorded outcome is checked first only so the refusal is legible; the
    // gate below is what actually decides, and it does not consult this field.
    if record.outcome != Some(Outcome::Approved) {
        return Ok(Err(NotPromoted::Unverifiable(format!(
            "run {run_id:?} ended as {:?}, not approved",
            record.outcome
        ))));
    }

    let token = match gate::reaffirm(
        &config.gate,
        &record,
        config.gate.review.must_differ_from_author,
    ) {
        Ok(token) => token,
        Err(refusal) => return Ok(Err(NotPromoted::Refused(refusal))),
    };

    let run_branch = format!("ostraka/{run_id}");
    let commit = match rev_parse(repo, &run_branch)? {
        Some(sha) => sha,
        None => {
            return Ok(Err(NotPromoted::Unverifiable(format!(
                "no branch {run_branch:?} in this repository; the run's commit is not here"
            ))));
        }
    };

    let message = commit_message(repo, &commit)?;
    if let Err(why) = trailers_agree(&message, run_id, &record, &token) {
        return Ok(Err(NotPromoted::Unverifiable(why)));
    }

    let target = branch
        .map(str::to_string)
        .unwrap_or_else(|| default_branch(run_id));
    if rev_parse(repo, &target)?.is_some() {
        return Ok(Err(NotPromoted::Unverifiable(format!(
            "branch {target:?} already exists; name another with --branch"
        ))));
    }

    let out = Command::new("git")
        .args(["branch", &target, &commit])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "git branch failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    Ok(Ok(Promotion {
        branch: target,
        commit,
        run_branch,
        token,
    }))
}

fn read_record(records_root: &Path, run_id: &str) -> Result<RunRecord> {
    let path = records_root.join("runs").join(run_id).join("record.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Other(format!("no run {run_id:?}: {e}")))?;
    serde_json::from_str(&text).map_err(|e| Error::Other(format!("run record is unreadable: {e}")))
}

fn rev_parse(repo: &Path, rev: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", rev])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok(None);
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!sha.is_empty()).then_some(sha))
}

fn commit_message(repo: &Path, commit: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%B", commit])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Checks the commit's trailers against the record the gate just read.
///
/// The record is a file beside the repository; the trailers are inside history.
/// Requiring both to say the same thing means a promotion cannot be conjured by
/// editing one of them.
fn trailers_agree(
    message: &str,
    run_id: &str,
    record: &RunRecord,
    token: &MergeToken,
) -> std::result::Result<(), String> {
    let has = |prefix: &str, value: &str| {
        message.lines().any(|line| {
            line.trim().strip_prefix(prefix).is_some_and(|rest| {
                rest.trim() == value || rest.trim().starts_with(&format!("{value} ("))
            })
        })
    };

    if !has("Run:", run_id) {
        return Err(format!(
            "the commit on this run's branch does not carry `Run: {run_id}`; it is not the commit \
             this run produced"
        ));
    }
    if !has("Authored-by:", record.author.as_str()) {
        return Err(format!(
            "the commit says it was written by someone other than {}, which the run record names",
            record.author
        ));
    }
    if !has("Reviewed-by:", token.reviewer().as_str()) {
        return Err(format!(
            "the commit says it was reviewed by someone other than {}, which the approval names",
            token.reviewer()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::gate::{Approval, CheckRecord, Verdict};
    use ostraka_core::identity::ActorId;

    fn record() -> RunRecord {
        RunRecord {
            run_id: "r1".into(),
            task_id: "t1".into(),
            prompt: "do a thing".into(),
            author: ActorId::new("archon"),
            adapter: "a".into(),
            repository: "only".into(),
            started_at: "now".into(),
            finished_at: None,
            checks: vec![CheckRecord {
                name: "test".into(),
                cmd: "true".into(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 1,
            }],
            approval: Some(Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Approve,
            }),
            usage: Vec::new(),
            outcome: Some(Outcome::Approved),
        }
    }

    fn token() -> MergeToken {
        let spec = ostraka_core::gate::GateSpec {
            timeout_secs: None,
            checks: vec![ostraka_core::gate::Check {
                name: "test".into(),
                cmd: "true".into(),
                required: true,
            }],
            review: ostraka_core::gate::ReviewPolicy::default(),
        };
        gate::reaffirm(&spec, &record(), true).expect("the record earns a token")
    }

    #[test]
    fn agreeing_trailers_are_accepted() {
        let message = "Do a thing\n\nRun: r1\nAuthored-by: archon (claude-code)\nReviewed-by: \
                       ephor (codex)\n";
        assert!(trailers_agree(message, "r1", &record(), &token()).is_ok());
    }

    #[test]
    fn a_commit_from_a_different_run_is_not_this_runs_commit() {
        let message = "Do a thing\n\nRun: r2\nAuthored-by: archon (c)\nReviewed-by: ephor (d)\n";
        let err = trailers_agree(message, "r1", &record(), &token()).expect_err("must refuse");
        assert!(err.contains("Run: r1"), "{err}");
    }

    #[test]
    fn a_commit_naming_a_different_reviewer_than_the_approval_is_refused() {
        // Forging a promotion would take editing the record and rewriting the
        // commit; agreeing with only one of them is not enough.
        let message = "Do a thing\n\nRun: r1\nAuthored-by: archon (c)\nReviewed-by: archon (c)\n";
        let err = trailers_agree(message, "r1", &record(), &token()).expect_err("must refuse");
        assert!(err.contains("reviewed by"), "{err}");
    }

    #[test]
    fn a_run_whose_required_check_never_passed_earns_no_token() {
        let mut r = record();
        r.checks[0].exit_code = Some(1);
        let spec = ostraka_core::gate::GateSpec {
            timeout_secs: None,
            checks: vec![ostraka_core::gate::Check {
                name: "test".into(),
                cmd: "true".into(),
                required: true,
            }],
            review: ostraka_core::gate::ReviewPolicy::default(),
        };
        assert!(gate::reaffirm(&spec, &r, true).is_err());
    }

    #[test]
    fn a_check_added_since_the_run_blocks_promotion() {
        // The record cannot show a check passing that did not exist when it was
        // written, and promoting as though it had would launder the addition.
        let spec = ostraka_core::gate::GateSpec {
            timeout_secs: None,
            checks: vec![
                ostraka_core::gate::Check {
                    name: "test".into(),
                    cmd: "true".into(),
                    required: true,
                },
                ostraka_core::gate::Check {
                    name: "lint".into(),
                    cmd: "true".into(),
                    required: true,
                },
            ],
            review: ostraka_core::gate::ReviewPolicy::default(),
        };
        match gate::reaffirm(&spec, &record(), true) {
            Err(Refusal::ChecksFailed { failed, .. }) => assert_eq!(failed, ["lint"]),
            other => panic!("expected the missing check to block: {other:?}"),
        }
    }

    #[test]
    fn a_self_approved_run_cannot_be_promoted() {
        let mut r = record();
        r.approval = Some(Approval {
            reviewer: ActorId::new("archon"),
            verdict: Verdict::Approve,
        });
        let spec = ostraka_core::gate::GateSpec {
            timeout_secs: None,
            checks: vec![ostraka_core::gate::Check {
                name: "test".into(),
                cmd: "true".into(),
                required: true,
            }],
            review: ostraka_core::gate::ReviewPolicy::default(),
        };
        assert!(matches!(
            gate::reaffirm(&spec, &r, true),
            Err(Refusal::SelfApproval { .. })
        ));
    }
}
