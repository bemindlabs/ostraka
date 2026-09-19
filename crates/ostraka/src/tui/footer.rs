//! What the footer says, worked out apart from drawing it.
//!
//! The footer is aggregate: how the listed runs ended, what is waiting and
//! going, what the last run came to, and the one thing most worth doing next.
//! The side pane is per profile, and neither repeats the other.
//!
//! Everything here is a function of state already read off disk elsewhere, on
//! the tick, so nothing is read while drawing. A suggestion is derived from that
//! state and from what `remedy.rs` diagnoses — never from the words of an error,
//! which change when the program that wrote them does. Where nothing is clearly
//! worth doing, there is no suggestion: a weak guess on the bottom row teaches
//! people to stop reading it.

use crate::mode::Mode;
use ostraka_core::gate::Verdict;
use ostraka_core::record::{Outcome, RunRecord};
use ostraka_runtime::index::RunSummary;

/// The facts the footer is drawn from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub approved: usize,
    pub rejected: usize,
    pub failed: usize,
    /// Tasks on the list that nobody has taken.
    pub waiting: usize,
    /// Runs in progress in any process, `drain` workers included.
    pub going: usize,
    /// Approved runs whose branch is still there and was never promoted.
    pub unpromoted: usize,
    /// The newest listed run, in a phrase's worth of facts.
    pub last: Option<Last>,
}

/// How the newest run ended, as far as a phrase can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Last {
    Approved,
    /// A required check failed; the first one that did.
    RefusedByCheck(String),
    /// Every check passed and the reviewer said no.
    RefusedByReviewer(String),
    /// Refused, with nothing on record saying by what.
    Refused,
    /// The agent or the runtime failed before there was a verdict.
    Failed,
    /// No outcome recorded: still going, or its process ended without one.
    Unfinished,
}

impl Last {
    /// Read off the run's summary and, where there is one, its record.
    ///
    /// The record says which check failed and who rejected. Without it, the
    /// summary still says the outcome, and the phrase says only that much.
    pub fn of(summary: &RunSummary, record: Option<&RunRecord>) -> Last {
        match summary.outcome {
            None => Last::Unfinished,
            Some(Outcome::Approved) => Last::Approved,
            Some(Outcome::Failed) => Last::Failed,
            Some(Outcome::Rejected) => {
                let Some(record) = record.filter(|r| r.run_id == summary.run_id) else {
                    return Last::Refused;
                };
                if let Some(check) = record.checks.iter().find(|c| !c.passed()) {
                    return Last::RefusedByCheck(check.name.clone());
                }
                match &record.approval {
                    Some(approval) if matches!(approval.verdict, Verdict::Reject { .. }) => {
                        Last::RefusedByReviewer(approval.reviewer.as_str().to_string())
                    }
                    _ => Last::Refused,
                }
            }
        }
    }

    /// The phrase: what happened to the last run, in a few words.
    pub fn phrase(&self) -> String {
        match self {
            Last::Approved => "last run approved".to_string(),
            Last::RefusedByCheck(check) => format!("last run refused by the {check} check"),
            Last::RefusedByReviewer(reviewer) => {
                format!("last run refused by the reviewer, {reviewer}")
            }
            Last::Refused => "last run refused".to_string(),
            Last::Failed => "last run could not finish".to_string(),
            Last::Unfinished => "last run has no outcome yet".to_string(),
        }
    }
}

/// How the listed runs ended.
pub fn outcomes(runs: &[&RunSummary]) -> (usize, usize, usize) {
    runs.iter()
        .fold((0, 0, 0), |(a, r, f), run| match run.outcome {
            Some(Outcome::Approved) => (a + 1, r, f),
            Some(Outcome::Rejected) => (a, r + 1, f),
            Some(Outcome::Failed) => (a, r, f + 1),
            None => (a, r, f),
        })
}

/// The facts, from what the browser already holds.
pub fn facts(
    listed: &[&RunSummary],
    waiting: usize,
    going: usize,
    unpromoted: &std::collections::HashSet<String>,
    last_record: Option<&RunRecord>,
) -> Facts {
    let (approved, rejected, failed) = outcomes(listed);
    Facts {
        approved,
        rejected,
        failed,
        waiting,
        going,
        // Of the listed runs only, like the outcomes beside it.
        unpromoted: listed
            .iter()
            .filter(|run| unpromoted.contains(&run.run_id))
            .count(),
        last: listed.first().map(|newest| Last::of(newest, last_record)),
    }
}

