//! Reading `ostraka.toml`.

use crate::gate::GateSpec;
use crate::policy::Policy;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_worktree_base")]
    pub base: String,
}

fn default_worktree_base() -> String {
    "worktrees".to_string()
}

// Written out rather than derived: a derived Default would give `base` an empty
// string whenever the `[worktree]` table is absent entirely, and the runtime
// would then create worktrees at the project root.
impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base: default_worktree_base(),
        }
    }
}

/// The project's configuration, as read from `ostraka.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub gate: GateSpec,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub worktree: WorktreeConfig,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|source| Error::Parse {
            what: "ostraka.toml",
            source,
        })
    }

    /// Rejects configurations that would make the gate meaningless.
    ///
    /// A project with no checks and no independent review is not gated, and
    /// saying so at load time is better than discovering it at merge time.
    pub fn validate(&self) -> Result<()> {
        if self.gate.checks.is_empty() {
            return Err(Error::Invalid(
                "[gate] declares no checks; the gate would pass everything".to_string(),
            ));
        }
        for check in &self.gate.checks {
            if check.cmd.trim().is_empty() {
                return Err(Error::Invalid(format!(
                    "check {:?} has an empty command",
                    check.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [gate]
        checks = [
            { name = "test", cmd = "cargo test --workspace", required = true },
        ]

        [gate.review]
        must_differ_from_author = true
    "#;

    #[test]
    fn parses_and_validates_a_real_config() {
        let cfg = Config::parse(SAMPLE).expect("parses");
        cfg.validate().expect("valid");
        assert_eq!(cfg.gate.checks.len(), 1);
        assert!(cfg.gate.review.must_differ_from_author);
        assert_eq!(cfg.worktree.base, "worktrees");
    }

    #[test]
    fn an_absent_worktree_table_still_yields_a_usable_base() {
        let cfg = Config::parse(SAMPLE).expect("parses");
        assert_eq!(cfg.worktree.base, "worktrees");
        assert_eq!(WorktreeConfig::default().base, "worktrees");
    }

    #[test]
    fn a_gate_with_no_checks_is_refused() {
        let cfg = Config::parse("[gate]\nchecks = []\n").expect("parses");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn an_empty_command_is_refused() {
        let cfg = Config::parse(
            r#"
            [gate]
            checks = [{ name = "lint", cmd = "  " }]
            "#,
        )
        .expect("parses");
        assert!(cfg.validate().is_err());
    }
}
