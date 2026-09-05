//! What an agent is asked to do.

use crate::identity::ActorId;
use serde::{Deserialize, Serialize};

/// A unit of work handed to one adapter in one worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Stable identifier; also the run-record directory name.
    pub id: String,
    /// The instruction given to the agent, verbatim.
    pub prompt: String,
    /// Which adapter profile executes it. A profile id, never a vendor name
    /// baked into the runtime.
    pub adapter: String,
    /// Who is accountable for the resulting change.
    pub author: ActorId,
    /// Git ref the worktree branches from.
    #[serde(default = "default_base_ref")]
    pub base_ref: String,
    /// Optional model hint passed through to the adapter's args template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn default_base_ref() -> String {
    "HEAD".to_string()
}

/// The kind of work a task represents. Review tasks are routed differently:
/// the gate refuses to accept a review produced by the change's own author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Author,
    Review,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_ref_defaults_to_head() {
        let spec: TaskSpec = toml::from_str(
            r#"
            id = "t1"
            prompt = "do the thing"
            adapter = "example"
            author = "archon"
            "#,
        )
        .expect("valid task spec");
        assert_eq!(spec.base_ref, "HEAD");
        assert!(spec.model.is_none());
    }
}
