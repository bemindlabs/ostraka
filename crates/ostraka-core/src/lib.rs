//! Domain types for Ostraka.
//!
//! This crate holds types and parsing only. It spawns no processes, performs no
//! IO beyond reading configuration, and depends on nothing but serde, toml and
//! thiserror. Keeping it inert is what lets every other crate agree on the same
//! vocabulary without inheriting a runtime.

pub mod config;
pub mod gate;
pub mod identity;
pub mod manifest;
pub mod policy;
pub mod record;
pub mod task;

use thiserror::Error;

/// Anything that can go wrong while reading or validating configuration.
#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to parse {what}: {source}")]
    Parse {
        what: &'static str,
        #[source]
        source: toml::de::Error,
    },

    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
