//! What an adapter can be relied on to do, and whether it is usable here.

use serde::{Deserialize, Serialize};

/// Declared abilities of a vendor CLI. The runtime reads these instead of
/// special-casing any particular tool.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Runs to completion without a terminal UI or interactive prompts.
    #[serde(default)]
    pub headless: bool,
    /// Emits machine-readable events rather than only prose.
    #[serde(default)]
    pub streams_json: bool,
    /// Can continue a previous session.
    #[serde(default)]
    pub resumable: bool,
}

/// Whether an adapter can be used on this machine right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready { version: Option<String> },
    NotFound { command: String },
    Unusable { reason: String },
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Ready { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ready_counts_as_usable() {
        assert!(Availability::Ready { version: None }.is_ready());
        assert!(
            !Availability::NotFound {
                command: "nope".into()
            }
            .is_ready()
        );
        assert!(
            !Availability::Unusable {
                reason: "not authenticated".into()
            }
            .is_ready()
        );
    }
}
