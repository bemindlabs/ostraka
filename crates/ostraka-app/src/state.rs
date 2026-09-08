//! What the window knows, with no window involved.
//!
//! Every decision the application makes lives here and is tested without a
//! display: which runs match a filter, what is selected, what has been loaded,
//! whether a run can be promoted. The drawing code reads this and renders it,
//! the same division the terminal browser uses — and for the same reason. A
//! desktop application is a second view over one set of records, not a second
//! implementation of what a run means.

use ostraka_core::record::{Event, RunRecord};
use ostraka_runtime::index::{self, BackendUsage, RunSummary};
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{Error, Result, orchestrator};
use std::path::{Path, PathBuf};

/// Which part of a run the detail side is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    #[default]
    Checks,
    Events,
    Diff,
}

impl Detail {
    pub const ALL: [Detail; 3] = [Detail::Checks, Detail::Events, Detail::Diff];

    pub fn title(self) -> &'static str {
        match self {
            Self::Checks => "Checks",
            Self::Events => "Events",
            Self::Diff => "Diff",
        }
    }
}

pub struct State {
    pub project: PathBuf,
    /// Every run the index returned, newest first.
    pub runs: Vec<RunSummary>,
    /// Indices into `runs` that match the filter; the selection indexes this.
    pub matching: Vec<usize>,
    pub selected: Option<usize>,
    pub filter: String,
    pub detail: Detail,
    /// Loaded for the selection only, and dropped when it changes.
    pub record: Option<RunRecord>,
    pub events: Vec<Event>,
    /// `None` until asked for; `Some(None)` once asked and there was none.
    pub diff: Option<Option<String>>,
    pub status: Option<String>,
}

impl State {
    pub fn load(project: &Path) -> Result<Self> {
        let runs = index::list(&project.join(".ostraka"))?;
        let mut state = Self {
            project: project.to_path_buf(),
            runs,
            matching: Vec::new(),
            selected: None,
            filter: String::new(),
            detail: Detail::default(),
            record: None,
            events: Vec::new(),
            diff: None,
            status: None,
        };
        state.refilter();
        Ok(state)
    }

    /// Re-reads the records from disk, keeping the filter and the selected run.
    ///
    /// By run id rather than by position: a new run appears at the top and would
    /// otherwise shift the selection onto a different run than the one someone
    /// was reading.
    pub fn reload(&mut self) {
        let was = self.selected_run().map(|r| r.run_id.clone());
        match index::list(&self.project.join(".ostraka")) {
            Ok(runs) => self.runs = runs,
            Err(e) => {
                self.status = Some(format!("could not reload: {e}"));
                return;
            }
        }
        self.refilter();
        if let Some(id) = was {
            self.select_by_id(&id);
        }
        self.status = Some(format!("reloaded — {} run(s)", self.runs.len()));
    }

