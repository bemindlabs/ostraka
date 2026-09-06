//! The engine.
//!
//! Plan, route, isolate, execute, gate, review, record. The orchestrator lives
//! here and so does the gate, deliberately: see [`gate::MergeToken`] for why
//! they must not be separated.

pub mod gate;
pub mod orchestrator;
pub mod promote;
pub mod record;
pub mod review;
pub mod route;
pub mod worktree;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] ostraka_core::Error),

    #[error(transparent)]
    Adapter(#[from] ostraka_adapter::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
