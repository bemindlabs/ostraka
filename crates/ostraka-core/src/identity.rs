//! Who performed an action.
//!
//! Every record carries an actor, because the audit trail is only useful if a
//! change traces back to whoever authorized it.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The identity of an agent as it appears in records and approvals.
///
/// Two identities are equal when their strings are equal. The gate compares
/// them to decide whether a review is independent of the work it reviews, so
/// this type deliberately has no notion of "close enough".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId(String);

impl ActorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_compare_exactly() {
        assert_eq!(ActorId::new("archon"), ActorId::new("archon"));
        assert_ne!(ActorId::new("archon"), ActorId::new("Archon"));
    }
}