    pub fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        let previous = self.selected_run().map(|r| r.run_id.clone());
        self.matching = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| matches(run, &needle))
            .map(|(i, _)| i)
            .collect();

        // Keep the same run selected if it survived the filter, otherwise fall
        // to the first match rather than to whatever now sits at that index.
        self.selected = None;
        match previous {
            Some(id) => {
                self.select_by_id(&id);
                if self.selected.is_none() && !self.matching.is_empty() {
                    self.select(0);
                }
            }
            None if !self.matching.is_empty() => self.select(0),
            None => {}
        }
    }

    pub fn select(&mut self, position: usize) {
        if position >= self.matching.len() {
            return;
        }
        if self.selected == Some(position) {
            return;
        }
        self.selected = Some(position);
        self.forget_detail();
    }

    fn select_by_id(&mut self, run_id: &str) {
        if let Some(position) = self
            .matching
            .iter()
            .position(|i| self.runs.get(*i).is_some_and(|r| r.run_id == run_id))
        {
            self.selected = Some(position);
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.matching.is_empty() {
            return;
        }
        let last = self.matching.len() - 1;
        let next = self
            .selected
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(last);
        self.select(next);
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        self.runs.get(*self.matching.get(self.selected?)?)
    }

    pub fn show(&mut self, detail: Detail) {
        self.detail = detail;
    }

    fn forget_detail(&mut self) {
        self.record = None;
        self.events.clear();
        self.diff = None;
    }

    /// Loads whatever the current view needs and has not got yet.
    ///
    /// Called every frame, so it must do nothing when there is nothing to do:
    /// the diff costs a git call and the record costs a read, and repeating
    /// either sixty times a second would be a busy loop with a window on it.
    pub fn ensure_loaded(&mut self) {
        let Some(run_id) = self.selected_run().map(|r| r.run_id.clone()) else {
            return;
        };
        if self.record.is_none() {
            if let Ok((record, events)) =
                orchestrator::replay(&self.project.join(".ostraka"), &run_id)
            {
                self.record = Some(record);
                self.events = events;
            }
        }
        if self.detail == Detail::Diff && self.diff.is_none() {
            self.diff = Some(index::diff(&self.project, &run_id).ok().flatten());
        }
    }

    /// Totals for the runs currently listed, per backend.
    pub fn backends(&self) -> Vec<BackendUsage> {
        let listed: Vec<RunSummary> = self
            .matching
            .iter()
            .filter_map(|i| self.runs.get(*i))
            .cloned()
            .collect();
        index::by_backend(&listed)
    }

    /// Whether promoting the selection could possibly succeed.
    ///
    /// Only a hint for the button's enabled state. The gate decides, and it
    /// decides again inside `promote` — this cannot approve anything.
    pub fn can_promote(&self) -> bool {
        self.selected_run().is_some_and(RunSummary::approved)
    }

    /// Promotes the selected run, reporting the outcome as a status message.
    pub fn promote_selected(&mut self) {
        let Some(run) = self.selected_run().map(|r| r.run_id.clone()) else {
            self.status = Some("nothing selected".to_string());
            return;
        };
        self.status = Some(self.promote_run(&run));
    }

    fn promote_run(&self, run_id: &str) -> String {
        let config = match load_config(&self.project) {
            Ok(config) => config,
            Err(e) => return format!("could not read ostraka.toml: {e}"),
        };
        let records = self.project.join(".ostraka");
        match promote::promote(&self.project, &records, run_id, &config, None) {
            Ok(Ok(p)) => format!("promoted to {} — nothing merged", p.branch),
            Ok(Err(NotPromoted::Refused(r))) => format!("the gate refuses this run: {r:?}"),
            Ok(Err(why)) => format!("not promoted — {why}"),
            Err(e) => format!("not promoted — {e}"),
        }
    }
}

fn matches(run: &RunSummary, needle: &str) -> bool {
    needle.is_empty()
        || run.prompt.to_lowercase().contains(needle)
        || run.run_id.to_lowercase().contains(needle)
        || outcome_word(run).contains(needle)
}

pub fn outcome_word(run: &RunSummary) -> &'static str {
    use ostraka_core::record::Outcome;
    match run.outcome {
        Some(Outcome::Approved) => "approved",
        Some(Outcome::Rejected) => "refused",
        Some(Outcome::Failed) => "failed",
        None => "unfinished",
    }
}