/// Approved runs still waiting to be promoted, from the branches a repository
/// has: a run's own branch `ostraka/<run>` still there, and no
/// `promoted/<run>`. A run pruned after its change was merged has neither, and
/// is not waiting on anybody.
pub fn unpromoted(approved: &[&str], branches: &[String]) -> Vec<String> {
    let has = |name: String| branches.contains(&name);
    approved
        .iter()
        .filter(|run| has(format!("ostraka/{run}")) && !has(format!("promoted/{run}")))
        .map(|run| run.to_string())
        .collect()
}

/// The state a suggestion is chosen from.
#[derive(Debug, Clone, Default)]
pub struct Situation<'a> {
    pub facts: Facts,
    /// `remedy.rs` found something in the way of a run here.
    pub blocked: bool,
    /// The run selected in the list is approved and not yet promoted.
    pub selected_unpromoted: Option<&'a str>,
    /// The mode the thread in front runs in.
    pub mode: Mode,
}

/// The one next action worth naming, and which command takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggestion {
    /// Something is in the way; the fix command walks the steps.
    Fix,
    /// The selected run is approved and nobody has promoted it.
    Promote(String),
    /// Approved runs wait for promotion, and none of them is selected.
    OpenRuns(usize),
    /// A check refused the last run, and loop mode retries with the reason.
    Loop,
    /// Tasks are waiting and nothing is taking them.
    Drain(usize),
}

/// The single most useful next action, or none.
///
/// First match wins, in the order the brief ranks them: what blocks every run,
/// then finished work waiting on a person, then the last refusal, then the
/// queue. The setup screen has its own footer and is not handled here.
pub fn suggest(state: &Situation) -> Option<Suggestion> {
    let facts = &state.facts;
    if state.blocked {
        return Some(Suggestion::Fix);
    }
    if let Some(run) = state.selected_unpromoted {
        return Some(Suggestion::Promote(run.to_string()));
    }
    if facts.unpromoted > 0 {
        return Some(Suggestion::OpenRuns(facts.unpromoted));
    }
    if matches!(facts.last, Some(Last::RefusedByCheck(_))) && state.mode != Mode::Loop {
        return Some(Suggestion::Loop);
    }
    if facts.waiting > 0 && facts.going == 0 {
        return Some(Suggestion::Drain(facts.waiting));
    }
    None
}

/// The counts, each only where it is not zero.
pub fn counts(facts: &Facts) -> Vec<String> {
    let mut said = Vec::new();
    for (n, word) in [
        (facts.approved, "approved"),
        (facts.rejected, "rejected"),
        (facts.failed, "failed"),
    ] {
        if n > 0 {
            said.push(format!("{n} {word}"));
        }
    }
    if facts.going > 0 {
        said.push(format!("{} going", facts.going));
    }
    if facts.waiting > 0 {
        said.push(format!("{} waiting", facts.waiting));
    }
    if facts.unpromoted > 0 {
        said.push(format!("{} not promoted", facts.unpromoted));
    }
    said
}

