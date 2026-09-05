//! The verification gate.
//!
//! The previous generation declared these commands in configuration and never
//! ran them; the gate was prose the agent was trusted to have obeyed. Here the
//! commands are data the runtime executes, and the outcome types below are the
//! only evidence that they passed.

use crate::identity::ActorId;
use serde::{Deserialize, Serialize};

/// One check the project must pass, as declared in `ostraka.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub cmd: String,
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

/// The gate as configured for a project.
///
/// The default is an empty gate, which `Config::validate` refuses: a project
/// must say how it is verified rather than inherit a silent pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateSpec {
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub review: ReviewPolicy,
}

/// Rules the review must satisfy before an approval counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPolicy {
    /// When true, an approval from the change's own author is rejected.
    #[serde(default = "default_must_differ")]
    pub must_differ_from_author: bool,
}

fn default_must_differ() -> bool {
    true
}

impl Default for ReviewPolicy {
    fn default() -> Self {
        Self {
            must_differ_from_author: default_must_differ(),
        }
    }
}

/// What actually happened when a check ran. Produced only by execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRecord {
    pub name: String,
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl CheckRecord {
    pub fn passed(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// A reviewer's decision.
///
/// Parsing is deliberately fail-safe: anything that is not an explicit approval
/// is a rejection. A reviewer that crashes, times out, or answers in prose has
/// not approved anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Approve,
    Reject { reason: String },
}

impl Verdict {
    /// Reads a verdict from a reviewer's output. The contract is a first line of
    /// `VERDICT: APPROVE` or `VERDICT: REJECT: <reason>`.
    pub fn parse(output: &str) -> Self {
        let first = output.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let line = first.trim();

        let Some(rest) = line.strip_prefix("VERDICT:") else {
            return Self::unparseable(line);
        };
        let rest = rest.trim();

        if rest.eq_ignore_ascii_case("APPROVE") {
            return Verdict::Approve;
        }
        if let Some(reason) = rest.strip_prefix("REJECT:") {
            return Verdict::Reject {
                reason: reason.trim().to_string(),
            };
        }
        if rest.eq_ignore_ascii_case("REJECT") {
            return Verdict::Reject {
                reason: "reviewer gave no reason".to_string(),
            };
        }
        Self::unparseable(line)
    }

    fn unparseable(line: &str) -> Self {
        Verdict::Reject {
            reason: format!("unparseable verdict line: {line:?}"),
        }
    }

    pub fn is_approve(&self) -> bool {
        matches!(self, Verdict::Approve)
    }
}

/// A review that has been carried out, tied to the identity that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub reviewer: ActorId,
    pub verdict: Verdict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_is_recognized() {
        assert_eq!(Verdict::parse("VERDICT: APPROVE"), Verdict::Approve);
        assert_eq!(
            Verdict::parse("\n\nVERDICT: approve\ntrailing chatter"),
            Verdict::Approve
        );
    }

    #[test]
    fn reject_carries_its_reason() {
        assert_eq!(
            Verdict::parse("VERDICT: REJECT: tests do not cover the new branch"),
            Verdict::Reject {
                reason: "tests do not cover the new branch".to_string()
            }
        );
    }

    #[test]
    fn anything_unparseable_is_a_rejection() {
        for output in ["", "looks good to me!", "APPROVE", "VERDICT: maybe"] {
            assert!(
                !Verdict::parse(output).is_approve(),
                "{output:?} must not read as approval"
            );
        }
    }

    #[test]
    fn a_check_passes_only_on_exit_zero() {
        let mut rec = CheckRecord {
            name: "test".into(),
            cmd: "cargo test".into(),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 1,
        };
        assert!(rec.passed());
        rec.exit_code = Some(101);
        assert!(!rec.passed());
        // A process killed by a signal reports no code at all.
        rec.exit_code = None;
        assert!(!rec.passed());
    }
}
