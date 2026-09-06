//! The run record: what happened, in order, replayable afterwards.
//!
//! An action that leaves no trace did not happen. Every event an adapter emits
//! is normalized into the shapes below and appended to the run's log, with the
//! vendor's own payload preserved verbatim alongside it.

use crate::gate::{Approval, CheckRecord};
use crate::identity::ActorId;
use serde::{Deserialize, Serialize};

/// A normalized event from a running agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Message {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    ToolUse {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    Error {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    Finished {
        exit_code: Option<i32>,
        files_touched: Vec<String>,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Checks passed and an independent reviewer approved.
    Approved,
    /// A required check failed, or the reviewer rejected.
    Rejected,
    /// The agent or the runtime failed before a verdict existed.
    Failed,
}

/// The durable record of one task, start to finish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub task_id: String,
    /// What was asked. Defaulted so records written before this field existed
    /// still read back — a run id is opaque, and a list of runs nobody can
    /// identify is a list nobody uses.
    #[serde(default)]
    pub prompt: String,
    pub author: ActorId,
    pub adapter: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub checks: Vec<CheckRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_through_jsonl() {
        let event = Event::Finished {
            exit_code: Some(0),
            files_touched: vec!["src/lib.rs".to_string()],
        };
        let line = serde_json::to_string(&event).expect("serializes");
        let back: Event = serde_json::from_str(&line).expect("deserializes");
        match back {
            Event::Finished { exit_code, .. } => assert_eq!(exit_code, Some(0)),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
