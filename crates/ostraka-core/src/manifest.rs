//! Reading the previous generation's agent manifests.
//!
//! Those manifests carry per-agent `lintCmd` / `formatCmd` / `testCmd` /
//! `buildCmd` fields. They are read here only as a fallback for projects that
//! have not yet declared a `[gate]` of their own: checks describe a project, so
//! seven agents in one workspace repeating the same four strings is a modelling
//! error this crate does not perpetuate.

use crate::gate::{Check, GateSpec};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyManifest {
    pub name: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    #[serde(rename = "lintCmd", default)]
    pub lint_cmd: String,
    #[serde(rename = "formatCmd", default)]
    pub format_cmd: String,
    #[serde(rename = "testCmd", default)]
    pub test_cmd: String,
    #[serde(rename = "buildCmd", default)]
    pub build_cmd: String,
}

impl LegacyManifest {
    /// Converts the manifest's four commands into a gate, dropping any that are
    /// blank. Returns `None` when the manifest declares nothing runnable.
    pub fn to_gate(&self) -> Option<GateSpec> {
        let pairs = [
            ("format", &self.format_cmd),
            ("lint", &self.lint_cmd),
            ("test", &self.test_cmd),
            ("build", &self.build_cmd),
        ];
        let checks: Vec<Check> = pairs
            .into_iter()
            .filter(|(_, cmd)| !cmd.trim().is_empty())
            .map(|(name, cmd)| Check {
                name: name.to_string(),
                cmd: cmd.clone(),
                required: true,
            })
            .collect();

        if checks.is_empty() {
            return None;
        }
        Some(GateSpec {
            checks,
            review: Default::default(),
        })
    }
}

/// Where a project's gate came from. Recorded so a run can explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateSource {
    ProjectConfig,
    LegacyManifest,
}

pub fn parse_manifest(json: &str) -> Result<LegacyManifest> {
    serde_json::from_str(json).map_err(|e| Error::Invalid(format!("manifest: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_commands_do_not_become_checks() {
        let m = LegacyManifest {
            name: "archon".into(),
            agent_id: "agent-archon".into(),
            lint_cmd: "cargo clippy".into(),
            format_cmd: String::new(),
            test_cmd: "   ".into(),
            build_cmd: "cargo build".into(),
        };
        let gate = m.to_gate().expect("has runnable checks");
        let names: Vec<&str> = gate.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["lint", "build"]);
    }

    #[test]
    fn a_manifest_with_nothing_runnable_yields_no_gate() {
        let m = LegacyManifest {
            name: "x".into(),
            agent_id: "agent-x".into(),
            lint_cmd: String::new(),
            format_cmd: String::new(),
            test_cmd: String::new(),
            build_cmd: String::new(),
        };
        assert!(m.to_gate().is_none());
    }
}
