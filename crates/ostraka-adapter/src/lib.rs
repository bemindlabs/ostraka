//! The vendor boundary.
//!
//! The runtime favors no vendor, so no vendor is named in this crate. A
//! supported CLI is a TOML profile in `adapters/`, loaded at runtime; adding one
//! is a file, not a release.
//!
//! The contract is deliberately small. An adapter runs a prompt headlessly to
//! completion in a directory, exits with a status, and leaves its work on disk.
//! The unit of truth is the git diff in the worktree, not the transcript — which
//! is what makes a change written by one vendor reviewable by another.

pub mod capability;
pub mod environment;
pub mod event;
pub mod isolation;
pub mod process;
pub mod profile;
pub mod usage;

use ostraka_core::record::Event;
use ostraka_core::task::TaskSpec;
use std::path::Path;
use thiserror::Error;

pub use capability::{Availability, Capabilities};
pub use isolation::Isolation;
pub use profile::Profile;

#[derive(Debug, Error)]
pub enum Error {
    #[error("adapter profile {id:?}: {message}")]
    Profile { id: String, message: String },

    #[error(
        "adapter profile {id:?}: could not isolate this vendor from the operator's setup: {message}"
    )]
    Isolation { id: String, message: String },

    #[error("failed to launch {command:?}: {source}")]
    Launch {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// How a finished run turned out, from the adapter's point of view.
#[derive(Debug, Clone)]
pub struct AdapterOutcome {
    pub exit_code: Option<i32>,
    pub files_touched: Vec<String>,
    /// What the vendor said the run cost. `None` when it said nothing.
    pub usage: Option<ostraka_core::record::TokenUsage>,
    /// Why it failed, in the vendor's own words. `None` on success.
    ///
    /// A coding CLI reports an expired credential, a rate limit or a missing
    /// sandbox capability on stderr and exits non-zero, saying nothing on
    /// stdout. Without this, every one of those becomes the same unreadable
    /// "exited with 1" — and a refusal nobody can act on is barely better than
    /// no refusal at all.
    pub diagnostics: Option<String>,
}

/// A live agent process, yielding normalized events until it ends.
pub trait Session {
    fn next_event(&mut self) -> Option<Event>;
    fn finish(self: Box<Self>) -> AdapterOutcome;
}

/// One way of running a coding agent.
pub trait VendorAdapter: Send + Sync {
    /// The profile id. Comes from configuration, never a literal in this crate.
    fn id(&self) -> &str;

    /// Whether this agent can actually be run here: on PATH, and answering.
    fn probe(&self) -> Availability;

    /// What the runtime may rely on this agent to do.
    fn capabilities(&self) -> Capabilities;

    /// Start the agent against a task inside an existing worktree.
    fn launch(&self, spec: &TaskSpec, worktree: &Path) -> Result<Box<dyn Session>>;
}
