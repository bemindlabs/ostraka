//! What an adapter can be relied on to do, and whether it is usable here.

use serde::{Deserialize, Serialize};

/// Declared abilities of a vendor CLI. The runtime reads these instead of
/// special-casing any particular tool.
///
/// Every field here describes **the invocation this profile declares**, not
/// what the binary could be made to do if it were invoked differently. The
/// runtime only ever runs the declared command line, so the other reading
/// describes something nothing can act on.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Runs to completion without a terminal UI or interactive prompts.
    #[serde(default)]
    pub headless: bool,
    /// Whether this profile's declared invocation emits machine-readable
    /// events rather than only prose.
    ///
    /// **Deprecated, and it should never have been a field.** Under the rule
    /// above it is exactly `event_format != EventFormat::None`, which the
    /// profile already says — so the two can disagree, and they did: `codex`
    /// and `kimi-cli` both claimed it while invoking their CLI in a prose mode,
    /// on the strength of what the binary is able to do elsewhere. Nothing
    /// reads this, which is the only reason that never produced a wrong answer.
    ///
    /// It stays until the next major, because 1.0.0 is a stable API under
    /// SemVer and a field removed the day after a release makes the number
    /// mean nothing. `check-hygiene.sh` holds it equal to `event_format` in
    /// the meantime, so it cannot drift again while it waits.
    #[serde(default)]
    pub streams_json: bool,
    /// Can continue a previous session.
    #[serde(default)]
    pub resumable: bool,
}

impl Capabilities {
    /// What `streams_json` is, derived from the thing that actually decides it.
    ///
    /// Here so the answer has one home before the field goes: a caller that
    /// wants to know whether a profile's output can be parsed should ask its
    /// `event_format`, and this is that question spelled the old way.
    pub fn emits_events(format: crate::profile::EventFormat) -> bool {
        format != crate::profile::EventFormat::None
    }
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