fn load_config(project: &Path) -> Result<ostraka_core::config::Config> {
    let path = project.join("ostraka.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Other(format!("{}: {e}", path.display())))?;
    Ok(ostraka_core::config::Config::parse(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::identity::ActorId;
    use ostraka_core::record::Outcome;

    fn summary(run_id: &str, prompt: &str, outcome: Option<Outcome>) -> RunSummary {
        RunSummary {
            run_id: run_id.to_string(),
            started_at: String::new(),
            prompt: prompt.to_string(),
            author: ActorId::new("archon"),
            adapter: "claude-code".to_string(),
            repository: "only".to_string(),
            reviewer: Some(ActorId::new("ephor")),
            outcome,
            checks_passed: 1,
            checks_total: 1,
            usage: Vec::new(),
        }
    }

    fn state(runs: Vec<RunSummary>) -> State {
        let mut s = State {
            project: PathBuf::from("/p"),
            runs,
            matching: Vec::new(),
            selected: None,
            filter: String::new(),
            detail: Detail::Checks,
            record: None,
            events: Vec::new(),
            diff: None,
            status: None,
        };
        s.refilter();
        s
    }

    #[test]
    fn the_first_run_is_selected_when_there_is_one() {
        let s = state(vec![summary("t1-1", "one", Some(Outcome::Approved))]);
        assert_eq!(s.selected_run().map(|r| r.prompt.as_str()), Some("one"));
    }

    #[test]
    fn an_empty_project_selects_nothing_rather_than_index_zero() {
        let s = state(Vec::new());
        assert!(s.selected.is_none());
        assert!(s.selected_run().is_none());
    }

    #[test]
    fn a_filter_keeps_the_selected_run_when_it_survives() {
        // Selection follows the run, not the row it happened to be on.
        let mut s = state(vec![
            summary("t1-1", "alpha", Some(Outcome::Approved)),
            summary("t2-2", "beta", Some(Outcome::Approved)),
        ]);
        s.select(1);
        assert_eq!(s.selected_run().map(|r| r.prompt.as_str()), Some("beta"));
        s.filter = "bet".into();
        s.refilter();
        assert_eq!(s.selected_run().map(|r| r.prompt.as_str()), Some("beta"));
    }

    #[test]
    fn a_filter_that_hides_the_selection_falls_to_the_first_match() {
        let mut s = state(vec![
            summary("t1-1", "alpha", Some(Outcome::Approved)),
            summary("t2-2", "beta", Some(Outcome::Approved)),
        ]);
        s.select(1);
        s.filter = "alph".into();
        s.refilter();
        assert_eq!(s.selected_run().map(|r| r.prompt.as_str()), Some("alpha"));
    }

    #[test]
    fn a_filter_matching_nothing_selects_nothing() {
        let mut s = state(vec![summary("t1-1", "alpha", Some(Outcome::Approved))]);
        s.filter = "no such run".into();
        s.refilter();
        assert!(s.selected_run().is_none());
        assert!(s.matching.is_empty());
    }

    #[test]
    fn the_outcome_word_is_filterable() {
        let mut s = state(vec![
            summary("t1-1", "one", Some(Outcome::Approved)),
            summary("t2-2", "two", Some(Outcome::Rejected)),
        ]);
        s.filter = "refused".into();
        s.refilter();
        assert_eq!(s.matching.len(), 1);
        assert_eq!(s.selected_run().map(|r| r.prompt.as_str()), Some("two"));
    }

    #[test]
    fn changing_the_selection_drops_what_was_loaded_for_the_old_one() {
        // Otherwise the detail side shows one run's checks beside another's id.
        let mut s = state(vec![
            summary("t1-1", "alpha", Some(Outcome::Approved)),
            summary("t2-2", "beta", Some(Outcome::Approved)),
        ]);
        s.diff = Some(Some("a diff".into()));
        s.select(1);
        assert!(s.diff.is_none());
    }

    #[test]
    fn reselecting_the_same_run_keeps_what_was_already_loaded() {
        // Called from a frame loop, so this must not re-read on every frame.
        let mut s = state(vec![summary("t1-1", "alpha", Some(Outcome::Approved))]);
        s.diff = Some(Some("a diff".into()));
        s.select(0);
        assert_eq!(s.diff, Some(Some("a diff".into())));
    }

    #[test]
    fn only_an_approved_run_offers_promotion() {
        let approved = state(vec![summary("t1-1", "one", Some(Outcome::Approved))]);
        assert!(approved.can_promote());
        let refused = state(vec![summary("t1-1", "one", Some(Outcome::Rejected))]);
        assert!(!refused.can_promote());
        let unfinished = state(vec![summary("t1-1", "one", None)]);
        assert!(!unfinished.can_promote());
    }

    #[test]
    fn moving_stays_inside_the_list() {
        let mut s = state(vec![summary("t1-1", "a", None), summary("t2-2", "b", None)]);
        s.move_by(-1);
        assert_eq!(s.selected, Some(0));
        s.move_by(9);
        assert_eq!(s.selected, Some(1));
        s.move_by(-9);
        assert_eq!(s.selected, Some(0));
    }

    #[test]
    fn moving_in_an_empty_list_does_nothing() {
        let mut s = state(Vec::new());
        s.move_by(1);
        assert!(s.selected.is_none());
    }
}
