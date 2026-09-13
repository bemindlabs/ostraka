//! Declared limits a run must stay inside.
//!
//! Policy is stated up front and enforced at the gate, so that what an agent is
//! permitted to do is readable before it does anything.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    /// Paths a task may modify. Empty means the policy places no path limit.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Wall-clock ceiling for one agent run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// When true, a run that touches a path outside `allowed_paths` is rejected
    /// even if every check passed.
    #[serde(default)]
    pub enforce_paths: bool,
}

impl Policy {
    /// Whether a set of touched paths satisfies this policy.
    pub fn permits(&self, touched: &[String]) -> bool {
        if !self.enforce_paths || self.allowed_paths.is_empty() {
            return true;
        }
        touched
            .iter()
            .all(|p| self.allowed_paths.iter().any(|a| within(p, a)))
    }
}

/// Whether `path` is `allowed` or lies beneath it, one path component at a time.
///
/// Not `starts_with`. A string prefix made `src` permit `src-other/secrets.rs`
/// and `docs` permit `docsite/`, which is a path limit that holds only for
/// people who happen to end every entry with a slash. A trailing slash is
/// accepted and means the same thing as its absence.
fn within(path: &str, allowed: &str) -> bool {
    let allowed = allowed.trim_end_matches('/');
    allowed.is_empty()
        || path == allowed
        || path
            .strip_prefix(allowed)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unenforced_policy_permits_everything() {
        let policy = Policy::default();
        assert!(policy.permits(&["anywhere/at/all.rs".to_string()]));
    }

    #[test]
    fn enforcement_confines_writes_to_declared_paths() {
        let policy = Policy {
            allowed_paths: vec!["crates/".to_string()],
            enforce_paths: true,
            ..Policy::default()
        };
        assert!(policy.permits(&["crates/ostraka-core/src/lib.rs".to_string()]));
        assert!(!policy.permits(&[".github/workflows/release.yml".to_string()]));
    }

    #[test]
    fn a_declared_path_is_matched_by_component_not_by_prefix() {
        let policy = Policy {
            allowed_paths: vec!["src".to_string()],
            enforce_paths: true,
            ..Policy::default()
        };
        assert!(policy.permits(&["src/lib.rs".to_string()]));
        assert!(policy.permits(&["src".to_string()]));
        assert!(
            !policy.permits(&["src-other/secrets.rs".to_string()]),
            "a sibling that shares a prefix was treated as inside"
        );
        assert!(!policy.permits(&["srcfile.rs".to_string()]));

        // A trailing slash means the same thing as none.
        let slashed = Policy {
            allowed_paths: vec!["docs/".to_string()],
            ..policy
        };
        assert!(slashed.permits(&["docs/guide.md".to_string()]));
        assert!(!slashed.permits(&["docsite/index.html".to_string()]));
    }
}
