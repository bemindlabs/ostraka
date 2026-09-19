//! The agents beside the work: every profile this workspace has, whether it
//! can run here, what it is doing now, and whether it is the one a run would
//! use next.
//!
//! This is the mapping, kept out of the view so that drawing stays a function
//! of state and nothing about it has to be worked out while drawing. Each fact
//! comes from something that already answers it:
//!
//! - which profiles there are: `.ostraka/adapters`, as `Workspace::profiles`
//!   reads it;
//! - whether one can run here: the probe `ostraka adapters` uses, taken once on
//!   a thread and held, never repeated per frame;
//! - what one is doing: `index::live`, which reads the `live.json` the runtime
//!   writes beside every run in progress — so a `drain` worker in another
//!   process shows up exactly as a pane in this one does;
//! - which pair a run would use: `run::would_route`, the same choice `execute`
//!   makes, taken on a thread.
//!
//! A profile is only ever doing what the runtime says it is doing. Between the
//! agents — making the worktree, running the checks — no profile is running,
//! and the row says idle rather than claiming work that is not happening.

use super::view::Agent;
use ostraka_runtime::progress::Phase;
use ostraka_runtime::record::Live;

/// One thing a profile is doing now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// Writing the change for this run.
    Writing(String),
    /// Reviewing this run's change.
    Reviewing(String),
}

/// Whether a profile can run on this machine, as its probe said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Not asked yet. Probing runs every CLI, so it happens on a thread and the
    /// row says so until it answers rather than guessing.
    Asking,
    /// It answered. The note is what the probe reported, a version where the
    /// CLI gave one.
    Ready(String),
    /// It did not: not installed, or it would not start. The note says which.
    Missing(String),
}

/// One profile, as the side pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub probe: Probe,
    /// Everything it is doing, in every pane and every worker. Empty is idle.
    /// A profile can be in several runs at once.
    pub doing: Vec<Doing>,
    /// The profile a run started now would write with.
    pub writes_next: bool,
    /// The profile a run started now would be reviewed by.
    pub reviews_next: bool,
}

/// The rows, one per configured profile, in id order.
///
/// `profiles` is what the workspace has; `probes` is what the probe answered,
/// which may be nothing yet; `live` is every run in progress; `next` is the
/// pair a run started now would use, where one could be made.
pub fn rows(
    profiles: &[String],
    probes: &[Agent],
    live: &[(String, Live)],
    next: Option<&(String, String)>,
) -> Vec<Row> {
    let mut ids = profiles.to_vec();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let probe = match probes.iter().find(|a| a.configured && a.id == id) {
                None => Probe::Asking,
                Some(agent) if agent.ready => Probe::Ready(agent.note.clone()),
                Some(agent) => Probe::Missing(agent.note.clone()),
            };
            let doing = live
                .iter()
                .filter_map(|(run, live)| match live.phase {
                    Phase::Authoring if live.author == id => Some(Doing::Writing(run.clone())),
                    Phase::Reviewing if live.reviewer == id => Some(Doing::Reviewing(run.clone())),
                    _ => None,
                })
                .collect();
            Row {
                writes_next: next.is_some_and(|(author, _)| *author == id),
                reviews_next: next.is_some_and(|(_, reviewer)| *reviewer == id),
                id,
                probe,
                doing,
            }
        })
        .collect()
}

/// The narrowest terminal the pane appears on.
///
/// Below it, the width is the work's: a transcript is what this screen is for,
/// and one squeezed to make room for a list of profiles is one nobody can read.
/// Chosen so the work keeps at least the seventy-two columns below which its
/// detail already moves behind Enter, with the pane and its gap beside it.
pub const SHOWN_FROM: u16 = 100;

