//! Reading `ostraka.toml`.

use crate::gate::GateSpec;
use crate::policy::Policy;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_worktree_base")]
    pub base: String,
    /// Paths linked from the project into every worktree before anything runs.
    ///
    /// A worktree is a fresh checkout, so anything git ignores is absent from
    /// it — `node_modules/`, `.venv/`, `vendor/`. Without them the gate cannot
    /// execute and neither can the agent, and the run fails for a reason that
    /// has nothing to do with the change.
    ///
    /// Linked rather than copied: installing per worktree costs hundreds of
    /// megabytes each, and a run should not be the reason a disk fills.
    #[serde(default)]
    pub link: Vec<String>,
    /// A command run inside the worktree before the agent starts.
    ///
    /// For what linking cannot express — generated code, a build step, an
    /// install that must be per-worktree. It runs before the author, not just
    /// before the gate: an agent that cannot run the project's tools cannot see
    /// what it broke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

/// True for a relative path that cannot climb out of the directory it joins.
fn is_contained(path: &str) -> bool {
    use std::path::{Component, Path};
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn default_worktree_base() -> String {
    // Under the directory ostraka owns, and resolved against the workspace
    // rather than against a repository: worktrees belong to the runtime, not
    // to the thing being worked on, and a checkout of somebody's repository is
    // not somewhere to leave copies of it.
    ".ostraka/worktrees".to_string()
}

// Written out rather than derived: a derived Default would give `base` an empty
// string whenever the `[worktree]` table is absent entirely, and the runtime
// would then create worktrees at the project root.
impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base: default_worktree_base(),
            link: Vec::new(),
            setup: None,
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
        for path in &self.worktree.link {
            if !is_contained(path) {
                return Err(Error::Invalid(format!(
                    "[worktree] link {path:?} must be a relative path inside the project"
                )));
            }
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

    #[test]
    fn a_link_cannot_climb_out_of_the_project() {
        for bad in ["../secrets", "/etc", "a/../../b"] {
            let text = format!(
                "[gate]\nchecks = [{{ name = \"t\", cmd = \"true\", required = true }}]\n[worktree]\nlink = [\"{bad}\"]\n"
            );
            let config = Config::parse(&text).expect("parses");
            assert!(config.validate().is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn an_ordinary_link_is_accepted() {
        let text = "[gate]\nchecks = [{ name = \"t\", cmd = \"true\", required = true }]\n[worktree]\nlink = [\"node_modules\", \"packages/app/node_modules\"]\n";
        Config::parse(text)
            .expect("parses")
            .validate()
            .expect("valid");
    }

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
        assert_eq!(cfg.worktree.base, ".ostraka/worktrees");
    }

    #[test]
    fn an_absent_worktree_table_still_yields_a_usable_base() {
        let cfg = Config::parse(SAMPLE).expect("parses");
        assert_eq!(cfg.worktree.base, ".ostraka/worktrees");
        assert_eq!(WorktreeConfig::default().base, ".ostraka/worktrees");
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