/// Which segments of a row fit in `width`, in priority order.
///
/// A segment fits whole or is dropped, and so is everything after it: a row
/// that skipped a segment to squeeze a later one in would reorder priorities
/// by width. The first segment alone may be cut, because it is the one that
/// has to show. Segments are joined by `gap` cells.
pub fn fit(widths: &[usize], width: usize, gap: usize) -> (usize, Option<usize>) {
    let mut used = 0;
    for (i, w) in widths.iter().enumerate() {
        let need = if i == 0 { *w } else { used + gap + w };
        if need > width {
            return if i == 0 { (1, Some(width)) } else { (i, None) };
        }
        used = need;
    }
    (widths.len(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::identity::ActorId;

    fn summary(run: &str, outcome: Option<Outcome>) -> RunSummary {
        RunSummary {
            run_id: run.into(),
            started_at: String::new(),
            prompt: String::new(),
            author: ActorId::new("a"),
            adapter: "w".into(),
            repository: "r".into(),
            reviewer: None,
            outcome,
            checks_passed: 0,
            checks_total: 0,
            usage: Vec::new(),
        }
    }

    fn situation(facts: Facts) -> Situation<'static> {
        Situation {
            facts,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_worth_doing_suggests_nothing() {
        assert_eq!(suggest(&situation(Facts::default())), None);
        let quiet = Facts {
            approved: 3,
            last: Some(Last::Approved),
            ..Default::default()
        };
        assert_eq!(suggest(&situation(quiet)), None);
    }

    #[test]
    fn something_in_the_way_comes_before_everything() {
        let state = Situation {
            blocked: true,
            selected_unpromoted: Some("t1"),
            facts: Facts {
                waiting: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(suggest(&state), Some(Suggestion::Fix));
    }

    #[test]
    fn an_approved_run_not_promoted_is_promoted_where_it_is_selected() {
        let state = Situation {
            selected_unpromoted: Some("t1"),
            facts: Facts {
                unpromoted: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(suggest(&state), Some(Suggestion::Promote("t1".into())));

        // Not selected: `p` would promote the wrong run, so the list is named.
        let elsewhere = situation(Facts {
            unpromoted: 2,
            ..Default::default()
        });
        assert_eq!(suggest(&elsewhere), Some(Suggestion::OpenRuns(2)));
    }

    #[test]
    fn a_run_a_check_refused_suggests_loop_mode_unless_already_in_it() {
        let refused = Facts {
            last: Some(Last::RefusedByCheck("test".into())),
            ..Default::default()
        };
        assert_eq!(suggest(&situation(refused.clone())), Some(Suggestion::Loop));
        let looping = Situation {
            mode: Mode::Loop,
            ..situation(refused)
        };
        assert_eq!(suggest(&looping), None);

        // A reviewer's no is not something a retry with the check's output
        // answers.
        let by_reviewer = Facts {
            last: Some(Last::RefusedByReviewer("ephor".into())),
            ..Default::default()
        };
        assert_eq!(suggest(&situation(by_reviewer)), None);
    }

    #[test]
    fn waiting_tasks_suggest_a_drain_only_when_nothing_is_taking_them() {
        let waiting = Facts {
            waiting: 3,
            ..Default::default()
        };
        assert_eq!(
            suggest(&situation(waiting.clone())),
            Some(Suggestion::Drain(3))
        );
        let taken = Facts {
            going: 1,
            ..waiting
        };
        assert_eq!(suggest(&situation(taken)), None);
    }

    #[test]
    fn the_last_run_is_read_from_its_record() {
        use ostraka_core::gate::{Approval, CheckRecord};
        let refused = summary("t1", Some(Outcome::Rejected));
        let mut record = RunRecord {
            run_id: "t1".into(),
            task_id: "t".into(),
            prompt: String::new(),
            author: ActorId::new("a"),
            adapter: "w".into(),
            repository: "r".into(),
            started_at: String::new(),
            finished_at: None,
            checks: vec![CheckRecord {
                name: "test".into(),
                cmd: "false".into(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 1,
            }],
            approval: None,
            usage: Vec::new(),
            outcome: Some(Outcome::Rejected),
        };
        assert_eq!(
            Last::of(&refused, Some(&record)),
            Last::RefusedByCheck("test".into())
        );

        record.checks[0].exit_code = Some(0);
        record.approval = Some(Approval {
            reviewer: ActorId::new("ephor"),
            verdict: Verdict::Reject {
                reason: "no".into(),
            },
        });
        assert_eq!(
            Last::of(&refused, Some(&record)),
            Last::RefusedByReviewer("ephor".into())
        );
        // Another run's record says nothing about this one.
        assert_eq!(
            Last::of(&summary("t2", Some(Outcome::Rejected)), Some(&record)),
            Last::Refused
        );
        assert_eq!(Last::of(&summary("t3", None), None), Last::Unfinished);
        assert_eq!(
            Last::RefusedByCheck("test".into()).phrase(),
            "last run refused by the test check"
        );
    }

    #[test]
    fn a_run_is_waiting_for_promotion_only_while_its_branch_is_there() {
        let branches = vec![
            "ostraka/t1".to_string(),
            "ostraka/t2".to_string(),
            "promoted/t2".to_string(),
        ];
        // t3's branch was pruned after its change was merged.
        assert_eq!(unpromoted(&["t1", "t2", "t3"], &branches), ["t1"]);
    }

    #[test]
    fn counts_say_only_what_is_not_zero() {
        let facts = Facts {
            approved: 2,
            failed: 1,
            waiting: 4,
            ..Default::default()
        };
        assert_eq!(counts(&facts), ["2 approved", "1 failed", "4 waiting"]);
        assert!(counts(&Facts::default()).is_empty());
        assert_eq!(
            outcomes(&[
                &summary("a", Some(Outcome::Approved)),
                &summary("b", Some(Outcome::Rejected)),
                &summary("c", None),
            ]),
            (1, 1, 0)
        );
    }

    /// Dropped from the end, never skipped over, and only the first cut.
    #[test]
    fn a_row_drops_from_the_end_and_cuts_only_its_first_segment() {
        assert_eq!(fit(&[10, 5, 5], 30, 3), (3, None));
        assert_eq!(fit(&[10, 5, 5], 20, 3), (2, None));
        // The third would fit alone in what the second leaves, but is behind it.
        assert_eq!(fit(&[10, 30, 2], 20, 3), (1, None));
        assert_eq!(fit(&[40, 5], 20, 3), (1, Some(20)));
        assert_eq!(fit(&[], 20, 3), (0, None));
    }
}