/// How wide the pane is when it is shown.
pub const WIDTH: u16 = 26;

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, ready: bool, note: &str) -> Agent {
        Agent {
            id: id.to_string(),
            ready,
            note: note.to_string(),
            configured: true,
        }
    }

    fn live(author: &str, reviewer: &str, phase: Phase) -> Live {
        Live {
            author: author.to_string(),
            reviewer: reviewer.to_string(),
            phase,
        }
    }

    #[test]
    fn every_configured_profile_is_a_row_whatever_it_is_doing() {
        let rows = rows(&["codex".into(), "agy".into()], &[], &[], None);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["agy", "codex"]);
        // Not probed yet is said, not guessed.
        assert!(rows.iter().all(|r| r.probe == Probe::Asking));
        assert!(rows.iter().all(|r| r.doing.is_empty()));
    }

    #[test]
    fn a_probe_answers_ready_or_missing_with_what_it_said() {
        let probes = [
            agent("codex", true, "codex 0.41.0"),
            agent("agy", false, "not installed"),
        ];
        let rows = rows(&["agy".into(), "codex".into()], &probes, &[], None);
        assert_eq!(rows[0].probe, Probe::Missing("not installed".into()));
        assert_eq!(rows[1].probe, Probe::Ready("codex 0.41.0".into()));
    }

    /// An installed CLI with no profile here is offered elsewhere, and is not
    /// part of this workspace's agents.
    #[test]
    fn a_cli_with_no_profile_here_is_not_a_row() {
        let mut stray = agent("grok", true, "grok 1.0");
        stray.configured = false;
        let rows = rows(&["codex".into()], &[stray], &[], None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].probe, Probe::Asking);
    }

    /// Doing is read from the phase. Writing while the author runs, reviewing
    /// while the reviewer runs, and idle in between, when no agent is.
    #[test]
    fn what_a_profile_is_doing_follows_the_phase_of_each_run() {
        let profiles = ["writer".into(), "reader".into()];
        let at = |phase| {
            rows(
                &profiles,
                &[],
                &[("t1".into(), live("writer", "reader", phase))],
                None,
            )
        };

        let authoring = at(Phase::Authoring);
        let writer = authoring.iter().find(|r| r.id == "writer").expect("writer");
        let reader = authoring.iter().find(|r| r.id == "reader").expect("reader");
        assert_eq!(writer.doing, [Doing::Writing("t1".into())]);
        assert!(reader.doing.is_empty(), "the reviewer is not reviewing yet");

        let reviewing = at(Phase::Reviewing);
        let writer = reviewing.iter().find(|r| r.id == "writer").expect("writer");
        let reader = reviewing.iter().find(|r| r.id == "reader").expect("reader");
        assert!(writer.doing.is_empty(), "the author has finished");
        assert_eq!(reader.doing, [Doing::Reviewing("t1".into())]);

        for phase in [Phase::Isolating, Phase::Preparing, Phase::Gating] {
            assert!(
                at(phase).iter().all(|r| r.doing.is_empty()),
                "{phase:?}: no agent runs in this phase"
            );
        }
    }

    /// Runs in every pane and every worker, a profile in several of them at
    /// once, and each one named.
    #[test]
    fn a_profile_can_be_in_several_runs_at_once() {
        let runs = [
            (
                "t1".to_string(),
                live("codex", "claude-code", Phase::Authoring),
            ),
            (
                "t2".to_string(),
                live("claude-code", "codex", Phase::Reviewing),
            ),
            (
                "t3".to_string(),
                live("codex", "claude-code", Phase::Authoring),
            ),
        ];
        let rows = rows(&["claude-code".into(), "codex".into()], &[], &runs, None);
        let codex = rows.iter().find(|r| r.id == "codex").expect("codex");
        assert_eq!(
            codex.doing,
            [
                Doing::Writing("t1".into()),
                Doing::Reviewing("t2".into()),
                Doing::Writing("t3".into()),
            ]
        );
        let claude = rows
            .iter()
            .find(|r| r.id == "claude-code")
            .expect("claude-code");
        assert!(claude.doing.is_empty(), "{claude:?}");
    }

    #[test]
    fn the_pair_a_run_would_use_is_marked_and_nothing_else_is() {
        let next = ("codex".to_string(), "claude-code".to_string());
        let marked = rows(
            &["agy".into(), "claude-code".into(), "codex".into()],
            &[],
            &[],
            Some(&next),
        );
        let marks: Vec<(&str, bool, bool)> = marked
            .iter()
            .map(|r| (r.id.as_str(), r.writes_next, r.reviews_next))
            .collect();
        assert_eq!(
            marks,
            [
                ("agy", false, false),
                ("claude-code", false, true),
                ("codex", true, false),
            ]
        );
        // Where no pair could be made, nothing is marked.
        assert!(
            rows(&["codex".into()], &[], &[], None)
                .iter()
                .all(|r| !r.writes_next && !r.reviews_next)
        );
    }
}
