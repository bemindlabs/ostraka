//! What the browser shows, as a function of what it knows.
//!
//! Drawing is pure: state in, cells out. That is what lets it be tested against
//! a buffer rather than looked at — a screen nobody asserts on is a screen that
//! quietly stops saying what it used to.
//!
//! Two screens, and the difference is what you are doing. **Work** is the
//! thread: what you asked, what happened, and a box to ask the next thing —
//! full width, because that is the thing you are reading. **Record** is one run
//! out of the history, looked up. The list of runs is a dialog rather than a
//! column, because a column costs half the width of the screen to show
//! something you look at once every twenty minutes.
//!
//! Regions are divided by rules, not by space alone. Four regions separated
//! only by gaps read as one region with holes in it, and the eye re-derives the
//! boundaries every time it looks.
//!
//! Colour carries meaning here rather than decoration. Who is speaking is a
//! colour — the runtime is muted, the author is the accent, the gate is the
//! colour of something being tested, the reviewer is its own — so a transcript
//! can be scanned for "what did the reviewer say" without reading it.

use crate::chord::{Action as Chord, label, panes_label};
use crate::init::{Action, Plan};
use crate::mode::Mode;
use crate::remedy::Remedy;
use crate::tui::command::{Command, Situation};
use crate::tui::mention::{self, Candidate, Mentionable};
use crate::tui::pane::{self, Pane};
use crate::tui::theme;
use crate::tui::thread::{Thread, Turn};
use crate::workspace::{Repository, Workspace};
use ostraka_core::gate::{CheckRecord, Verdict};
use ostraka_core::record::{Event, Outcome, RunRecord};
use ostraka_runtime::index::{self, BackendUsage, RunSummary};
use ostraka_runtime::progress::{Phase, Step};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};
use std::path::Path;

/// Rows the chrome takes at its smallest: the breadcrumb, its rule, the three
/// of the input box and the status line.
const CHROME: u16 = 6;

/// How tall the input box is allowed to grow before it scrolls instead.
const PROMPT_LINES: usize = 6;

/// Which part of a run the record screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Checks,
    Events,
    Diff,
}

impl Detail {
    pub const ALL: [Detail; 3] = [Detail::Checks, Detail::Events, Detail::Diff];

    pub fn next(self) -> Self {
        match self {
            Self::Checks => Self::Events,
            Self::Events => Self::Diff,
            Self::Diff => Self::Checks,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Checks => "checks",
            Self::Events => "events",
            Self::Diff => "diff",
        }
    }
}

/// What the browser is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The thread: what has been asked here and what came of it.
    Work,
    /// One run out of the history.
    Record,
}

/// Where a keystroke goes.
///
/// Work opens on the prompt, because the first thing anybody does here is say
/// what they want. A screen you have to unlock before you can type into it is
/// a screen that makes you press a key to do the obvious thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Prompt,
    Keys,
}

/// The one thing that can be open over the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Keys,
    Commands,
    Runs,
    Agents,
    Leaving,
    Fix,
    Settings,
    Repos,
    /// What finished runs left, and whether to clear it.
    Prune,
    /// The models each profile lists, to pick the one that writes.
    Models,
    /// A vendor could not run, and the other profiles that answer.
    Fallback,
}

/// One adapter profile, as the agents dialog shows it.
pub struct Agent {
    pub id: String,
    pub ready: bool,
    pub note: String,
    /// This workspace has a profile for it. A false here is a CLI that is
    /// installed and has none — offered so that an empty list stops being the
    /// answer to "which agents can I use", and written when one is chosen.
    pub configured: bool,
}

pub struct App {
    pub workspace: Workspace,
    /// The lines of work open here, and which one has the screen.
    pub panes: Vec<Pane>,
    pub at: usize,
    /// The project path as the breadcrumb says it: resolved, and shortened to
    /// `~` where it sits under the operator's home.
    ///
    /// Worked out once, when the browser opens, rather than every frame.
    /// Drawing is a pure function of state, and a `canonicalize` per frame
    /// would make it a function of the filesystem as well.
    pub where_shown: String,
    /// Every run, in the order the index gave them.
    pub runs: Vec<RunSummary>,
    /// Indices into `runs` that match the filter. The selection indexes this.
    pub matching: Vec<usize>,
    pub selected: usize,
    /// First entry the runs dialog is showing, so a selection below the fold
    /// scrolls rather than disappearing off it.
    pub list_top: usize,
    /// What has been typed into the runs dialog.
    pub filter: String,
    /// The full record of the selected run, loaded on demand: a listing holds
    /// summaries, and check output is the thing worth reading on a failure.
    pub record: Option<RunRecord>,
    pub events: Vec<Event>,
    /// `None` until asked for; `Some(None)` once asked and not found.
    pub diff: Option<Option<String>>,
    pub detail: Detail,
    /// The adapter profiles this project has, probed when first asked for.
    /// Not at startup: probing runs every vendor's binary, and a browser that
    /// took three seconds to open would be a browser nobody left open.
    pub agents: Vec<Agent>,
    pub screen: Screen,
    pub focus: Focus,
    pub scroll: u16,
    /// Height of the content at the last draw, so a page key can move by a
    /// page rather than by a number somebody guessed.
    pub page: u16,
    pub status: Option<String>,
    /// Present when this directory is not a project yet: what `init` would
    /// write. `None` once there is nothing left to write.
    pub setup: Option<Plan>,
    /// The worktrees `Command::Prune` found, held while the dialog asks.
    pub leftovers: Vec<crate::prune::Leftover>,
    /// Why a run cannot work here, when something is in the way that setting
    /// the project up does not fix. `None` means nothing known is wrong.
    pub blocked: Option<String>,
    /// The steps out of it, once somebody has asked to be walked through them.
    pub remedy: Option<Remedy>,
    /// What this project is configured to do, read when settings are opened.
    pub project_facts: Vec<(String, String)>,
    /// The settings row being edited, and what has been typed into it.
    pub editing: Option<String>,
    pub dialog: Option<Dialog>,
    /// What has been typed into the command palette.
    pub query: String,
    /// Which command the palette has selected.
    pub pick: usize,
    /// The leader key has been pressed and the next one completes a command.
    pub leader: bool,
    /// Someone asked to leave while a run was going. The browser stays up
    /// until that run has actually stopped.
    pub leaving: bool,
    /// Ticks of the event loop. What the spinner is a function of, so that
    /// drawing is a function of state rather than of the clock.
    pub tick: u64,
    pub quit: bool,
    /// Where the task's text was last drawn, and how far down the box was
    /// scrolled, so a click can be read against what was actually on screen.
    pub prompt_text: Rect,
    pub prompt_scroll: u16,
    /// Which pane each column was drawn for, and where, at the last draw.
    /// Empty while one pane has the screen.
    pub columns: Vec<(usize, Rect)>,
    /// Where each pane's transcript was drawn, and how far it was scrolled,
    /// at the last draw. What a drag is read against: a screen row means a
    /// line of the transcript only together with the scroll it was drawn at.
    /// Recorded for the single-pane screen as well as for columns.
    pub transcripts: Vec<(usize, Rect, u16)>,
    /// Text selected in a pane with the mouse, waiting to be copied.
    pub selection: Option<super::select::Selection>,
    /// How far the keys dialog is scrolled. It lists more keys than fit on
    /// most terminals, and truncating it hid the ones somebody came for.
    pub keys_scroll: u16,
    /// The agents beside the work are wanted. On unless somebody hid them.
    pub side: bool,
    /// The last draw had room for them. Nothing is probed or read for a pane
    /// nobody can see.
    pub side_shown: bool,
    /// This workspace's profile ids, read when the browser opens.
    pub profile_ids: Vec<String>,
    /// What each profile's probe answered. Taken once, on a thread, and held:
    /// probing runs every CLI, and doing that per frame would be the browser
    /// launching vendors four times a second.
    pub probes: Vec<Agent>,
    pub probes_loading: Option<std::sync::mpsc::Receiver<Vec<Agent>>>,
    /// Every run in progress, from this process or any other, as the runtime
    /// reports it. Read about once a second.
    pub live: Vec<(String, ostraka_runtime::record::Live)>,
    /// The pair a run started now would use, the thread choice it was worked
    /// out for, and the working-out while it happens.
    pub next_pair: Option<(String, String)>,
    pub next_pair_for: Option<(Option<String>, Option<String>)>,
    pub next_pair_loading: Option<std::sync::mpsc::Receiver<Option<(String, String)>>>,
    /// The task list by repository, read on the same tick as `live`.
    pub task_groups: Vec<super::roster::TaskGroup>,
    /// Repositories somebody closed in the pane, by name. Kept across reads,
    /// so a group stays closed while its tasks come and go.
    pub tasks_closed: std::collections::HashSet<String>,
    /// Where each repository's header was last drawn, for a click to find.
    pub task_headers: Vec<(Rect, String)>,
    /// Approved runs whose branch is still there and was never promoted,
    /// read from each repository's branches on the tick.
    pub unpromoted: std::collections::HashSet<String>,
    /// The newest listed run's record, read when the newest run changes: it
    /// is what says which check or which reviewer refused it.
    pub last_record: Option<ostraka_core::record::RunRecord>,
    /// The first pane shown when there are more panes than columns.
    pub first: usize,
    /// What `@` can name here: this repository's paths and the agents. Read
    /// when a mention starts, not on every frame.
    pub mentionable: Mentionable,
    /// The picker's rows: each profile that lists its models, then its models.
    pub models: Vec<crate::models::Model>,
    /// Why a profile listed less than it might have.
    pub models_notes: Vec<String>,
    /// The listing still running. Each profile's CLI is asked in turn, and a
    /// screen that froze while one fetched over the network would be a screen
    /// nobody could stop.
    pub models_loading: Option<std::sync::mpsc::Receiver<Vec<crate::models::Catalog>>>,
    /// The run a vendor could not finish, while another profile is offered.
    pub fallback: Option<super::session::Fallback>,
    /// The profiles being asked whether they answer. Probing runs each CLI.
    pub agents_loading: Option<std::sync::mpsc::Receiver<Vec<Agent>>>,
}

impl App {
    pub fn new(workspace: Workspace, runs: Vec<RunSummary>) -> Self {
        let matching = (0..runs.len()).collect();
        // Parsed, not probed: this is only which profiles exist.
        let profile_ids = workspace
            .profiles()
            .map(|profiles| profiles.into_iter().map(|p| p.id).collect())
            .unwrap_or_default();
        Self {
            where_shown: where_we_are(&workspace.root),
            workspace,
            panes: vec![Pane::default()],
            at: 0,
            runs,
            matching,
            selected: 0,
            list_top: 0,
            filter: String::new(),
            record: None,
            events: Vec::new(),
            diff: None,
            detail: Detail::Checks,
            agents: Vec::new(),
            screen: Screen::Work,
            focus: Focus::Prompt,
            scroll: 0,
            page: 10,
            status: None,
            setup: None,
            leftovers: Vec::new(),
            blocked: None,
            remedy: None,
            project_facts: Vec::new(),
            editing: None,
            dialog: None,
            query: String::new(),
            pick: 0,
            leader: false,
            leaving: false,
            tick: 0,
            quit: false,
            prompt_text: Rect::default(),
            prompt_scroll: 0,
            columns: Vec::new(),
            transcripts: Vec::new(),
            selection: None,
            keys_scroll: 0,
            side: true,
            side_shown: false,
            profile_ids,
            probes: Vec::new(),
            probes_loading: None,
            live: Vec::new(),
            next_pair: None,
            next_pair_for: None,
            next_pair_loading: None,
            task_groups: Vec::new(),
            tasks_closed: Default::default(),
            task_headers: Vec::new(),
            unpromoted: Default::default(),
            last_record: None,
            first: 0,
            mentionable: Mentionable::default(),
            models: Vec::new(),
            models_notes: Vec::new(),
            models_loading: None,
            fallback: None,
            agents_loading: None,
        }
    }

    /// The pane with the screen.
    pub fn pane(&self) -> &Pane {
        self.panes
            .get(self.at)
            .expect("a browser always has a pane")
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        let at = self.at;
        self.panes.get_mut(at).expect("a browser always has a pane")
    }

    /// The line of work on the screen.
    pub fn thread(&self) -> &Thread {
        &self.pane().thread
    }

    pub fn thread_mut(&mut self) -> &mut Thread {
        &mut self.pane_mut().thread
    }

    pub fn repository(&self) -> Option<&Repository> {
        self.pane().repository.as_ref()
    }

    /// Any pane at all. Asked on the way out: the browser waits on every run it
    /// started, not only the one in front of it.
    pub fn anything_running(&self) -> bool {
        self.panes.iter().any(Pane::running)
    }

    /// Opens another line of work, in the same repository as this one.
    ///
    /// The repository is carried over because a second pane is usually a
    /// second thing to do in the same place; `w` moves it somewhere else.
    pub fn open_pane(&mut self) {
        self.park();
        let repository = self.pane().repository.clone();
        self.panes.push(Pane::new(repository));
        self.at = self.panes.len() - 1;
        self.scroll = 0;
    }

    /// Keeps where the pane in front was scrolled to, so a column still on the
    /// screen does not jump when another pane takes the keys.
    fn park(&mut self) {
        // Only the work screen's scroll belongs to a pane. The record screen
        // scrolls the same field, and parking it would move a column to wherever
        // a record had been read to.
        if self.screen != Screen::Work {
            return;
        }
        let scroll = self.scroll;
        self.pane_mut().scroll = scroll;
    }

    /// Gives another pane the keys and moves nothing else.
    pub fn focus_pane(&mut self, index: usize) {
        if index >= self.panes.len() || index == self.at {
            return;
        }
        self.park();
        self.at = index;
        if self.screen == Screen::Work {
            self.scroll = self.pane().scroll;
        }
    }

    /// Swaps this pane with its neighbour. The keys go with it, because the
    /// pane being moved is the one somebody is arranging.
    pub fn move_pane(&mut self, delta: isize) -> bool {
        let Some(to) = self
            .at
            .checked_add_signed(delta)
            .filter(|to| *to < self.panes.len())
        else {
            return false;
        };
        self.panes.swap(self.at, to);
        self.at = to;
        true
    }

    /// Gives this pane a larger or smaller share of the width.
    pub fn resize_pane(&mut self, delta: i16) -> bool {
        let pane = self.pane_mut();
        let weight = pane
            .weight
            .saturating_add_signed(delta)
            .clamp(1, pane::MAX_WEIGHT);
        let changed = weight != pane.weight;
        pane.weight = weight;
        changed
    }

    pub fn even_panes(&mut self) {
        for pane in &mut self.panes {
            pane.weight = pane::WEIGHT;
        }
    }

    /// Moves to the next pane, wrapping.
    pub fn next_pane(&mut self, delta: isize) {
        if self.panes.len() < 2 {
            return;
        }
        self.park();
        let count = self.panes.len() as isize;
        self.at = ((self.at as isize + delta).rem_euclid(count)) as usize;
        self.scroll = 0;
    }

    /// Closes this one, unless it is the only one or something is going in it.
    ///
    /// Returns what to say when it declines. A pane closed out from under a
    /// run would abandon the thread writing into a worktree, and a browser
    /// with no panes is a browser with nothing to type into.
    pub fn close_pane(&mut self) -> Option<String> {
        if self.pane().running() {
            return Some("this pane is running \u{2014} s asks it to stop".to_string());
        }
        if self.panes.len() < 2 {
            return Some("this is the only pane".to_string());
        }
        self.panes.remove(self.at);
        self.at = self.at.min(self.panes.len() - 1);
        self.scroll = 0;
        None
    }

    pub fn current(&self) -> Option<&RunSummary> {
        self.runs.get(*self.matching.get(self.selected)?)
    }

    /// Recomputes which runs match, keeping the selection in range.
    pub fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.matching = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| {
                needle.is_empty()
                    || run.prompt.to_lowercase().contains(&needle)
                    || run.run_id.to_lowercase().contains(&needle)
                    || outcome_word(run.outcome).contains(&needle)
                    // What this screen called a rejection until 1.1.0. Somebody
                    // who learned to type it should still find those runs.
                    || (needle == "refused" && run.outcome == Some(Outcome::Rejected))
            })
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.matching.len().saturating_sub(1));
        self.list_top = 0;
        self.forget_detail();
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.matching.is_empty() {
            return;
        }
        let last = self.matching.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.forget_detail();
    }

    /// Scrolls one pane, whichever it is, by a number of lines.
    ///
    /// The pane in front scrolls with the screen and any other keeps its own
    /// place, which is two fields rather than one — so a caller that has an
    /// index rather than "the one in front" goes through here.
    pub fn scroll_pane(&mut self, index: usize, delta: i16) {
        if index == self.at {
            self.scroll_by(delta);
            return;
        }
        let pane = &mut self.panes[index];
        pane.scroll = pane.scroll.saturating_add_signed(delta);
        if delta < 0 {
            pane.follow = false;
        }
    }

    pub fn scroll_by(&mut self, delta: i16) {
        self.scroll = self.scroll.saturating_add_signed(delta);
        // Scrolling up is someone reading something the tail is about to push
        // off the screen. Following again is `G`, and the end of the run.
        if delta < 0 {
            self.pane_mut().follow = false;
        }
    }

    /// Everything that describes one run, dropped when a different one is shown.
    pub fn forget_detail(&mut self) {
        self.record = None;
        self.events.clear();
        self.diff = None;
        self.scroll = 0;
    }

    /// What the browser is in the middle of, as the command list reads it.
    pub fn situation(&self) -> Situation {
        Situation {
            unconfigured: self.setup.is_some(),
            running: self.pane().running(),
            blocked: self.blocked.is_some(),
            panes: self.panes.len(),
            split: self.columns.len() > 1,
            selected: self.selection.is_some(),
            ready: self.thread().ready.is_some(),
        }
    }

    /// The commands the palette is currently offering.
    /// The models the picker shows for what has been typed into it.
    pub fn model_rows(&self) -> Vec<crate::models::Model> {
        let needle = self.query.trim().to_ascii_lowercase();
        self.models
            .iter()
            .filter(|row| {
                needle.is_empty()
                    || row.profile.to_ascii_lowercase().contains(&needle)
                    || row
                        .model
                        .as_deref()
                        .is_some_and(|m| m.to_ascii_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    }

    pub fn commands(&self) -> Vec<Command> {
        Command::matching(&self.query, self.situation())
    }

    pub fn picked(&self) -> Option<Command> {
        self.commands().get(self.pick).copied()
    }

    pub fn move_pick(&mut self, delta: isize) {
        let count = self.commands().len();
        if count == 0 {
            self.pick = 0;
            return;
        }
        self.pick = self.pick.saturating_add_signed(delta).min(count - 1);
    }

    /// Opens a dialog from a clean slate.
    pub fn open(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.query.clear();
        self.pick = 0;
        // Opened at the top. A reference that reopens halfway down, where it
        // was left last time, is one somebody has to scroll back up to read.
        self.keys_scroll = 0;
        self.leader = false;
    }

    pub fn close(&mut self) {
        self.dialog = None;
        self.query.clear();
        self.pick = 0;
    }

    /// Walks back and forward through what has been asked here.
    pub fn recall(&mut self, delta: isize) {
        let history = self.pane().thread.history.clone();
        if history.is_empty() {
            return;
        }
        let at = match (self.pane_mut().history_at, delta) {
            (None, d) if d < 0 => history.len() - 1,
            (None, _) => return,
            (Some(at), d) => match at.checked_add_signed(d) {
                Some(next) if next < history.len() => next,
                // Forward past the newest is back to what was being written.
                Some(_) => {
                    self.pane_mut().history_at = None;
                    self.pane_mut().replace(String::new());
                    return;
                }
                None => 0,
            },
        };
        self.pane_mut().history_at = Some(at);
        let said = history[at].clone();
        self.pane_mut().replace(said);
    }

    /// True while what is in the box is a command being picked rather than a
    /// task being written.
    ///
    /// One word behind a slash. A task with a space in it is a task, however
    /// it starts — the shape being recognised here is somebody reaching for a
    /// command, not somebody writing a sentence.
    pub fn slashing(&self) -> bool {
        self.focus == Focus::Prompt
            && self.dialog.is_none()
            && self.pane().prompt.starts_with('/')
            && !self.pane().prompt.contains(char::is_whitespace)
    }

    /// The commands the slash in the box is offering.
    pub fn slash_matches(&self) -> Vec<Command> {
        let word = self.pane().prompt.trim_start_matches('/').to_lowercase();
        let offered = Command::offered(self.situation());
        // A word that names a command exactly is that command, not the first
        // of everything it is a prefix of — and it is how `/help` and `/l`
        // reach anything at all, since neither is a slug.
        if let Some(exact) = Command::named(&word).filter(|c| offered.contains(c)) {
            return vec![exact];
        }
        offered
            .into_iter()
            .filter(|c| {
                word.is_empty()
                    || c.slug().starts_with(&word)
                    || c.name().to_lowercase().contains(&word)
            })
            .collect()
    }

    pub fn slash_picked(&self) -> Option<Command> {
        self.slash_matches().get(self.pick).copied()
    }

    /// The `@word` being written at the cursor, when there is one.
    pub fn mentioning(&self) -> Option<String> {
        if self.focus != Focus::Prompt || self.dialog.is_some() || self.slashing() {
            return None;
        }
        let pane = self.pane();
        mention::token(&pane.prompt, pane.at()).map(|(_, word)| word.to_string())
    }

    /// What the `@word` at the cursor could be completed to.
    pub fn mention_matches(&self) -> Vec<Candidate> {
        self.mentioning()
            .map(|word| mention::candidates(&word, &self.mentionable))
            .unwrap_or_default()
    }

    pub fn mention_picked(&self) -> Option<Candidate> {
        let matches = self.mention_matches();
        let at = self.pick.min(matches.len().saturating_sub(1));
        matches.get(at).cloned()
    }

    /// Reads what `@` can name, once for each repository a pane is in.
    pub fn load_mentionable(&mut self) {
        let repository = self.repository().map(|r| r.path.clone());
        if self.mentionable.loaded && self.mentionable.repository == repository {
            return;
        }
        let agents = self
            .workspace
            .profiles()
            .unwrap_or_default()
            .into_iter()
            .map(|profile| profile.id)
            .collect();
        let paths = repository
            .as_deref()
            .map(mention::paths)
            .unwrap_or_default();
        self.mentionable = Mentionable {
            loaded: true,
            repository,
            agents,
            paths,
        };
    }

    /// Replaces the `@word` at the cursor with the candidate picked for it.
    pub fn complete_mention(&mut self) -> bool {
        let Some(candidate) = self.mention_picked() else {
            return false;
        };
        let pane = self.pane_mut();
        let Some((prompt, cursor)) = mention::complete(&pane.prompt, pane.at(), &candidate) else {
            return false;
        };
        pane.cursor = (cursor < prompt.len()).then_some(cursor);
        pane.prompt = prompt;
        self.pick = 0;
        true
    }

    /// How many rows the input box wants, border included.
    fn prompt_height(&self) -> u16 {
        // Split rather than `lines`, which drops the empty row a trailing
        // newline starts — the row the cursor is on after alt-enter.
        let lines = self
            .pane()
            .prompt
            .split('\n')
            .count()
            .clamp(1, PROMPT_LINES);
        lines as u16 + 2
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    // At most a third of a short terminal: a box that grew to six lines on a
    // twelve-row screen would leave four rows for the work it is about.
    let box_height = app.prompt_height().min((screen.height / 3).max(3));
    // The bar costs a row and only appears once there is something to
    // navigate: one line of work needs no bar saying which one it is.
    let bar = u16::from(app.panes.len() > 1);
    // The footer earns its extra rows only where there are rows to spare, and
    // "to spare" was measured rather than guessed. A first draft started the
    // third row at twenty-six — a common default, and the size the flow tests
    // drive — and taking two rows there cost the setup screen the sentence
    // explaining its gate and cost a transcript two lines of what an agent had
    // just said. The thresholds below are where the tests put them, and are why
    // twenty-six now gets one row rather than three.
    //
    // Not during setup at all. That screen is a full page of explanation with
    // its own footer, and most commands are gated off until it has been taken,
    // so the rows would be spent listing what cannot be done.
    let footer = match screen.height {
        _ if app.setup.is_some() => 1,
        h if h >= 34 => 3,
        h if h >= 28 => 2,
        _ => 1,
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(bar),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(box_height),
            Constraint::Length(footer),
        ])
        .split(screen);

    frame.render_widget(breadcrumb(app, screen.width), theme::inset(rows[0]));
    if bar == 1 {
        frame.render_widget(
            Paragraph::new(pane_bar(app, rows[1].width.saturating_sub(theme::GUTTER))),
            theme::inset(rows[1]),
        );
    }
    frame.render_widget(
        Paragraph::new(theme::rule(rows[2].width.saturating_sub(theme::GUTTER))),
        theme::inset(rows[2]),
    );

    let content = theme::inset(rows[3]);
    // The agents take their width off the right of the work, and only where
    // the work keeps enough to be read. They never take the keys: nothing in
    // them answers a click or a key, so the box keeps the focus it had.
    let (content, side) = beside(app, content, screen.width);
    app.side_shown = side.is_some();
    // Measured off the area the work is drawn in, so the column count and
    // the columns cannot disagree about how wide it is. The thresholds are
    // terminal widths — the work plus the gutter `inset` took off its left —
    // which is what they were before the agents took any width.
    let work_width = content.width + theme::GUTTER;
    app.columns.clear();
    app.transcripts.clear();
    if app.setup.is_some() {
        render_setup(frame, app, content);
    } else {
        match app.screen {
            Screen::Work => match columns_for(app, work_width) {
                1 => render_work(frame, app, content),
                count => render_columns(frame, app, content, count),
            },
            Screen::Record => render_record(frame, app, content),
        }
    }
    if let Some(area) = side {
        render_roster(frame, app, area);
    }

    (app.prompt_text, app.prompt_scroll) = prompt_view(app, rows[4]);
    render_prompt(frame, app, rows[4]);
    if app.slashing() {
        render_slash(frame, app, rows[4]);
    } else if !app.mention_matches().is_empty() {
        render_mention(frame, app, rows[4]);
    }
    let width = rows[5].width.saturating_sub(theme::GUTTER);
    let mut lines = Vec::new();
    // The commands are the row added first, as they were before the facts
    // had one; the facts get a row of their own at three, and until then ride
    // on the bottom row after the suggestion, where the width allows.
    if footer >= 2 {
        lines.push(commands_row(app, width));
    }
    if footer >= 3 {
        lines.push(facts_row(app, width));
    }
    lines.push(status_bar(app, width, footer < 3));
    frame.render_widget(Paragraph::new(lines), theme::inset(rows[5]));

    match app.dialog {
        Some(Dialog::Keys) => render_keys(frame, app, screen),
        Some(Dialog::Commands) => render_palette(frame, app, screen),
        Some(Dialog::Runs) => render_runs(frame, app, screen),
        Some(Dialog::Agents) => render_agents(frame, app, screen),
        Some(Dialog::Leaving) => render_leaving(frame, app, screen),
        Some(Dialog::Fix) => render_fix(frame, app, screen),
        Some(Dialog::Settings) => render_settings(frame, app, screen),
        Some(Dialog::Repos) => render_repos(frame, app, screen),
        Some(Dialog::Prune) => render_prune(frame, app, screen),
        Some(Dialog::Models) => render_models(frame, app, screen),
        Some(Dialog::Fallback) => render_fallback(frame, app, screen),
        None => {}
    }

    // A frame this short has no room for chrome and content both. Saying so is
    // better than drawing a screen with nothing on it, which reads as a hang.
    if screen.height < CHROME {
        frame.render_widget(Clear, screen);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "terminal too short",
                theme::on(theme::WARN),
            ))),
            screen,
        );
    }
}

/// Where you are, said once, quietly, at the top.
fn breadcrumb(app: &App, width: u16) -> Paragraph<'static> {
    // Room is reserved for the branch only when there is one to say. A path
    // cut short to leave space for nothing is a path cut short for nothing.
    let room = if app.thread().continuing() {
        width.saturating_sub(38)
    } else {
        width.saturating_sub(2)
    };
    let mut spans = vec![Span::styled(
        truncate(&app.where_shown, room as usize),
        theme::muted(),
    )];
    // Which repository, because a workspace can hold several and a task goes
    // into exactly one of them.
    if let Some(repo) = &app.pane().repository {
        spans.push(Span::styled("  ", theme::muted()));
        spans.push(Span::styled(repo.name.clone(), theme::accent()));
    }
    // What the next run will stand on, where that is not simply `HEAD`. It
    // belongs beside the path because it is the same kind of fact: where you
    // are working.
    if app.thread().continuing() {
        spans.push(Span::styled("   on ", theme::muted()));
        spans.push(Span::styled(
            truncate(&app.thread().base_ref, 28),
            theme::accent(),
        ));
    }
    Paragraph::new(Line::from(spans))
}

/// The lines of work open here, and which one has the screen.
///
/// Numbered, because the number is what a keystroke takes, and marked where
/// something is running — a run in a pane nobody is looking at is still a run,
/// and a bar that did not say so would be a bar that hid it.
fn pane_bar(app: &App, width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, pane) in app.panes.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", theme::muted()));
        }
        let here = i == app.at;
        spans.push(Span::styled(
            format!("{} ", i + 1),
            if here {
                theme::accent()
            } else {
                theme::muted()
            },
        ));
        spans.push(Span::styled(
            truncate(&pane.title(), 18),
            if here {
                theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme::muted()
            },
        ));
        if pane.running() {
            spans.push(Span::styled(" \u{b7}", theme::on(theme::WARN)));
        }
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if used + 16 <= width as usize {
        spans.push(Span::styled(
            format!("    {} new", label(Chord::NewPane)),
            theme::muted(),
        ));
    }
    Line::from(spans)
}

/// A project path as a person would write it.
fn where_we_are(project: &Path) -> String {
    let full = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf())
        .display()
        .to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && full.starts_with(&home) => {
            format!("~{}", &full[home.len()..])
        }
        _ => full,
    }
}

/// The narrowest terminal that puts panes side by side. Two columns of eighty
/// are two transcripts somebody can read; below that, switching reads better.
const SPLIT_WIDTH: u16 = 160;
/// How much width each further column needs before it is added.
const COLUMN: u16 = 80;
/// The width every column is given before the weights share out the rest, so
/// a pane made as narrow as it goes is still a pane somebody can read.
const COLUMN_MIN: u16 = 48;
/// More than this and each is too narrow to be worth the room it takes.
const MAX_COLUMNS: usize = 3;
/// A rule down the middle of a cell either side of it.
const SEAM: u16 = 3;

/// How many panes the work screen shows at once on a terminal this wide.
/// The columns between the work and the agents beside it.
const SIDE_GAP: u16 = 2;

/// The work's area and, where there is room and they are wanted, the agents'.
///
/// Not while a directory is being set up — there are no profiles to list yet,
/// and that screen needs its width to explain itself — and not in a workspace
/// with no profiles at all, where the pane would be a heading over nothing.
fn beside(app: &App, content: Rect, screen_width: u16) -> (Rect, Option<Rect>) {
    let wanted = app.side && app.setup.is_none() && !app.profile_ids.is_empty();
    if !wanted || screen_width < super::roster::SHOWN_FROM {
        return (content, None);
    }
    let side_width = super::roster::WIDTH.min(content.width);
    let work = Rect {
        width: content.width.saturating_sub(side_width + SIDE_GAP),
        ..content
    };
    let side = Rect {
        x: work.x + work.width + SIDE_GAP,
        width: side_width,
        ..content
    };
    (work, Some(side))
}

/// Every profile in this workspace: can it run, what is it doing, and is it
/// the one a run would use next. A picture of `roster::rows` and nothing more;
/// the rows are worked out there.
fn render_roster(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = super::roster::rows(
        &app.profile_ids,
        &app.probes,
        &app.live,
        app.next_pair.as_ref(),
    );
    let room = area.width as usize;
    let mut lines = vec![
        Line::from(Span::styled("agents", theme::bold())),
        Line::from(Span::styled("\u{2500}".repeat(room), theme::muted())),
    ];
    for row in &rows {
        use super::roster::{Doing, Probe};
        let (word, colour) = match &row.probe {
            Probe::Asking => ("\u{2026}", theme::MUTED),
            Probe::Ready(_) => ("ready", theme::OK),
            Probe::Missing(_) => ("missing", theme::BAD),
        };
        let name_room = room.saturating_sub(word.chars().count() + 1);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<name_room$}", truncate(&row.id, name_room)),
                theme::text(),
            ),
            Span::raw(" "),
            Span::styled(word.to_string(), theme::on(colour)),
        ]));
        // What the probe said and, when nothing is going, that nothing is —
        // one row, because a profile per three rows does not fit the ten that
        // ship on a terminal of ordinary height.
        let note = match &row.probe {
            Probe::Ready(note) | Probe::Missing(note) => note.as_str(),
            Probe::Asking => "",
        };
        let second = match (note.is_empty(), row.doing.is_empty()) {
            (true, true) => "idle".to_string(),
            // Idle first: a long version string is what gets cut, not the
            // one word that says what the profile is doing.
            (false, true) => format!("idle \u{b7} {note}"),
            (_, false) => note.to_string(),
        };
        if !second.is_empty() {
            lines.push(dim(format!(
                "  {}",
                truncate(&second, room.saturating_sub(2))
            )));
        }
        let mut next = Vec::new();
        if row.writes_next {
            next.push("writes next");
        }
        if row.reviews_next {
            next.push("reviews next");
        }
        if !next.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {}", next.join(", ")),
                theme::accent(),
            )));
        }
        for doing in &row.doing {
            let (verb, run) = match doing {
                Doing::Writing(run) => ("writing", run),
                Doing::Reviewing(run) => ("reviewing", run),
            };
            let run_room = room.saturating_sub(verb.len() + 3);
            lines.push(Line::from(vec![
                Span::styled(format!("  {verb} "), theme::on(theme::WARN)),
                Span::styled(truncate(run, run_room), theme::text()),
            ]));
        }
    }
    // Cut from the bottom, and said. Not `theme::fit`, which keeps a dialog's
    // last line because that is its way out: here the last line is somebody's
    // status, and keeping it alone under the cut attaches it to nobody.
    let height = area.height as usize;
    if lines.len() > height && height > 1 {
        let hidden = lines.len() - (height - 1);
        lines.truncate(height - 1);
        lines.push(dim(format!("\u{2026} {hidden} more lines")));
    }
    // The tasks get what the profiles leave: a blank line, a heading and a
    // rule, then as much of the list as fits, headers first.
    app.task_headers.clear();
    const TASK_HEADING: usize = 3;
    let left = height.saturating_sub(lines.len());
    if !app.task_groups.is_empty() && left > TASK_HEADING {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("tasks", theme::bold())));
        lines.push(Line::from(Span::styled(
            "\u{2500}".repeat(room),
            theme::muted(),
        )));
        let task_lines =
            super::roster::task_lines(&app.task_groups, &app.tasks_closed, left - TASK_HEADING);
        for line in task_lines {
            if let super::roster::TaskLine::Header { repository, .. } = &line {
                let y = area.y + u16::try_from(lines.len()).unwrap_or(u16::MAX);
                app.task_headers.push((
                    Rect {
                        y,
                        height: 1,
                        ..area
                    },
                    repository.clone(),
                ));
            }
            lines.push(task_line(&line, room));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// One line of the task section.
///
/// A header reads `▾ name  ●1 ○2 ✓5`: going, waiting and done, each only where
/// there are some. A task reads its state's mark, the time part of its id and
/// as much of its prompt as fits. The first column is kept for a mark on a task
/// waiting on a person, which the decisions queue will set.
fn task_line(line: &super::roster::TaskLine, room: usize) -> Line<'static> {
    use super::roster::{TaskLine, TaskState};
    match line {
        TaskLine::Header {
            repository,
            waiting,
            going,
            done,
            open,
        } => {
            let counts: Vec<String> = [
                (*going, "\u{25cf}"),
                (*waiting, "\u{25cb}"),
                (*done, "\u{2713}"),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, mark)| format!("{mark}{n}"))
            .collect();
            let counts = counts.join(" ");
            let arrow = if *open { "\u{25be}" } else { "\u{25b8}" };
            let name_room = room.saturating_sub(counts.chars().count() + 3);
            Line::from(vec![
                Span::styled(format!("{arrow} "), theme::muted()),
                Span::styled(
                    format!("{:<name_room$}", truncate(repository, name_room)),
                    theme::bold(),
                ),
                Span::raw(" "),
                Span::styled(counts, theme::muted()),
            ])
        }
        TaskLine::Task(row) => {
            let (mark, colour) = match &row.state {
                TaskState::Going(_) => ("\u{25cf}", theme::WARN),
                TaskState::Waiting => ("\u{25cb}", theme::MUTED),
                TaskState::Done(Some(outcome)) if outcome == "approved" => ("\u{2713}", theme::OK),
                TaskState::Done(_) => ("\u{2717}", theme::BAD),
            };
            let person = if row.needs_person { "!" } else { " " };
            let id = super::roster::short_id(&row.id);
            let phase = match &row.state {
                TaskState::Going(Some(phase)) => format!("{} ", phase.title()),
                _ => String::new(),
            };
            let used = 2 + id.chars().count() + 1 + phase.chars().count();
            let prompt = row.prompt.lines().next().unwrap_or_default();
            Line::from(vec![
                Span::styled(person.to_string(), theme::on(theme::WARN)),
                Span::styled(format!("{mark} "), theme::on(colour)),
                Span::styled(format!("{id} "), theme::muted()),
                Span::styled(phase, theme::on(theme::WARN)),
                Span::styled(truncate(prompt, room.saturating_sub(used)), theme::text()),
            ])
        }
        TaskLine::More(n) => dim(format!("  \u{2026} {n} more")),
    }
}

fn columns_for(app: &App, width: u16) -> usize {
    if width < SPLIT_WIDTH {
        return 1;
    }
    app.panes
        .len()
        .min(MAX_COLUMNS)
        .min(usize::from(width / COLUMN))
        .max(1)
}

/// Shares `total` cells between columns by weight, after giving each column its
/// minimum. Whatever rounding leaves over goes to the last column.
fn column_widths(total: u16, weights: &[u16]) -> Vec<u16> {
    let count = u16::try_from(weights.len()).unwrap_or(u16::MAX).max(1);
    let floor = COLUMN_MIN.min(total / count);
    let spare = u32::from(total.saturating_sub(floor * count));
    let sum = weights.iter().map(|w| u32::from(*w)).sum::<u32>().max(1);
    let mut widths: Vec<u16> = weights
        .iter()
        .map(|w| floor + u16::try_from(spare * u32::from(*w) / sum).unwrap_or(0))
        .collect();
    let used: u16 = widths.iter().sum();
    if let Some(last) = widths.last_mut() {
        *last += total.saturating_sub(used);
    }
    widths
}

/// The panes side by side, each in its own column, with the pane in front
/// always among them.
fn render_columns(frame: &mut Frame, app: &mut App, area: Rect, count: usize) {
    if app.at < app.first {
        app.first = app.at;
    }
    if app.at >= app.first + count {
        app.first = app.at + 1 - count;
    }
    app.first = app.first.min(app.panes.len() - count);

    let shown: Vec<usize> = (app.first..app.first + count).collect();
    let weights: Vec<u16> = shown.iter().map(|i| app.panes[*i].weight).collect();
    let seams = SEAM * (count as u16 - 1);
    let widths = column_widths(area.width.saturating_sub(seams), &weights);

    let mut x = area.x;
    for (n, (index, width)) in shown.into_iter().zip(widths).enumerate() {
        if n > 0 {
            let rule: Vec<Line<'static>> = (0..area.height)
                .map(|_| Line::from(Span::styled(theme::CONTINUE, theme::muted())))
                .collect();
            frame.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: x + 1,
                    width: 1,
                    ..area
                },
            );
            x += SEAM;
        }
        let column = Rect { x, width, ..area };
        x += width;
        app.columns.push((index, column));
        render_column(frame, app, index, column);
    }
}

/// One pane's column: which pane it is, then its transcript.
fn render_column(frame: &mut Frame, app: &mut App, index: usize, area: Rect) {
    let front = index == app.at;
    let pane = &app.panes[index];
    let mut head = vec![
        Span::styled(
            format!("{} ", index + 1),
            if front {
                theme::accent()
            } else {
                theme::muted()
            },
        ),
        Span::styled(
            truncate(&pane.title(), area.width.saturating_sub(6) as usize),
            if front {
                theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme::muted()
            },
        ),
    ];
    if pane.running() {
        head.push(Span::styled(" \u{b7}", theme::on(theme::WARN)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(head)),
        Rect {
            height: area.height.min(1),
            ..area
        },
    );
    let body = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(1),
        ..area
    };
    render_transcript(frame, app, index, body);
}

/// The thread: everything asked here, and what came of it.
fn render_work(frame: &mut Frame, app: &mut App, area: Rect) {
    render_transcript(frame, app, app.at, area);
}

/// One pane's thread. The pane in front scrolls with the screen and sets the
/// page the page keys move by; any other keeps the place it was left at.
fn render_transcript(frame: &mut Frame, app: &mut App, index: usize, area: Rect) {
    let front = index == app.at;
    if front {
        app.page = area.height.saturating_sub(1).max(1);
    }
    if app.panes[index].thread.is_empty() {
        let lines = if front {
            opening(app, area.width, area.height)
        } else {
            vec![dim("Nothing asked in this pane yet.".to_string())]
        };
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let lines = thread_lines(&app.panes[index].thread, area.width, app.tick);
    let overflow = lines.len().saturating_sub(area.height as usize) as u16;
    let pane = &app.panes[index];
    let scroll = match (pane.follow, front) {
        (true, _) => overflow,
        (false, true) => app.scroll,
        (false, false) => pane.scroll,
    }
    .min(overflow);
    if front {
        app.scroll = scroll;
    } else {
        app.panes[index].scroll = scroll;
    }
    // Where this transcript was drawn and how far down it was, so a drag can
    // be read back into the lines it was drawn from. A screen row on its own
    // says nothing: the same row is a different line after a scroll.
    app.transcripts.push((index, area, scroll));
    let lines = match &app.selection {
        Some(selection) if selection.pane == index => super::select::highlight(lines, selection),
        _ => lines,
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

/// The transcript of one pane, as the last draw rendered it.
///
/// Rendered again rather than kept from the draw: a `Vec<Line>` held across
/// frames is a second copy of the screen that can disagree with the one on it,
/// and this is asked for once, when somebody copies.
pub fn transcript_of(app: &App, index: usize) -> Vec<Line<'static>> {
    let width = app
        .transcripts
        .iter()
        .find(|(at, _, _)| *at == index)
        .map(|(_, area, _)| area.width)
        .unwrap_or(80);
    thread_lines(&app.panes[index].thread, width, app.tick)
}

/// What is on screen before anything has been asked in this session.
///
/// "Nothing has been asked here yet" is true of the thread and false of the
/// directory, and in a repository with fifty runs behind it that reads as a
/// browser that has lost them. What was asked here last is the context
/// somebody opening this wants, so it is the first thing they get.
fn opening(app: &App, width: u16, height: u16) -> Vec<Line<'static>> {
    /// Enough to recognise where the work got to, and not a second list.
    const RECENT: usize = 5;
    /// What the summary costs besides its rows: a heading, three blanks, the
    /// count and the rule under it.
    const AROUND: usize = 6;

    // The guidance is what somebody opening this for the first time needs, and
    // the summary is context. On a screen too short for both, the context goes
    // — a run they cannot see is a run `l` still lists.
    let guidance = if app.blocked.is_some() { 8 } else { 4 };
    let room = (height as usize).saturating_sub(guidance + AROUND);
    let recent = room.min(RECENT);

    let mut lines = Vec::new();
    if !app.runs.is_empty() && recent > 0 {
        lines.push(Line::from(Span::styled("Last asked here", theme::bold())));
        lines.push(Line::from(""));
        for run in app.runs.iter().take(recent) {
            let (mark, colour) = marker(run.outcome);
            lines.push(Line::from(vec![
                Span::styled(format!(" {mark}  "), theme::on(colour)),
                Span::styled(format!("{:<13}", when(&run.run_id)), theme::muted()),
                Span::styled(
                    format!(
                        "{:<width$}",
                        truncate(&described(&run.prompt), width.saturating_sub(34) as usize),
                        width = width.saturating_sub(34) as usize
                    ),
                    theme::text(),
                ),
                Span::styled(
                    format!("  {}", outcome_word(run.outcome)),
                    theme::on(colour),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(counted(app), theme::muted()),
            Span::styled("  \u{b7}  ", theme::muted()),
            Span::styled("l", theme::accent()),
            Span::styled(" looks any of them up", theme::muted()),
        ]));
        lines.push(Line::from(""));
        lines.push(theme::rule(width));
        lines.push(Line::from(""));
    }

    // What is in the way, where there is something. It was said on the setup
    // screen and that screen is gone the moment setting up is done — leaving a
    // workspace that is configured, has nowhere to run anything, and says so
    // only if you press enter and find out.
    if let Some(blocked) = &app.blocked {
        for part in wrap(blocked, width as usize) {
            lines.push(Line::from(Span::styled(part, theme::on(theme::WARN))));
        }
        lines.push(Line::from(vec![
            Span::styled("x", theme::accent()),
            Span::styled(" walks through it.", theme::muted()),
        ]));
        lines.push(Line::from(""));
    }

    // What can be done here, rather than how the machinery works. A first
    // screen that opens with "isolated, gated and reviewed" describes a
    // pipeline to somebody who has not yet asked it anything, and a thread
    // opens in ask, where none of those three happens.
    lines.push(Line::from(vec![
        Span::styled("Ask anything about this repository. ", theme::muted()),
        Span::styled("enter", theme::accent()),
        Span::styled(" answers; nothing is changed.", theme::muted()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("e", theme::accent()),
        Span::styled(
            " runs what you asked: gated, reviewed by a different agent.",
            theme::muted(),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("shift-tab", theme::accent()),
        Span::styled(" switches what enter does.", theme::muted()),
    ]));
    // A chain says where the next one will start, which is the other half of
    // "where did I get to".
    if app.thread().continuing() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("The next one starts from ", theme::muted()),
            Span::styled(app.thread().base_ref.clone(), theme::accent()),
            Span::styled(".", theme::muted()),
        ]));
    }
    lines
}

fn thread_lines(thread: &Thread, width: u16, tick: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for turn in &thread.turns {
        lines.extend(asked(&turn.prompt, width));
        lines.extend(steps_lines(&turn.steps, None, width, tick));
        lines.push(ended_rule(turn, width));
        lines.push(Line::from(""));
    }
    if let Some(session) = &thread.live {
        lines.extend(asked(&session.prompt, width));
        lines.extend(steps_lines(&session.steps, session.phase, width, tick));
        if session.stopping {
            lines.push(dim(
                "stopping \u{2014} the agent is being asked to stop".into()
            ));
        }
    }
    lines
}

/// What was asked, marked as yours rather than as anything an agent said.
fn asked(prompt: &str, width: u16) -> Vec<Line<'static>> {
    wrap(prompt, width.saturating_sub(2) as usize)
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            Line::from(vec![
                Span::styled(
                    if i == 0 { theme::CURSOR } else { " " },
                    theme::on(theme::ACCENT),
                ),
                Span::raw(" "),
                Span::styled(part, theme::bold()),
            ])
        })
        .collect()
}

/// The rule that closes a turn, carrying how it ended.
fn ended_rule(turn: &Turn, width: u16) -> Line<'static> {
    match (&turn.finished, &turn.failed) {
        (Some(finished), _) => {
            let (_, colour) = marker(finished.outcome);
            theme::labelled_rule(width, &finished.summary, colour)
        }
        // Not a verdict. The run did not get far enough to have one, and
        // saying "refused" here would put a judgement nobody made on record.
        (None, Some(why)) => {
            theme::labelled_rule(width, &format!("did not finish \u{2014} {why}"), theme::BAD)
        }
        (None, None) => theme::rule(width),
    }
}

fn steps_lines(
    steps: &[Step],
    running: Option<Phase>,
    width: u16,
    tick: u64,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut phase = Phase::Isolating;
    for step in steps {
        match step {
            Step::Entered(entered) => {
                phase = *entered;
                lines.push(phase_line(*entered, running, tick));
            }
            Step::Said { event, .. } => {
                let (kind, text, colour) = describe_event(event);
                let gutter = Span::styled(
                    format!("{} ", theme::CONTINUE),
                    theme::on(phase_colour(phase)),
                );
                lines.extend(event_rows(
                    vec![
                        gutter.clone(),
                        Span::styled(format!("{kind} "), theme::muted()),
                    ],
                    vec![gutter, Span::raw(" ".repeat(kind.len() + 1))],
                    &text,
                    theme::on(colour),
                    width,
                ));
            }
            Step::Checked(record) => lines.extend(checked_lines(record)),
        }
    }
    lines
}

/// A phase heading, in the colour of whoever is doing it.
fn phase_line(phase: Phase, running: Option<Phase>, tick: u64) -> Line<'static> {
    let mut spans = vec![Span::styled(
        phase.title(),
        theme::on(phase_colour(phase)).add_modifier(Modifier::BOLD),
    )];
    if running == Some(phase) {
        let frame = theme::SPINNER[tick as usize % theme::SPINNER.len()];
        spans.push(Span::styled(format!("  {frame}"), theme::accent()));
    }
    Line::from(spans)
}

/// Who a phase belongs to, as a colour.
///
/// The two the runtime does itself are chrome; the three that cost something —
/// an agent writing, the project's own checks, a second agent reading — each
/// get their own, so a transcript can be scanned for one of them.
fn phase_colour(phase: Phase) -> Color {
    match phase {
        Phase::Isolating | Phase::Preparing => theme::MUTED,
        Phase::Authoring => theme::ACCENT,
        Phase::Gating => theme::WARN,
        Phase::Reviewing => theme::REVIEW,
    }
}

/// One finished check, under the rail, with its output if it failed.
fn checked_lines(record: &CheckRecord) -> Vec<Line<'static>> {
    let (mark, word, colour) = check_mark(record);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", theme::CONTINUE), theme::on(theme::WARN)),
        Span::styled(format!("{mark} {word}"), theme::on(colour)),
        Span::styled(
            format!("  {:<10} {}ms", record.name, record.duration_ms),
            theme::text(),
        ),
    ])];
    if !record.passed() {
        for line in tail(&record.stderr, 12)
            .into_iter()
            .chain(tail(&record.stdout, 12))
        {
            lines.push(dim(format!("{}      {line}", theme::CONTINUE)));
        }
    }
    lines
}

/// How a check is said, in the one place that decides it.
fn check_mark(record: &CheckRecord) -> (&'static str, &'static str, Color) {
    if record.passed() {
        (theme::PASSED, "pass", theme::OK)
    } else {
        (theme::FAILED, "FAIL", theme::BAD)
    }
}

/// What an event is, said the same way wherever it is shown.
fn describe_event(event: &Event) -> (&'static str, String, Color) {
    match event {
        // As the vendor wrote it. Flattening newlines here turned a list, a
        // stack trace or a block of code into one run-on line, and then the
        // pane cut that line at its width — so what an agent said was never on
        // the screen whole. `event_rows` folds it to the width instead.
        Event::Message { text, .. } => ("said", text.clone(), theme::TEXT),
        Event::ToolUse { name, .. } => ("tool", name.clone(), theme::ACCENT),
        Event::Error { message, .. } => ("err ", message.clone(), theme::BAD),
        Event::Finished {
            exit_code,
            files_touched,
        } => (
            "done",
            format!(
                "exit {}, {} file(s)",
                exit_code.map(|c| c.to_string()).unwrap_or("?".into()),
                files_touched.len()
            ),
            theme::MUTED,
        ),
    }
}

/// One run out of the history, read back.
fn render_record(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(run) = app.current().cloned() else {
        frame.render_widget(
            Paragraph::new(dim("No run selected. `l` lists them.".to_string())),
            area,
        );
        return;
    };
    app.page = area.height.saturating_sub(1).max(1);

    let mut lines = summary_lines(app, &run, area.width as usize);
    lines.push(tabs(app));
    lines.push(theme::rule(area.width));
    match app.detail {
        Detail::Checks => lines.extend(check_lines(app, &run)),
        Detail::Events => lines.extend(event_lines(app, area.width)),
        Detail::Diff => lines.extend(diff_lines(app)),
    }

    let overflow = lines.len().saturating_sub(area.height as usize);
    app.scroll = app.scroll.min(overflow as u16);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
}

/// The three panes, named, with the one you are in marked.
fn tabs(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, pane) in Detail::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", theme::muted()));
        }
        spans.push(Span::styled(
            pane.title(),
            if *pane == app.detail {
                theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme::muted()
            },
        ));
    }
    Line::from(spans)
}

/// Where the task's text sits inside the box, and how far down the box is
/// scrolled to keep the cursor's row in it.
///
/// One function, because the box is drawn from it and a click is read against
/// it, and two sums of the same margins are how a click lands a cell away.
fn prompt_view(app: &App, area: Rect) -> (Rect, u16) {
    // The gutter either side, then the border, then a cell of padding.
    let text = Rect {
        x: area.x.saturating_add(theme::GUTTER + 2),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(theme::GUTTER * 2 + 4),
        height: area.height.saturating_sub(2),
    };
    let (row, _) = app.pane().cursor_row();
    let scroll = row.saturating_sub(usize::from(text.height.max(1)) - 1);
    (text, u16::try_from(scroll).unwrap_or(u16::MAX))
}

/// The line that says where typing goes.
fn render_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let inner = Rect {
        x: area.x.saturating_add(theme::GUTTER),
        width: area.width.saturating_sub(theme::GUTTER * 2),
        ..area
    };
    let (_, scroll) = prompt_view(app, area);
    let writing = app.focus == Focus::Prompt && app.dialog.is_none();
    let block = theme::panel(writing).padding(Padding::horizontal(1));

    let text: Vec<Line<'static>> = if !writing && app.pane().prompt.is_empty() {
        vec![Line::from(Span::styled(
            format!(
                "n  write a task     l  runs     {}  commands     ?  keys",
                label(Chord::Commands)
            ),
            theme::muted(),
        ))]
    } else {
        let pane = app.pane();
        let (cursor_row, offset) = pane.cursor_row();
        pane.prompt
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                if !writing || i != cursor_row {
                    return Line::from(Span::styled(line.to_string(), theme::text()));
                }
                // At the end of a row the cursor is the bar after it. Inside
                // one it is the character it would type before, reversed: a bar
                // there would push the rest of the row a cell to the right of
                // where a click on it would find it.
                let (before, after) = line.split_at(offset);
                let mut spans = vec![Span::styled(before.to_string(), theme::text())];
                match after.chars().next() {
                    None => spans.push(Span::styled(theme::CURSOR, theme::accent())),
                    Some(c) => {
                        let (under, rest) = after.split_at(c.len_utf8());
                        spans.push(Span::styled(
                            under.to_string(),
                            theme::text().add_modifier(Modifier::REVERSED),
                        ));
                        spans.push(Span::styled(rest.to_string(), theme::text()));
                    }
                }
                if pane.prompt.is_empty() {
                    // What enter will do here, which is not the same sentence
                    // in every mode. "enter runs it" under a box that answers
                    // questions is the wrong promise, and the modes that do
                    // run are the ones worth saying so about.
                    let hint = if pane.thread.pending_plan.is_some() {
                        "  enter runs the plan above \u{b7} or write another task"
                    } else if pane.thread.ready.is_some() {
                        "  ask again \u{b7} or e to have it done"
                    } else if pane.thread.mode.consults() {
                        "  ask anything about this repository \u{b7} nothing is changed"
                    } else {
                        "  say what the agent should do \u{b7} enter runs it"
                    };
                    spans.push(Span::styled(hint, theme::muted()));
                }
                Line::from(spans)
            })
            .collect()
    };

    let mark = if writing {
        Span::styled("\u{203a}", theme::accent().add_modifier(Modifier::BOLD))
    } else {
        Span::styled("\u{203a}", theme::muted())
    };
    // What enter will do, on the box enter is pressed in. Muted for the mode
    // that runs, marked for the three that do something else.
    let mode = app.thread().mode;
    let named = Span::styled(
        format!("{} ", mode.word()),
        if mode == Mode::Auto {
            theme::muted()
        } else {
            theme::accent()
        },
    );
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll, 0))
            .block(block.title(Line::from(vec![
                Span::raw(" "),
                mark,
                Span::raw(" "),
                named,
            ]))),
        inner,
    );
}

/// The commands a slash in the box is offering, at the box rather than in the
/// middle of the screen.
///
/// Anchored where the typing is, because that is where the eye already is. The
/// palette in the middle of the screen is for going and finding a command; this
/// is for the one you were halfway through naming.
fn render_slash(frame: &mut Frame, app: &App, box_area: Rect) {
    const SHOWN: usize = 6;
    let matches = app.slash_matches();
    let rows = matches.len().clamp(1, SHOWN);
    let height = rows as u16 + 2;
    let width = box_area.width.saturating_sub(theme::GUTTER * 2);
    if box_area.y < height {
        return;
    }
    let area = Rect {
        x: box_area.x + theme::GUTTER,
        y: box_area.y - height,
        width,
        height,
    };

    let mut lines = Vec::new();
    if matches.is_empty() {
        lines.push(dim("no command by that name".to_string()));
    }
    let first = app.pick.saturating_sub(SHOWN - 1);
    for (i, command) in matches.iter().enumerate().skip(first).take(SHOWN) {
        let here = i == app.pick;
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(
                format!(" /{:<12}", command.slug()),
                if here {
                    theme::accent().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
            Span::styled(command.about().to_string(), theme::muted()),
        ]));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(1))),
        area,
    );
}

/// What an `@` in the box could name, at the box, the way a slash offers the
/// commands.
fn render_mention(frame: &mut Frame, app: &App, box_area: Rect) {
    const SHOWN: usize = 6;
    let matches = app.mention_matches();
    let rows = matches.len().clamp(1, SHOWN);
    let height = rows as u16 + 2;
    let width = box_area.width.saturating_sub(theme::GUTTER * 2);
    if box_area.y < height {
        return;
    }
    let area = Rect {
        x: box_area.x + theme::GUTTER,
        y: box_area.y - height,
        width,
        height,
    };
    // The kind is the last word on the row, and the path has what is left.
    let room = usize::from(width).saturating_sub(18);
    let pick = app.pick.min(matches.len().saturating_sub(1));
    let first = pick.saturating_sub(SHOWN - 1);
    let lines: Vec<Line<'static>> = matches
        .iter()
        .enumerate()
        .skip(first)
        .take(SHOWN)
        .map(|(i, candidate)| {
            let here = i == pick;
            Line::from(vec![
                Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
                Span::styled(
                    format!(" @{:<room$} ", truncate(&candidate.text, room)),
                    if here {
                        theme::accent().add_modifier(Modifier::BOLD)
                    } else {
                        theme::text()
                    },
                ),
                Span::styled(candidate.kind.word(), theme::muted()),
            ])
        })
        .collect();

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(1))),
        area,
    );
}

/// The repositories this workspace works on.
fn render_repos(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 78u16.min(screen.width);
    let repositories = app.workspace.repositories();
    let mut lines = vec![
        Line::from(Span::styled("Repositories", theme::bold())),
        theme::rule(width.saturating_sub(6)),
    ];
    if repositories.is_empty() {
        // The directory this workspace actually uses, which is `repositories/`
        // only until somebody says otherwise.
        let dir = app.workspace.repositories_dir();
        let shown = dir
            .strip_prefix(&app.workspace.root)
            .map(|inside| format!("{}/", inside.display()))
            .unwrap_or_else(|_| dir.display().to_string());
        lines.push(dim(format!("Nothing has been cloned into {shown} yet.")));
    }
    // Naming a new one. Cloning needs a URL only the operator knows; starting
    // one needs a name, which is a thing this screen can take.
    if let Some(name) = &app.editing {
        lines.push(Line::from(vec![
            Span::styled(" \u{203a} ", theme::accent()),
            Span::styled(name.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
            Span::styled("     a name, and enter starts it", theme::muted()),
        ]));
    }
    for (i, repo) in repositories.iter().enumerate() {
        let here = i == app.pick;
        let working = app.repository().is_some_and(|r| r.name == repo.name);
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(
                format!(" {:<24}", repo.name),
                if here {
                    theme::text().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
            Span::styled(if working { "working here" } else { "" }, theme::accent()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(match app.editing {
        Some(_) => dim("enter starts it \u{b7} esc puts it back".to_string()),
        None => Line::from(vec![
            Span::styled("enter", theme::accent()),
            Span::styled("  work here      ", theme::muted()),
            Span::styled("n", theme::accent()),
            Span::styled("  start one here      ", theme::muted()),
            Span::styled("esc", theme::accent()),
            Span::styled("  close", theme::muted()),
        ]),
    });

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// What this thread and this project are set to.
///
/// Two halves, and the difference between them matters. The top is this
/// thread's, changeable here and gone when the browser closes. The bottom is
/// the project's, read from `ostraka.toml` and shown rather than edited: a
/// gate is a thing a repository agrees on, and a screen that quietly rewrote
/// it would be a screen that changed what everybody else's runs are judged by.
fn render_settings(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 84u16.min(screen.width);
    let rows = settings_rows(app);
    let mut lines = vec![
        Line::from(Span::styled("This thread", theme::bold())),
        theme::rule(width.saturating_sub(6)),
    ];
    for (i, (name, value, editable)) in rows.iter().enumerate() {
        let here = i == app.pick;
        let shown = match (&app.editing, here) {
            (Some(buffer), true) => Line::from(vec![
                Span::styled(buffer.clone(), theme::text()),
                Span::styled(theme::CURSOR, theme::accent()),
            ]),
            _ => Line::from(Span::styled(
                value.clone(),
                if *editable {
                    theme::text()
                } else {
                    theme::muted()
                },
            )),
        };
        let mut spans = vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(format!(" {name:<12}"), theme::muted()),
        ];
        spans.extend(shown.spans);
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("This project", theme::bold())));
    lines.push(theme::rule(width.saturating_sub(6)));
    if app.project_facts.is_empty() {
        lines.push(dim("no ostraka.toml here yet".to_string()));
    }
    for (name, value) in &app.project_facts {
        lines.push(Line::from(vec![
            Span::styled(format!("  {name:<12}"), theme::muted()),
            Span::styled(
                truncate(value, width.saturating_sub(22) as usize),
                theme::text(),
            ),
        ]));
    }
    lines.push(dim(
        "  set in ostraka.toml; a gate is what the repository agrees on".to_string(),
    ));

    lines.push(Line::from(""));
    lines.push(match app.editing {
        Some(_) => dim("enter keeps it \u{b7} esc puts it back".to_string()),
        None => Line::from(vec![
            Span::styled("enter", theme::accent()),
            Span::styled("  change      ", theme::muted()),
            Span::styled("a", theme::accent()),
            Span::styled("  who writes and reviews      ", theme::muted()),
            Span::styled("esc", theme::accent()),
            Span::styled("  close", theme::muted()),
        ]),
    });

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// The thread's own settings: name, what it is, and whether it can be typed at.
pub fn settings_rows(app: &App) -> Vec<(&'static str, String, bool)> {
    let chosen = |id: &Option<String>| match id {
        Some(id) => id.clone(),
        None => "automatic".to_string(),
    };
    vec![
        (
            "repository",
            app.repository()
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "\u{2014}".into()),
            false,
        ),
        ("author", app.thread().author.clone(), true),
        ("reviewer", app.thread().reviewer.clone(), true),
        (
            "model",
            app.thread()
                .model
                .clone()
                .unwrap_or_else(|| "\u{2014}".into()),
            true,
        ),
        ("writes", chosen(&app.thread().adapter), false),
        ("reviews", chosen(&app.thread().review_adapter), false),
        ("starts from", app.thread().base_ref.clone(), false),
    ]
}

/// The commands, offered rather than remembered.
///
/// The same list the leader shows, with the chord it needs written in front of
/// it. Bare letters here would be the defect this browser has already fixed
/// once: on the work screen the box has the keys, so `n` types an `n`, and a
/// row of single letters is an invitation to find that out.
///
/// Truncated rather than wrapped. A row that grew would take a row from the
/// work, and the commands are ordered by what somebody reaches for first, so
/// what falls off the end is what falls off the end.
fn commands_row(app: &App, width: u16) -> Line<'static> {
    let mut spans = vec![
        Span::styled(label(Chord::Leader), theme::on(theme::WARN)),
        Span::styled("  ", theme::muted()),
    ];
    let mut used = label(Chord::Leader).chars().count() + 2;
    for (i, command) in Command::offered(app.situation()).iter().enumerate() {
        let text = format!("{} {}", command.leader(), command.name());
        let sep = usize::from(i > 0) * 3;
        if used + sep + text.chars().count() > width as usize {
            break;
        }
        if i > 0 {
            spans.push(Span::styled(" \u{b7} ", theme::muted()));
        }
        spans.push(Span::styled(
            command.leader().to_string(),
            theme::on(theme::ACCENT),
        ));
        spans.push(Span::styled(format!(" {}", command.name()), theme::muted()));
        used += sep + text.chars().count();
    }
    Line::from(spans)
}

/// One piece of a footer row: its spans, and how wide they draw.
struct Segment(Vec<Span<'static>>);

impl Segment {
    fn width(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        self.0.iter().map(|s| s.content.width()).sum()
    }
}

/// The gap between segments of a row.
const SEGMENT_GAP: &str = "  \u{b7}  ";

/// Segments in priority order, as many as fit, and never wrapped.
///
/// Which ones fit is `footer::fit`'s to decide; this only draws its answer. A
/// cut first segment keeps its first style and loses its end.
fn row_of(segments: Vec<Segment>, width: usize) -> (Vec<Span<'static>>, usize) {
    use unicode_width::UnicodeWidthStr;
    let widths: Vec<usize> = segments.iter().map(Segment::width).collect();
    let gap = SEGMENT_GAP.chars().count();
    let (kept, cut) = super::footer::fit(&widths, width, gap);
    let mut spans = Vec::new();
    let mut used = 0;
    for (i, segment) in segments.into_iter().take(kept).enumerate() {
        if i > 0 {
            spans.push(Span::styled(SEGMENT_GAP, theme::muted()));
            used += gap;
        }
        match cut {
            Some(room) if i == 0 => {
                let style = segment.0.first().map(|s| s.style).unwrap_or_default();
                let text: String = segment.0.iter().map(|s| s.content.as_ref()).collect();
                let text = truncate(&text, room);
                used += text.width();
                spans.push(Span::styled(text, style));
            }
            _ => {
                used += segment.width();
                spans.extend(segment.0);
            }
        }
    }
    (spans, used)
}

/// What the footer is drawn from, gathered from what the browser holds.
fn footer_facts(app: &App) -> super::footer::Facts {
    let listed: Vec<&RunSummary> = app
        .matching
        .iter()
        .filter_map(|i| app.runs.get(*i))
        .collect();
    super::footer::facts(
        &listed,
        app.task_groups.iter().map(|g| g.waiting).sum(),
        app.live.len(),
        &app.unpromoted,
        app.last_record.as_ref(),
    )
}

/// How the listed runs ended, what is waiting and going, and the last run.
fn facts_segments(app: &App) -> Vec<Segment> {
    let facts = footer_facts(app);
    let mut segments = Vec::new();
    let counts = super::footer::counts(&facts);
    if !counts.is_empty() {
        segments.push(Segment(vec![Span::styled(
            counts.join(", "),
            theme::text(),
        )]));
    }
    if let Some(last) = &facts.last {
        segments.push(Segment(vec![Span::styled(last.phrase(), theme::muted())]));
    }
    segments
}

/// The facts row: counts, the last run, and — only while the agents are not
/// beside the work, since they mark it there — which profiles write and
/// review next.
fn facts_row(app: &App, width: u16) -> Line<'static> {
    let mut segments = facts_segments(app);
    if !app.side_shown {
        segments.push(routing_segment(app));
    }
    Line::from(row_of(segments, width as usize).0)
}

/// What the next run will be made with: which profile writes and which
/// reviews. The path, the repository and the branch are on the top row.
fn routing_segment(app: &App) -> Segment {
    let chosen = |id: &Option<String>| match id {
        Some(id) => id.clone(),
        None => "automatic".to_string(),
    };
    Segment(vec![
        Span::styled("writes ", theme::muted()),
        Span::styled(chosen(&app.thread().adapter), theme::text()),
        Span::styled("  reviews ", theme::muted()),
        Span::styled(chosen(&app.thread().review_adapter), theme::text()),
    ])
}

/// The one next action worth naming, with the keys that take it.
fn suggestion_segment(app: &App) -> Option<Segment> {
    use super::footer::Suggestion;
    let selected_unpromoted = app
        .current()
        .filter(|run| app.unpromoted.contains(&run.run_id))
        .map(|run| run.run_id.as_str());
    let state = super::footer::Situation {
        facts: footer_facts(app),
        blocked: app.blocked.is_some(),
        selected_unpromoted,
        mode: app.thread().mode,
    };
    let keyed = |command: Command, said: String| {
        Segment(vec![
            Span::styled("next  ", theme::muted()),
            Span::styled(
                format!("{} {}", label(Chord::Leader), command.leader()),
                theme::on(theme::ACCENT),
            ),
            Span::styled(format!(" {said}"), theme::text()),
        ])
    };
    let runs = |n: usize| format!("{n} approved run{}", plural(n));
    Some(match super::footer::suggest(&state)? {
        Suggestion::Fix => keyed(Command::Fix, "fix what is in the way".into()),
        Suggestion::Promote(_) => keyed(Command::Promote, "promote the selected run".into()),
        Suggestion::OpenRuns(n) => keyed(Command::Runs, format!("{} to promote", runs(n))),
        Suggestion::Loop => keyed(
            Command::Mode,
            "loop mode, which retries with what the check said".into(),
        ),
        Suggestion::Drain(n) => Segment(vec![
            Span::styled("next  ", theme::muted()),
            Span::styled("ostraka drain", theme::on(theme::ACCENT)),
            Span::styled(
                format!(" takes the {n} waiting task{}", plural(n)),
                theme::text(),
            ),
        ]),
    })
}

/// The bottom line: what is happening, what to do next, and what it cost.
///
/// In priority order, dropped from the end: a status message or the running
/// state, the suggestion, then — where the facts have no row of their own —
/// the counts and the last run, and last the tokens, drawn at the right edge.
fn status_bar(app: &App, width: u16, only_row: bool) -> Line<'static> {
    use unicode_width::UnicodeWidthStr;
    // A status message first, even over the leader and setup lines: it is the
    // answer to what somebody just did, and the setup screen is where most
    // keys answer with a reason rather than an action.
    if let Some(status) = &app.status {
        let (spans, _) = row_of(
            vec![Segment(vec![Span::styled(
                status.clone(),
                theme::on(theme::WARN),
            )])],
            width as usize,
        );
        return Line::from(spans);
    }
    if app.leader {
        let mut spans = vec![Span::styled(
            format!("{}  ", label(Chord::Leader)),
            theme::on(theme::WARN),
        )];
        for (i, command) in Command::offered(app.situation()).iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" \u{b7} ", theme::muted()));
            }
            spans.push(Span::styled(
                command.leader().to_string(),
                theme::on(theme::WARN),
            ));
            spans.push(Span::styled(format!(" {}", command.name()), theme::muted()));
        }
        return Line::from(spans);
    }
    if app.setup.is_some() {
        return Line::from(Span::styled(
            "i set this directory up \u{b7} q quit",
            theme::muted(),
        ));
    }

    // Running first. Idle, the name and version go last among the segments:
    // the brief ranks the suggestion and the counts above them, and they are
    // the one thing on the row that never changes.
    let running = app
        .thread()
        .live
        .as_ref()
        .filter(|s| s.live())
        .map(|session| {
            Segment(vec![
                Span::styled(
                    if session.stopping {
                        "stopping"
                    } else {
                        "running"
                    },
                    theme::on(theme::WARN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  {}  \u{b7}  {}s",
                        session.phase.map(Phase::title).unwrap_or("\u{2026}"),
                        session.elapsed_secs
                    ),
                    theme::muted(),
                ),
            ])
        });
    let idle = running.is_none();
    let mut segments: Vec<Segment> = running.into_iter().collect();
    segments.extend(suggestion_segment(app));
    if only_row {
        segments.extend(facts_segments(app));
    }
    if idle {
        segments.push(Segment(vec![
            Span::styled("ostraka", theme::bold()),
            Span::styled(format!(" {}", env!("CARGO_PKG_VERSION")), theme::muted()),
        ]));
    }
    let width = width as usize;
    let (mut spans, used) = row_of(segments, width);

    // The tokens last, at the right edge, and only whole.
    let right = tokens(app);
    let right_width: usize = right.iter().map(|s| s.content.width()).sum();
    if used + right_width + 2 <= width {
        spans.push(Span::raw(" ".repeat(width - used - right_width)));
        spans.extend(right);
    }
    Line::from(spans)
}

fn counted(app: &App) -> String {
    let shown = app.matching.len();
    let total = app.runs.len();
    if shown == total {
        format!("{total} run{}", plural(total))
    } else {
        format!("{shown} of {total} runs")
    }
}

/// What each backend has cost, across the runs currently listed.
///
/// Every figure is a vendor's own accounting. A backend that reports nothing
/// does not appear here at all rather than appearing as a zero — "does not say"
/// and "spent nothing" are different claims, and only one of them is true.
fn tokens(app: &App) -> Vec<Span<'static>> {
    let listed: Vec<RunSummary> = app
        .matching
        .iter()
        .filter_map(|i| app.runs.get(*i))
        .cloned()
        .collect();
    let backends = index::by_backend(&listed);

    if backends.is_empty() {
        return vec![Span::styled(
            "tokens  no backend on these runs reported what it spent",
            theme::muted(),
        )];
    }

    let mut spans = vec![Span::styled("tokens  ", theme::muted())];
    for (i, backend) in backends.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" \u{b7} ", theme::muted()));
        }
        spans.push(Span::styled(backend.adapter.clone(), theme::accent()));
        spans.push(Span::styled(
            format!(" {}", describe_backend(backend)),
            theme::text(),
        ));
    }
    spans
}

fn describe_backend(backend: &BackendUsage) -> String {
    // The tilde is the whole point of tracking `approximate`: one vendor rounds
    // before it reports, so a figure including it is an estimate and says so.
    let about = if backend.approximate { "~" } else { "" };
    if backend.input == 0 && backend.output == 0 {
        return format!("{about}{} total", compact(backend.total));
    }
    let split = format!(
        "{about}{} in / {about}{} out",
        compact(backend.input),
        compact(backend.output)
    );
    if backend.total == 0 {
        split
    } else {
        format!("{split} \u{b7} {about}{} total", compact(backend.total))
    }
}

/// A token count at a glance. Exactness below a thousand, magnitude above it.
fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The opening screen in a directory that is not a project yet.
fn render_setup(frame: &mut Frame, app: &App, area: Rect) {
    let Some(plan) = &app.setup else { return };
    let mut lines = vec![
        bold("This directory is not an Ostraka project yet.".to_string()),
        Line::from(""),
        dim(format!("Detected {}.", plan.kind.describe())),
        Line::from(""),
        bold("Pressing i writes:".to_string()),
    ];
    for file in plan.files.iter() {
        let name = file
            .path
            .strip_prefix(&plan.project)
            .unwrap_or(&file.path)
            .display()
            .to_string();
        let (word, colour) = match file.action {
            Action::Create => ("create", theme::OK),
            Action::Append => ("append to", theme::ACCENT),
            Action::AlreadyThere => ("already there", theme::MUTED),
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{word:<14}"), theme::on(colour)),
            Span::raw(name),
        ]));
    }
    let mut tail = vec![
        Line::from(""),
        dim("Nothing already on disk is overwritten. `ostraka init` does the same.".to_string()),
    ];

    // What setting up will not fix, said before the offer is taken rather than
    // by a run failing later in somebody else's words.
    for (mark, said) in warnings(app, plan) {
        tail.push(Line::from(""));
        // Wrapped, because these are sentences rather than labels and the one
        // that overflows is the one explaining what will not work.
        for (i, part) in wrap(&said, area.width.saturating_sub(4) as usize)
            .into_iter()
            .enumerate()
        {
            tail.push(Line::from(vec![
                Span::styled(
                    if i == 0 {
                        format!("{mark}  ")
                    } else {
                        "   ".to_string()
                    },
                    theme::on(theme::WARN),
                ),
                Span::styled(part, theme::text()),
            ]));
        }
    }

    // The listing yields, the warnings do not. Every profile this binary ships
    // is one more line under "Pressing i writes:", and on a short terminal the
    // sixth of them pushed "the gate it writes fails on purpose" off the
    // bottom — which is the one sentence on this screen somebody has to read
    // before pressing the key. Adding a vendor is a file, so the list only
    // grows; what it may not do is grow over the warnings.
    let room = area.height as usize;
    if lines.len() + tail.len() > room {
        let keep = room
            .saturating_sub(tail.len())
            .saturating_sub(HEADING_LINES + 1);
        let hidden = lines.len() - HEADING_LINES - keep;
        lines.truncate(HEADING_LINES + keep);
        lines.push(dim(format!("  … and {hidden} more")));
    }
    lines.extend(tail);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Lines above the file listing in [`render_setup`]: the title, the blank after
/// it, what was detected, another blank, and "Pressing i writes:".
const HEADING_LINES: usize = 5;

/// What will still be wrong after `i`.
///
/// Both of these used to be found out from a failed run: one from git's own
/// words two minutes in, and one from a gate check that was written to fail on
/// purpose and never said so anywhere the operator would look.
fn warnings(app: &App, plan: &Plan) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(blocked) = &app.blocked {
        out.push((
            theme::FAILED,
            format!("{blocked} Nothing will run until that is dealt with, and x walks through it."),
        ));
    }
    if plan.kind == crate::init::Kind::Unknown {
        out.push((
            "!",
            "Ostraka could not tell how this project is verified, so the gate it writes \
             fails on purpose. Edit the checks in ostraka.toml before the first task — a \
             gate that passed everything would be worse."
                .to_string(),
        ));
    }
    out
}

/// What is in the way, and the way out of it, one step at a time.
///
/// The alternative was the sentence this replaces: git's own account of the
/// situation, from two layers down, arriving after a run had been started and
/// a vendor paid, leaving the operator to work out what to do about it.
fn render_fix(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 82u16.min(screen.width);
    let Some(remedy) = &app.remedy else { return };
    let inner = width.saturating_sub(6) as usize;

    let mut lines = vec![
        Line::from(Span::styled("Nothing will run here yet", theme::bold())),
        theme::rule(width.saturating_sub(6)),
    ];
    for part in wrap(&remedy.problem, inner) {
        lines.push(Line::from(Span::styled(part, theme::text())));
    }
    lines.push(Line::from(""));

    for (i, step) in remedy.steps.iter().enumerate() {
        let (mark, colour) = if i < remedy.at {
            (theme::PASSED, theme::OK)
        } else if i == remedy.at {
            (theme::CURSOR, theme::ACCENT)
        } else {
            (" ", theme::MUTED)
        };
        let here = i == remedy.at;
        let voice = if here {
            theme::text().add_modifier(Modifier::BOLD)
        } else {
            theme::text()
        };
        // Wrapped like everything else. A step is a sentence, and the one that
        // overflows is the one telling you what to do about it.
        for (line, part) in wrap(&step.said, inner.saturating_sub(5))
            .into_iter()
            .enumerate()
        {
            if line == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{mark} "), theme::on(colour)),
                    Span::styled(
                        format!("{}  ", i + 1),
                        if here {
                            theme::accent()
                        } else {
                            theme::muted()
                        },
                    ),
                    Span::styled(part, voice),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(part, voice),
                ]));
            }
        }
        // Only for the step about to be taken. What the others will do is not
        // what anybody is deciding about right now.
        if !here {
            continue;
        }
        if let Some(warns) = &step.warns {
            for part in wrap(warns, inner.saturating_sub(6)) {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(part, theme::on(theme::WARN)),
                ]));
            }
        }
        for argv in &step.commands {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(argv.join(" "), theme::accent()),
            ]));
        }
    }

    lines.push(Line::from(""));
    if let Some(said) = &remedy.said {
        for part in wrap(said, inner) {
            lines.push(Line::from(Span::styled(part, theme::on(theme::BAD))));
        }
        lines.push(Line::from(""));
    }
    lines.push(if remedy.failed {
        dim("esc closes this".to_string())
    } else if remedy.done() {
        Line::from(Span::styled(
            "Nothing is in the way now. esc closes this.",
            theme::on(theme::OK),
        ))
    } else if remedy
        .steps
        .get(remedy.at)
        .is_some_and(|step| !step.commands.is_empty())
    {
        Line::from(vec![
            Span::styled("y", theme::accent()),
            Span::styled("  do this step      ", theme::muted()),
            Span::styled("s", theme::accent()),
            Span::styled("  skip it      ", theme::muted()),
            Span::styled("esc", theme::accent()),
            Span::styled("  close", theme::muted()),
        ])
    } else {
        // Offering `y` for a step with nothing to run would mark it done and
        // change nothing, which is worse than saying it is not ours to take.
        Line::from(vec![
            Span::styled("This one is yours to do.      ", theme::muted()),
            Span::styled("esc", theme::accent()),
            Span::styled("  close", theme::muted()),
        ])
    });

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// Leaving, asked rather than assumed.
///
/// Quitting is not free here: it can discard a task that was being written and
/// it can stop a run that is going. Both are worth a sentence and a key.
fn render_leaving(frame: &mut Frame, app: &App, screen: Rect) {
    let mut lines = vec![Line::from(Span::styled(
        "Leave the browser?",
        theme::bold(),
    ))];
    if app.anything_running() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "A run is going. Leaving asks it to stop and waits for it.",
            theme::on(theme::WARN),
        )));
    }
    if !app.pane().prompt.trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "The task in the box is not written down anywhere.",
            theme::on(theme::WARN),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("y", theme::accent()),
        Span::styled("  leave        ", theme::muted()),
        Span::styled("n", theme::accent()),
        Span::styled(" / ", theme::muted()),
        Span::styled("esc", theme::accent()),
        Span::styled("  stay", theme::muted()),
    ]));

    let area = theme::centred(screen, 66, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// Every key the screen answers to, in one place someone can read.
///
/// The commands half is derived from `Command::ALL` rather than written out
/// beside it. It was written out, and a command added to the list did not
/// reach this dialog — so the one screen somebody opens when they cannot
/// remember a key was the one screen that did not know about it. That is the
/// drift the single list exists to prevent, and this was the reader that had
/// opted out of it.
///
/// What is written out is the half that has no command behind it: typing,
/// moving through the task, the mentions. Those are the box's own behaviour,
/// not entries in a list of actions.
fn render_keys(frame: &mut Frame, app: &App, screen: Rect) {
    let mode = app.thread().mode;
    let situation = app.situation();

    let mut rows: Vec<(String, String, Section)> = Vec::new();
    let mut writing = |key: &str, what: &str| {
        rows.push((key.to_string(), what.to_string(), Section::Writing));
    };
    writing("type", "what you type is the task");
    // What enter actually does here. "run it" was true when a thread opened in
    // auto and is a plain lie in ask, which is where a thread opens now — on
    // the one screen somebody reads when they are already confused.
    writing(
        "enter",
        match mode {
            Mode::Ask => "answer it \u{2014} nothing is written",
            Mode::Plan => "write a plan for it \u{2014} nothing is written",
            Mode::Loop => "run it, and try again with the reason if it is refused",
            Mode::Auto => "run it",
        },
    );
    writing(
        "shift-tab",
        "ask, plan, loop or auto \u{2014} what enter does",
    );
    writing("alt-enter", "another line, for a task that needs one");
    writing("up / down", "what you have asked here before");
    writing(
        "left / right",
        "move through the task; a click puts the cursor",
    );
    writing("@", "a file, a directory or an agent; tab completes it");
    writing(
        "esc / n",
        "put the task aside and take the keys; n gives the box back",
    );

    // Every command, from the one list. Offered ones first, so what can be
    // done now is at the top; the rest are shown greyed rather than hidden,
    // because a key somebody read about yesterday and cannot find today reads
    // as a browser that has lost it.
    let offered = Command::offered(situation);
    let row = |command: Command, section| {
        (
            format!(
                "{:<6} {} {}",
                command.key(),
                label(Chord::Leader),
                command.leader()
            ),
            command.about().to_string(),
            section,
        )
    };
    // Partitioned rather than walked in order. Emitting a heading whenever the
    // section changed put "commands" on the screen three times, because the
    // list interleaves what can be done now with what cannot.
    rows.extend(
        Command::ALL
            .into_iter()
            .filter(|c| offered.contains(c))
            .map(|c| row(c, Section::Commands)),
    );
    rows.extend(
        Command::ALL
            .into_iter()
            .filter(|c| !offered.contains(c))
            .map(|c| row(c, Section::Unavailable)),
    );

    let mut around = |key: String, what: &str| {
        rows.push((key, what.to_string(), Section::Around));
    };
    around(label(Chord::NewPane).to_string(), "another line of work");
    around(panes_label().to_string(), "move between them");
    around(
        "< > + - =".to_string(),
        "move, widen, narrow, even out the panes",
    );
    around("pgup / pgdn".to_string(), "scroll");
    around(label(Chord::Commands).to_string(), "the commands, by name");
    around(
        label(Chord::Leader).to_string(),
        "leader: the same commands, one key away",
    );

    // The column is as wide as the widest key in it, plus a gap. Fixed at
    // thirteen, `ctrl-] / ctrl-[` overran it and the description it was meant
    // to be separated from began in the next cell.
    let column = rows
        .iter()
        .map(|(key, _, _)| key.chars().count())
        .max()
        .unwrap_or(0)
        + 2;

    let mut lines = vec![
        Line::from(Span::styled("keys", theme::bold())),
        Line::from(""),
    ];
    let mut last: Option<Section> = None;
    for (key, what, section) in &rows {
        if last != Some(*section) {
            if last.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(dim(section.heading().to_string()));
            last = Some(*section);
        }
        let style = if *section == Section::Unavailable {
            theme::muted()
        } else {
            theme::accent()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<column$}"), style),
            Span::styled(what.clone(), theme::text()),
        ]));
    }

    // As wide as the terminal allows, up to a line length somebody can read
    // across. Fixed at seventy-six, the descriptions were clipped mid-word on
    // terminals with room to spare — a reference that stops mid-sentence is
    // one somebody has to guess the end of.
    let area = theme::centred(
        screen,
        screen.width.saturating_sub(4).min(100),
        screen.height.saturating_sub(2),
    );
    frame.render_widget(Clear, area);
    let block = theme::panel(true).padding(Padding::horizontal(2));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Scrolled rather than truncated. There are more keys than rows on most
    // terminals, and "… 5 more lines" is a dialog telling somebody the thing
    // they came for might be one of the five it will not show them.
    //
    // The way out is pinned to the last row instead of being the last line of
    // what scrolls. A dialog whose footer can be scrolled off is one somebody
    // is stuck in, which is the invariant the truncating version was keeping
    // and this nearly threw away.
    let body = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let foot = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    let hidden = lines.len().saturating_sub(body.height as usize) as u16;
    let scroll = app.keys_scroll.min(hidden);
    let left = hidden.saturating_sub(scroll);

    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body);
    frame.render_widget(
        Paragraph::new(dim(if hidden > 0 {
            format!("esc closes this \u{b7} up / down scrolls \u{b7} {left} more lines below")
        } else {
            "esc closes this".to_string()
        })),
        foot,
    );
}

/// What a row of the keys dialog is about.
///
/// Grouped because a flat list of everything puts "quit" and "what you type is
/// the task" at the same level, and somebody opening this wants to know how to
/// start before they want to know how to leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Writing,
    Commands,
    Unavailable,
    Around,
}

impl Section {
    fn heading(self) -> &'static str {
        match self {
            Section::Writing => "writing",
            Section::Commands => "commands",
            Section::Unavailable => "commands \u{2014} not right now",
            Section::Around => "getting around",
        }
    }
}

/// The commands, by name, for the times nobody remembers the key.
fn render_palette(frame: &mut Frame, app: &App, screen: Rect) {
    let commands = app.commands();
    let mut lines = vec![
        Line::from(vec![
            Span::styled("> ", theme::accent()),
            Span::styled(app.query.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
        ]),
        theme::rule(82),
    ];

    if commands.is_empty() {
        lines.push(dim("no command matches that".to_string()));
    }
    for (i, command) in commands.iter().enumerate() {
        let here = i == app.pick;
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(
                format!(" {:<24}", command.name()),
                if here {
                    theme::text().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
            Span::styled(format!("{:<6}", command.key()), theme::accent()),
            Span::styled(command.about().to_string(), theme::muted()),
        ]));
    }

    let area = theme::centred(screen, 86, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// A vendor that could not run, and the profiles that could take its seat.
fn render_fallback(frame: &mut Frame, app: &App, screen: Rect) {
    use super::session::Seat;
    let Some(offer) = &app.fallback else {
        return;
    };
    let (seat, who) = match offer.seat {
        Seat::Writes => ("write", "the author"),
        Seat::Reviews => ("review", "the reviewer"),
    };
    let who = offer.failed.clone().unwrap_or_else(|| who.to_string());
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{who} could not run"),
            theme::on(theme::BAD).add_modifier(Modifier::BOLD),
        )),
        dim(truncate(&offer.why, 80)),
        theme::rule(82),
        Line::from(Span::styled(
            format!("run the task again, with another profile to {seat}:"),
            theme::text(),
        )),
    ];
    if app.agents_loading.is_some() {
        lines.push(dim(
            "asking each profile whether it answers\u{2026}".to_string()
        ));
    } else if app.agents.is_empty() {
        lines.push(dim(
            "no other profile here \u{2014} esc keeps the task".to_string()
        ));
    }
    for (i, agent) in app.agents.iter().enumerate() {
        let here = i == app.pick;
        let (mark, colour) = if agent.ready {
            (theme::PASSED, theme::OK)
        } else {
            (theme::FAILED, theme::BAD)
        };
        let note = if agent.configured {
            truncate(&agent.note, 50)
        } else {
            truncate(
                &format!("{} \u{2014} not configured, choosing writes it", agent.note),
                60,
            )
        };
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(format!(" {mark}  "), theme::on(colour)),
            Span::styled(
                format!("{:<22}", truncate(&agent.id, 21)),
                if here {
                    theme::text().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
            Span::styled(note, theme::muted()),
        ]));
    }
    lines.push(dim(
        "a profile that answers can still be out of credit \u{2014} the run will say".to_string(),
    ));
    lines.push(dim(
        "enter runs it again \u{b7} esc keeps the task in the box".to_string(),
    ));

    let area = theme::centred(screen, 86, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// The models each profile lists, filtered by what has been typed.
fn render_models(frame: &mut Frame, app: &App, screen: Rect) {
    const SHOWN: usize = 14;
    let rows = app.model_rows();
    let chosen = format!(
        "writes   {}   model   {}",
        app.thread().adapter.as_deref().unwrap_or("automatic"),
        app.thread().model.as_deref().unwrap_or("its own default")
    );
    let mut lines = vec![
        Line::from(vec![
            Span::styled("> ", theme::accent()),
            Span::styled(app.query.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
        ]),
        Line::from(Span::styled(chosen, theme::muted())),
        theme::rule(82),
    ];
    if app.models_loading.is_some() {
        lines.push(dim("asking each profile what it has\u{2026}".to_string()));
    } else if app.models.is_empty() {
        lines.push(dim(
            "no profile here says how to list its models \u{2014} a [models] table in its profile does"
                .to_string(),
        ));
    } else if rows.is_empty() {
        lines.push(dim("no model matches that".to_string()));
    }
    let first = app.pick.saturating_sub(SHOWN - 1);
    for (i, row) in rows.iter().enumerate().skip(first).take(SHOWN) {
        let here = i == app.pick;
        let (model, style) = match &row.model {
            Some(model) => (model.clone(), theme::text()),
            None => ("its own default".to_string(), theme::muted()),
        };
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(
                format!(" {:<22}", truncate(&row.profile, 21)),
                theme::accent(),
            ),
            Span::styled(
                truncate(&model, 56),
                if here {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            ),
        ]));
    }
    if rows.len() > SHOWN {
        lines.push(dim(format!(
            "{} models \u{2014} type to narrow them",
            rows.len()
        )));
    }
    for note in &app.models_notes {
        lines.push(Line::from(Span::styled(
            truncate(note, 80),
            theme::on(theme::WARN),
        )));
    }
    lines.push(dim("enter picks \u{b7} esc closes".to_string()));

    let area = theme::centred(screen, 86, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// The runs recorded here, as somewhere to look one up.
///
/// A dialog rather than a column: it costs half the width of the screen to
/// stand there permanently, and it is looked at once in a while rather than
/// read while working.
fn render_runs(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 92u16.min(screen.width);
    let shown = 12usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("/ ", theme::accent()),
            Span::styled(app.filter.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
            Span::styled(format!("     {}", counted(app)), theme::muted()),
        ]),
        theme::rule(width.saturating_sub(6)),
    ];

    if app.matching.is_empty() {
        lines.push(dim(if app.runs.is_empty() {
            "Nothing has been run here yet.".to_string()
        } else {
            "Nothing matches that.".to_string()
        }));
    }

    let first = app.selected.saturating_sub(shown - 1).min(app.list_top);
    for (i, run) in app
        .matching
        .iter()
        .enumerate()
        .skip(first)
        .take(shown)
        .filter_map(|(i, index)| app.runs.get(*index).map(|run| (i, run)))
    {
        let here = i == app.selected;
        let (mark, colour) = marker(run.outcome);
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::on(colour)),
            Span::styled(format!(" {mark}  "), theme::on(colour)),
            Span::styled(format!("{:<13}", when(&run.run_id)), theme::muted()),
            Span::styled(
                truncate(&described(&run.prompt), width as usize - 44),
                if here {
                    theme::text().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(dim("enter opens it \u{b7} esc closes this".to_string()));

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

/// Who writes and who reviews, and the choosing of them.
///
/// Routing picks a pair on its own and is usually right — it prefers a
/// reviewer that is a *different binary* from the author, which is the
/// property that makes a review worth having. This is for when it is not:
/// naming one is a decision, and a decision is honoured.
/// What finished runs left, and the one key that clears it.
///
/// A list before an action, the way the command line is a dry run before
/// `--apply`. Removing a directory is not a keystroke to offer without saying
/// what it will take, and the sentence about branches is there because that is
/// the thing somebody is actually afraid of.
fn render_prune(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 84u16.min(screen.width);
    let mut lines = vec![
        Line::from(Span::styled("worktrees finished runs left", theme::bold())),
        theme::rule(width.saturating_sub(6)),
    ];

    // From the width this dialog actually got, not a number that happens to fit
    // the terminal it was written on: `width` is already `84.min(screen)`, so a
    // narrower screen would have overflowed a fixed 70 and a wider one would
    // have cut paths it had room for. Two for the indent, four for the frame
    // and its padding.
    let room = width.saturating_sub(6) as usize;

    if app.leftovers.is_empty() {
        lines.push(dim("nothing to prune".to_string()));
    } else {
        for l in &app.leftovers {
            let shown = l
                .path
                .strip_prefix(&app.workspace.root)
                .unwrap_or(&l.path)
                .display()
                .to_string();
            lines.push(Line::from(vec![
                Span::styled("  ", theme::text()),
                Span::styled(truncate(&shown, room.saturating_sub(2)), theme::text()),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(dim(
            "Branches and run records are untouched. A worktree can be made again; \
             the commit a run produced is what matters and it stays."
                .to_string(),
        ));
    }

    lines.push(Line::from(""));
    lines.push(dim(if app.leftovers.is_empty() {
        "esc closes this".to_string()
    } else {
        "y removes them \u{b7} esc leaves them alone".to_string()
    }));

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

fn render_agents(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 84u16.min(screen.width);
    let named = |chosen: &Option<String>| match chosen {
        Some(id) => (id.clone(), theme::ACCENT),
        None => ("automatic".to_string(), theme::MUTED),
    };
    let (author, author_colour) = named(&app.thread().adapter);
    let (reviewer, reviewer_colour) = named(&app.thread().review_adapter);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("writes   ", theme::muted()),
            Span::styled(author, theme::on(author_colour)),
            Span::styled("      reviews  ", theme::muted()),
            Span::styled(reviewer, theme::on(reviewer_colour)),
        ]),
        theme::rule(width.saturating_sub(6)),
    ];

    if app.agents.is_empty() {
        lines.push(dim(
            "no adapter profiles here, and none of the ones this build ships are installed"
                .to_string(),
        ));
    }
    for (i, agent) in app.agents.iter().enumerate() {
        let here = i == app.pick;
        let mut role = String::new();
        if app.thread().adapter.as_deref() == Some(agent.id.as_str()) {
            role.push_str("writes ");
        }
        if app.thread().review_adapter.as_deref() == Some(agent.id.as_str()) {
            role.push_str("reviews");
        }
        let (mark, colour) = if agent.ready {
            (theme::PASSED, theme::OK)
        } else {
            (theme::FAILED, theme::BAD)
        };
        // Installed and unwritten reads differently from configured and ready:
        // choosing it is also agreeing to it, and the row says so before the
        // key is pressed rather than after.
        //
        // In the note rather than the role column. The role column is nine
        // wide and `{:<9}` is a floor, not a ceiling — a longer string there
        // widens the column and pushes the version off the end of the line.
        let note = if agent.configured {
            truncate(&agent.note, 40)
        } else {
            truncate(
                &format!("{} — not configured, choosing writes it", agent.note),
                60,
            )
        };
        lines.push(Line::from(vec![
            Span::styled(if here { theme::CURSOR } else { " " }, theme::accent()),
            Span::styled(format!(" {mark}  "), theme::on(colour)),
            Span::styled(
                format!("{:<16}", agent.id),
                if here {
                    theme::text().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                },
            ),
            Span::styled(format!("{role:<9}"), theme::accent()),
            Span::styled(note, theme::muted()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(dim(
        "a writes \u{b7} r reviews \u{b7} x back to automatic \u{b7} esc closes this".to_string(),
    ));

    let area = theme::centred(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
}

fn summary_lines(app: &App, run: &RunSummary, width: usize) -> Vec<Line<'static>> {
    let (_, colour) = marker(run.outcome);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(run.run_id.clone(), theme::bold()),
            Span::raw("   "),
            Span::styled(outcome_word(run.outcome).to_string(), theme::on(colour)),
        ]),
        Line::from(""),
    ];
    lines.extend(field("task", &described(&run.prompt), width));
    if !run.adapter.is_empty() {
        lines.extend(field(
            "author",
            &format!("{} ({})", run.author, run.adapter),
            width,
        ));
    }
    if let Some(reviewer) = &run.reviewer {
        let verdict = app
            .record
            .as_ref()
            .and_then(|r| r.approval.as_ref())
            .map(|a| describe_verdict(&a.verdict))
            .unwrap_or_default();
        lines.extend(field("reviewer", &format!("{reviewer}{verdict}"), width));
    }
    lines.push(Line::from(""));
    lines
}

fn check_lines(app: &App, run: &RunSummary) -> Vec<Line<'static>> {
    let Some(record) = &app.record else {
        return vec![dim(format!(
            "checks: {}/{} passed",
            run.checks_passed, run.checks_total
        ))];
    };
    if record.checks.is_empty() {
        return vec![dim("no checks ran".to_string())];
    }

    let mut lines = Vec::new();
    for check in &record.checks {
        let (mark, word, colour) = check_mark(check);
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), theme::on(colour)),
            Span::styled(word, theme::on(colour)),
            Span::styled(
                format!("  {:<10} {}ms", check.name, check.duration_ms),
                theme::text(),
            ),
        ]));
        // Only for a failure. A passing check's output is noise, and a failing
        // one is the reason someone opened this screen.
        if !check.passed() {
            for line in tail(&check.stderr, 12)
                .into_iter()
                .chain(tail(&check.stdout, 12))
            {
                lines.push(dim(format!("     {line}")));
            }
        }
    }
    lines
}

fn event_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if app.events.is_empty() {
        return vec![dim("no events recorded".to_string())];
    }
    let mut lines = Vec::new();
    for event in &app.events {
        let (kind, text, colour) = describe_event(event);
        lines.extend(event_rows(
            vec![Span::styled(format!("{kind} "), theme::muted())],
            vec![Span::raw(" ".repeat(kind.len() + 1))],
            &text,
            theme::on(colour),
            width,
        ));
    }
    lines
}

/// One event, as many rows as its text needs at this width.
///
/// The first row carries `first` in front of it and the rest carry `rest`,
/// which is the same width in blanks, so a folded message reads as one block
/// under its label rather than wrapping back to the margin.
///
/// Folded, not cut. The pane used to cut every event to one row with an
/// ellipsis, so an agent's longer answers — the ones worth reading — were
/// never on the screen whole, and selecting them copied the ellipsis too.
fn event_rows(
    first: Vec<Span<'static>>,
    rest: Vec<Span<'static>>,
    text: &str,
    style: ratatui::style::Style,
    width: u16,
) -> Vec<Line<'static>> {
    let taken: usize = first.iter().map(|span| span.width()).sum();
    // What is left beside the label, and never more. A floor here would let a
    // row run past a narrow pane, which is the cut this exists to stop; `fold`
    // copes with a row of one cell.
    let room = (width as usize).saturating_sub(taken).max(1);
    fold(text, room)
        .into_iter()
        .enumerate()
        .map(|(n, row)| {
            let mut spans = if n == 0 { first.clone() } else { rest.clone() };
            spans.push(Span::styled(row, style));
            Line::from(spans)
        })
        .collect()
}

/// Text broken into rows no wider than `width` display columns.
///
/// Unlike `wrap`, which is for sentences this screen writes, this is for what
/// a vendor wrote: its own line breaks are kept, its indentation is kept, and
/// a word longer than the row — a path, a hash, a URL — is broken rather than
/// left to run off the edge. A row breaks at the last space that fits where
/// there is one. Widths are display widths, so a wide character counts as the
/// two cells it takes.
fn fold(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let width = width.max(1);
    let mut rows = Vec::new();
    for source in text.split('\n') {
        let source = source.trim_end_matches('\r');
        let mut row = String::new();
        let mut used = 0usize;
        // Where the row could be broken at a space: the byte offset after it,
        // and the width up to it.
        let mut space: Option<(usize, usize)> = None;
        for character in source.chars() {
            let cells = UnicodeWidthChar::width(character).unwrap_or(0);
            // Until the character fits: a break at a space can leave a tail
            // that, with this character, is still too wide.
            while used + cells > width && !row.is_empty() {
                match space.take() {
                    // Broken after the last space that fit. The space stays on
                    // the row it ends, and is trimmed off it.
                    Some((at, before)) => {
                        let tail = row.split_off(at);
                        rows.push(row.trim_end().to_string());
                        row = tail;
                        used -= before;
                    }
                    // No space to break at: a word longer than the row.
                    None => {
                        rows.push(std::mem::take(&mut row));
                        used = 0;
                    }
                }
            }
            row.push(character);
            used += cells;
            // Not a space in leading indentation: breaking there would push a
            // row of nothing but spaces, which trims to an empty row.
            if character == ' ' && row.chars().any(|c| c != ' ') {
                space = Some((row.len(), used));
            }
        }
        rows.push(row);
    }
    rows
}

fn diff_lines(app: &App) -> Vec<Line<'static>> {
    match &app.diff {
        None => vec![dim("reading the change\u{2026}".to_string())],
        // Two causes, one appearance, and the message names both rather than
        // guessing: a refused run never committed, and an old approved one may
        // have had its branch merged away since.
        Some(None) => vec![
            dim("this run produced no commit.".to_string()),
            dim("It was refused, or its branch has since been merged away.".to_string()),
        ],
        Some(Some(text)) => text
            .lines()
            .map(|line| {
                let colour = match line.as_bytes().first() {
                    Some(b'+') if !line.starts_with("+++") => theme::OK,
                    Some(b'-') if !line.starts_with("---") => theme::BAD,
                    Some(b'@') => theme::ACCENT,
                    _ => theme::MUTED,
                };
                Line::from(Span::styled(line.to_string(), theme::on(colour)))
            })
            .collect(),
    }
}

fn dim(text: String) -> Line<'static> {
    Line::from(Span::styled(text, theme::muted()))
}

fn bold(text: String) -> Line<'static> {
    Line::from(Span::styled(text, theme::bold()))
}

/// A run recorded before the record carried what was asked.
fn described(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        "(recorded before runs kept the task text)".to_string()
    } else {
        prompt.to_string()
    }
}

/// A labelled value, wrapped under its own label.
fn field(name: &str, value: &str, width: usize) -> Vec<Line<'static>> {
    const LABEL: usize = 10;
    wrap(value, width.saturating_sub(LABEL))
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            Line::from(vec![
                Span::styled(
                    if i == 0 {
                        format!("{name:<LABEL$}")
                    } else {
                        " ".repeat(LABEL)
                    },
                    theme::muted(),
                ),
                Span::styled(part, theme::text()),
            ])
        })
        .collect()
}

fn describe_verdict(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Approve => " \u{2014} approve".to_string(),
        Verdict::Reject { reason } => format!(" \u{2014} reject: {reason}"),
    }
}

fn marker(outcome: Option<Outcome>) -> (&'static str, Color) {
    match outcome {
        Some(Outcome::Approved) => (theme::PASSED, theme::OK),
        Some(Outcome::Rejected) => (theme::FAILED, theme::BAD),
        Some(Outcome::Failed) => ("!", theme::HALTED),
        None => ("\u{b7}", theme::MUTED),
    }
}

fn outcome_word(outcome: Option<Outcome>) -> &'static str {
    outcome.map_or("unfinished", Outcome::as_str)
}

/// `20260907T000300Z` out of a run id, as something a person reads.
fn when(run_id: &str) -> String {
    let Some((_, stamp)) = run_id.rsplit_once('-') else {
        return String::new();
    };
    let digits: Vec<char> = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 12 {
        return String::new();
    }
    let at = |a: usize, b: usize| digits[a..b].iter().collect::<String>();
    format!("{}-{} {}:{}", at(4, 6), at(6, 8), at(8, 10), at(10, 12))
}

fn truncate(text: &str, width: usize) -> String {
    let flat = text.replace('\n', " ");
    if width == 0 || flat.chars().count() <= width {
        return flat;
    }
    let kept: String = flat.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

/// Breaks text into lines of at most `width` characters, on spaces where it can.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let extra = if line.is_empty() { 0 } else { 1 };
        if !line.is_empty() && line.chars().count() + extra + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn tail(text: &str, lines: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all.iter()
        .rev()
        .take(lines)
        .rev()
        .map(|l| truncate(l, 70))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::session::{Finished, Session};
    use ostraka_core::gate::{Approval, CheckRecord};
    use ostraka_core::identity::ActorId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn summary(run_id: &str, prompt: &str, outcome: Option<Outcome>) -> RunSummary {
        RunSummary {
            run_id: run_id.to_string(),
            started_at: "2026-09-07T00:03:00Z".to_string(),
            prompt: prompt.to_string(),
            author: ActorId::new("archon"),
            adapter: "claude-code".to_string(),
            repository: "only".to_string(),
            reviewer: Some(ActorId::new("ephor")),
            outcome,
            checks_passed: 3,
            checks_total: 4,
            usage: Vec::new(),
        }
    }

    fn record(run_id: &str, prompt: &str, checks: Vec<CheckRecord>) -> RunRecord {
        RunRecord {
            run_id: run_id.into(),
            task_id: "t1".into(),
            prompt: prompt.into(),
            author: ActorId::new("archon"),
            adapter: "claude-code".into(),
            repository: "only".into(),
            started_at: String::new(),
            finished_at: None,
            checks,
            approval: None,
            usage: Vec::new(),
            outcome: Some(Outcome::Rejected),
        }
    }

    fn check(name: &str, code: i32, stderr: &str) -> CheckRecord {
        CheckRecord {
            name: name.to_string(),
            cmd: name.to_string(),
            exit_code: Some(code),
            stdout: String::new(),
            stderr: stderr.to_string(),
            duration_ms: 126,
        }
    }

    fn said(phase: Phase, text: &str) -> Step {
        Step::Said {
            phase,
            event: Event::Message {
                text: text.to_string(),
                raw: None,
            },
        }
    }

    fn finished(run_id: &str, outcome: Outcome, summary: &str) -> Finished {
        Finished {
            run_id: run_id.to_string(),
            outcome: Some(outcome),
            summary: summary.to_string(),
            plan: None,
        }
    }

    /// What an agent said is on the screen whole. It was cut to one row with an
    /// ellipsis, so a long answer — the kind worth reading — never was, and
    /// selecting it copied the ellipsis.
    #[test]
    fn what_an_agent_said_is_folded_to_the_pane_rather_than_cut() {
        let mut app = App::new(nowhere(), Vec::new());
        let long = format!("START {} END", "word ".repeat(40));
        turn(
            &mut app,
            "say something long",
            vec![
                Step::Entered(Phase::Authoring),
                said(Phase::Authoring, &long),
                said(Phase::Authoring, "first line\n  indented second\nthird"),
            ],
            Some(finished(
                "t1-20260920T000000Z",
                Outcome::Approved,
                "approved",
            )),
        );
        let out = screen(&mut app, 80, 40);
        assert!(out.contains("START"), "{out}");
        assert!(out.contains("END"), "the end of it was cut:\n{out}");
        // Asked of the transcript's own rows: the status line under it is
        // shortened to fit on purpose, and is not what this is about.
        assert!(
            !out.lines()
                .any(|l| l.contains("word") && l.contains('\u{2026}')),
            "an ellipsis stands in for text:\n{out}"
        );
        // Its own line breaks are kept, indentation included, rather than being
        // run together into one line.
        assert!(out.contains("first line"), "{out}");
        assert!(out.contains("  indented second"), "{out}");
        assert!(
            !out.contains("first line   indented"),
            "the line breaks were flattened:\n{out}"
        );
    }

    #[test]
    fn fold_keeps_line_breaks_and_breaks_at_a_space_that_fits() {
        assert_eq!(fold("", 10), vec![String::new()]);
        assert_eq!(fold("one two three", 8), vec!["one two", "three"]);
        assert_eq!(fold("a\nb", 10), vec!["a", "b"]);
        // Indentation is part of what was said.
        assert_eq!(fold("  x", 10), vec!["  x"]);
        // A word longer than the row is broken rather than left to run off it.
        assert_eq!(fold("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        // Widths are display widths: a wide character takes two cells.
        assert_eq!(
            fold("\u{4e16}\u{4e16}\u{4e16}", 4),
            vec!["\u{4e16}\u{4e16}", "\u{4e16}"]
        );
        // No row is ever wider than asked.
        // Measured in display width, the way it is drawn: a character count
        // would pass rows of wide characters twice as wide as asked.
        use unicode_width::UnicodeWidthStr;
        for text in [
            "lorem ipsum dolor ".repeat(20),
            "\u{4e16}\u{754c} ".repeat(20),
            "a\u{4e16}b\u{754c}c".repeat(10),
        ] {
            for row in fold(&text, 17) {
                assert!(row.width() <= 17, "{row:?} is {} wide", row.width());
            }
        }
    }

    /// From the #87 review: leading indentation is never a break point, so it
    /// cannot leave an empty row, and a break at a space that leaves too
    /// little room is followed by another, so no row runs a cell over.
    #[test]
    fn fold_never_breaks_at_indentation_or_runs_over_after_a_space() {
        use unicode_width::UnicodeWidthStr;
        assert_eq!(fold(" \u{4e16}\u{4e16}", 3), vec![" \u{4e16}", "\u{4e16}"]);
        assert_eq!(fold("    abcdefgh", 6), vec!["    ab", "cdefgh"]);
        for text in [
            " \u{4e16}\u{4e16}\u{4e16}".to_string(),
            "   \u{4e16} \u{754c}\u{754c}\u{754c}".to_string(),
            "  a \u{4e16}\u{4e16}\u{4e16}\u{4e16}".to_string(),
            "      indented words that wrap".to_string(),
            " x\u{4e16}".repeat(8),
        ] {
            for width in 2..12 {
                for row in fold(&text, width) {
                    assert!(
                        row.width() <= width,
                        "{text:?} at {width}: {row:?} is {} wide",
                        row.width()
                    );
                    assert!(!row.is_empty(), "{text:?} at {width} left an empty row");
                }
            }
        }
    }

    /// A pane so narrow the label takes nearly all of it still gets rows that
    /// fit, rather than rows held to a floor that runs past its edge.
    #[test]
    fn a_folded_event_never_runs_past_a_narrow_pane() {
        use unicode_width::UnicodeWidthStr;
        let rows = event_rows(
            vec![Span::raw("| said ")],
            vec![Span::raw("       ")],
            &"word ".repeat(10),
            ratatui::style::Style::default(),
            10,
        );
        for row in &rows {
            let width: usize = row.spans.iter().map(|s| s.content.width()).sum();
            assert!(width <= 10, "{row:?} is {width} wide");
        }
    }

    /// A workspace with two profiles, one writing a run and marked to write
    /// the next, one missing — the state the side pane is a picture of.
    fn with_agents() -> App {
        use ostraka_runtime::progress::Phase;
        use ostraka_runtime::record::Live;
        let mut app = App::new(nowhere(), Vec::new());
        app.profile_ids = vec!["zz-writer".into(), "zz-reader".into()];
        app.probes = vec![
            Agent {
                id: "zz-writer".into(),
                ready: true,
                note: "writer 1.2.3".into(),
                configured: true,
            },
            Agent {
                id: "zz-reader".into(),
                ready: false,
                note: "not installed".into(),
                configured: true,
            },
        ];
        app.live = vec![(
            "t9-run".into(),
            Live {
                author: "zz-writer".into(),
                reviewer: "zz-reader".into(),
                phase: Phase::Authoring,
            },
        )];
        app.next_pair = Some(("zz-writer".into(), "zz-reader".into()));
        app
    }

    #[test]
    fn the_agents_beside_the_work_show_what_the_state_says() {
        let mut app = with_agents();
        let out = screen(&mut app, 120, 24);
        assert!(app.side_shown);
        for want in [
            "zz-writer",
            "ready",
            "writer 1.2.3",
            "writes next",
            "writing t9-run",
            "zz-reader",
            "missing",
            "reviews next",
            "idle",
        ] {
            assert!(out.contains(want), "{want:?} is not on:\n{out}");
        }
    }

    fn with_tasks() -> App {
        use super::super::roster::{TaskGroup, TaskRow, TaskState};
        use ostraka_runtime::progress::Phase;
        let mut app = with_agents();
        let row = |id: &str, prompt: &str, state| TaskRow {
            id: id.into(),
            prompt: prompt.into(),
            state,
            needs_person: false,
        };
        app.task_groups = vec![
            TaskGroup {
                repository: "api".into(),
                waiting: 1,
                going: 1,
                done: 4,
                rows: vec![
                    row(
                        "k20260920T101500Z-0",
                        "fix login",
                        TaskState::Going(Some(Phase::Gating)),
                    ),
                    row("k20260920T101600Z-0", "add rate limit", TaskState::Waiting),
                    row(
                        "k20260920T090000Z-0",
                        "bump deps",
                        TaskState::Done(Some("approved".into())),
                    ),
                ],
            },
            TaskGroup {
                repository: "web".into(),
                waiting: 1,
                going: 0,
                done: 0,
                rows: vec![row("k20260920T110000Z-2", "dark mode", TaskState::Waiting)],
            },
        ];
        app
    }

    #[test]
    fn tasks_are_listed_under_the_agents_by_repository() {
        let mut app = with_tasks();
        let out = screen(&mut app, 120, 30);
        for want in [
            "tasks",
            "\u{25be} api",
            "\u{25cf}1 \u{25cb}1 \u{2713}4",
            "101500 gate fix login",
            "101600 add rate limit",
            "090000 bump deps",
            "\u{25be} web",
            "110000-2 dark mode",
        ] {
            assert!(out.contains(want), "{want:?} is not on:\n{out}");
        }
        assert_eq!(app.task_headers.len(), 2, "both headers answer a click");
    }

    /// Profiles first: on a short terminal the task list gives way, headers
    /// last, and the agents above it are drawn whole.
    #[test]
    fn a_short_pane_keeps_the_agents_and_folds_the_tasks() {
        let mut app = with_tasks();
        let out = screen(&mut app, 120, 20);
        assert!(out.contains("reviews next"), "the agents were cut:\n{out}");
        assert!(
            out.contains("\u{25b8} web") || out.contains("more"),
            "{out}"
        );
        assert!(
            !out.contains("dark mode"),
            "a task outlived its header:\n{out}"
        );
    }

    /// Below the threshold the width is the work's, and nothing is drawn,
    /// probed or read for a pane nobody can see.
    #[test]
    fn the_agents_step_aside_on_a_narrow_terminal() {
        let mut app = with_agents();
        let narrow = screen(&mut app, super::super::roster::SHOWN_FROM - 1, 24);
        assert!(!app.side_shown);
        assert!(!narrow.contains("zz-writer"), "{narrow}");

        let wide = screen(&mut app, super::super::roster::SHOWN_FROM, 24);
        assert!(app.side_shown);
        assert!(wide.contains("zz-writer"), "{wide}");
    }

    #[test]
    fn hidden_agents_stay_hidden_however_wide_the_terminal() {
        let mut app = with_agents();
        app.side = false;
        let out = screen(&mut app, 200, 24);
        assert!(!app.side_shown);
        assert!(!out.contains("zz-writer"), "{out}");
    }

    /// A workspace with no profiles has nothing to list, and a heading over
    /// nothing is not worth the width.
    #[test]
    fn no_profiles_means_no_pane() {
        let mut app = App::new(nowhere(), Vec::new());
        let _ = screen(&mut app, 200, 24);
        assert!(!app.side_shown);
    }

    #[test]
    fn an_at_in_the_box_offers_what_it_could_name() {
        let mut app = App::new(nowhere(), Vec::new());
        app.mentionable = Mentionable {
            loaded: true,
            repository: None,
            agents: vec!["codex".into()],
            paths: vec!["src/".into(), "src/main.rs".into()],
        };
        app.pane_mut().prompt = "fix @s".to_string();
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("@src/"), "{out}");
        assert!(out.contains("directory"), "{out}");

        // An address is text, and offers nothing.
        app.pane_mut().prompt = "mail me@s".to_string();
        let address = screen(&mut app, 100, 20);
        assert!(!address.contains("directory"), "{address}");
    }

    #[test]
    fn the_box_says_what_enter_will_do() {
        let mut app = App::new(nowhere(), Vec::new());
        assert!(screen(&mut app, 100, 20).contains("\u{203a} ask"));

        app.thread_mut().mode = Mode::Plan;
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("\u{203a} plan"), "{out}");

        app.thread_mut().pending_plan = Some(("a task".into(), "1. a step".into()));
        let waiting = screen(&mut app, 100, 20);
        assert!(waiting.contains("enter runs the plan above"), "{waiting}");
    }

    /// A turn that is over, put straight into the thread.
    fn turn(app: &mut App, prompt: &str, steps: Vec<Step>, ended: Option<Finished>) {
        app.thread_mut().turns.push(Turn {
            prompt: prompt.to_string(),
            steps,
            finished: ended,
            failed: None,
        });
    }

    fn working(app: &mut App, prompt: &str, steps: Vec<Step>) {
        app.thread_mut().live = Some(Session::recorded(prompt, steps, None));
    }

    /// A workspace that is not on disk, for the screens that do not touch it.
    fn nowhere() -> Workspace {
        Workspace::at(std::path::Path::new("/p"))
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draws");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Everything drawn in one colour, read back off the rendered cells.
    fn painted(app: &mut App, width: u16, height: u16, colour: Color) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draws");
        let buffer = terminal.backend().buffer().clone();
        let mut found = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                if cell.fg == colour {
                    found.push_str(cell.symbol());
                }
            }
        }
        found
    }

    #[test]
    fn a_directory_that_is_not_a_project_says_what_would_be_written() {
        let dir = std::env::temp_dir().join(format!("ostraka-view-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("repositories/work")).expect("scratch");
        std::fs::write(dir.join("repositories/work/Cargo.toml"), "[package]\n").expect("write");

        let mut app = App::new(Workspace::at(&dir), Vec::new());
        app.setup = Some(crate::init::plan(&dir));

        let out = screen(&mut app, 100, 24);
        assert!(out.contains("not an Ostraka project yet"), "{out}");
        assert!(out.contains("a Rust project"), "{out}");
        assert!(out.contains(".ostraka/adapters/codex.toml"), "{out}");
        assert!(out.contains("i set this directory up"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_names_what_is_already_there_rather_than_offering_to_rewrite_it() {
        let dir = std::env::temp_dir().join(format!("ostraka-view-half-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ostraka")).expect("scratch");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "# mine\n").expect("write");

        let mut app = App::new(Workspace::at(&dir), Vec::new());
        app.setup = Some(crate::init::plan(&dir));
        let out = screen(&mut app, 100, 24);
        assert!(out.contains("already there"), "{out}");
        assert!(
            out.contains("Nothing already on disk is overwritten"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_up_says_what_setting_up_will_not_fix() {
        // Both of these used to be found out from a failed run: one in git's
        // own words two minutes in, and one from a gate check written to fail
        // on purpose that never said so anywhere anybody would look.
        let dir = std::env::temp_dir().join(format!("ostraka-view-warn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let mut app = App::new(Workspace::at(&dir), Vec::new());
        app.setup = Some(crate::init::plan(&dir));
        app.blocked = Some(
            "This directory is not a git repository, so a run has nowhere to work.".to_string(),
        );

        let out = screen(&mut app, 100, 30);
        assert!(out.contains("not a git repository"), "{out}");
        assert!(out.contains("x walks through it"), "{out}");
        assert!(out.contains("fails on purpose"), "{out}");
        assert!(out.contains("before the first task"), "{out}");

        // A workspace whose repository has a language it recognises, and a
        // repository git knows, has neither to say.
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        std::fs::write(dir.join("repositories/work/Cargo.toml"), "[package]\n").expect("write");
        let mut ok = App::new(Workspace::at(&dir), Vec::new());
        ok.setup = Some(crate::init::plan(&dir));
        let quiet = screen(&mut ok, 100, 30);
        assert!(!quiet.contains("not a git repository"), "{quiet}");
        assert!(!quiet.contains("fails on purpose"), "{quiet}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slash_in_the_box_offers_the_commands_at_the_box() {
        // Anchored where the typing is, because that is where the eye already
        // is. Typing "/settings" used to start a run whose task was the word.
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = "/".into();
        assert!(app.slashing());
        let out = screen(&mut app, 100, 24);
        assert!(out.contains("/settings"), "{out}");
        assert!(out.contains("/runs"), "{out}");

        app.pane_mut().prompt = "/set".into();
        let narrowed = screen(&mut app, 100, 24);
        assert!(narrowed.contains("/settings"), "{narrowed}");
        assert!(!narrowed.contains("/runs"), "{narrowed}");
        assert_eq!(app.slash_picked(), Some(Command::Settings));

        // A word that is a command's own name reaches it even where the name
        // is not its slug.
        app.pane_mut().prompt = "/help".into();
        assert_eq!(app.slash_picked(), Some(Command::Keys));

        // A task is a task however it starts. The shape being recognised is
        // one word behind a slash, not any line beginning with one.
        app.pane_mut().prompt = "/tmp/x is where it goes".into();
        assert!(!app.slashing());
        assert!(!screen(&mut app, 100, 24).contains("/settings"));
    }

    #[test]
    fn a_slash_that_names_nothing_says_so_rather_than_offering_everything() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = "/xyzzy".into();
        assert!(screen(&mut app, 100, 24).contains("no command by that name"));
        assert_eq!(app.slash_picked(), None);
    }

    #[test]
    fn the_settings_show_what_is_this_threads_and_what_is_the_projects() {
        let mut app = App::new(nowhere(), Vec::new());
        app.thread_mut().model = Some("a-model".into());
        app.thread_mut().adapter = Some("claude-code".into());
        app.project_facts = vec![
            ("gate".into(), "format, test".into()),
            ("agent stops".into(), "900s".into()),
        ];
        app.open(Dialog::Settings);

        let out = screen(&mut app, 100, 28);
        assert!(out.contains("This thread"), "{out}");
        assert!(out.contains("a-model"), "{out}");
        assert!(out.contains("claude-code"), "{out}");
        assert!(out.contains("This project"), "{out}");
        assert!(out.contains("format, test"), "{out}");
        assert!(out.contains("900s"), "{out}");
        // The project's half is shown rather than edited, and says why.
        assert!(out.contains("the repository agrees on"), "{out}");

        // Editing a row types into it in place.
        app.editing = Some("archon".into());
        assert!(screen(&mut app, 100, 28).contains("archon"));
    }

    #[test]
    fn the_fix_dialog_walks_through_the_steps_one_at_a_time() {
        // What this replaces is a sentence: "did not finish — git worktree add
        // failed: fatal: not a git repository". True, from two layers down,
        // arriving after a vendor had been paid, and a dead end.
        let dir = std::env::temp_dir().join(format!("ostraka-view-fix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let mut app = App::new(Workspace::at(&dir), Vec::new());
        app.remedy = crate::remedy::Remedy::diagnose(&dir);
        app.open(Dialog::Fix);

        let out = screen(&mut app, 100, 30);
        assert!(out.contains("not a git repository"), "{out}");
        assert!(out.contains("git init"), "{out}");
        assert!(out.contains("do this step"), "{out}");
        // The commit warns about what it will take, and only while it is the
        // step being decided about.
        assert!(!out.contains("commits everything"), "{out}");

        app.remedy.as_mut().expect("a remedy").at = 1;
        let second = screen(&mut app, 100, 30);
        assert!(second.contains("commits everything"), "{second}");
        assert!(second.contains("git commit"), "{second}");

        app.remedy.as_mut().expect("a remedy").at = 2;
        assert!(screen(&mut app, 100, 30).contains("Nothing is in the way now"));

        // And a step that failed keeps git's own words rather than a summary.
        let remedy = app.remedy.as_mut().expect("a remedy");
        remedy.at = 1;
        remedy.failed = true;
        remedy.said = Some("Author identity unknown".to_string());
        let broken = screen(&mut app, 100, 30);
        assert!(broken.contains("Author identity unknown"), "{broken}");
        assert!(!broken.contains("do this step"), "{broken}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_step_only_a_person_can_take_is_not_offered_as_one_to_press() {
        // Pressing `y` on a step with nothing to run would mark it done and
        // change nothing, which is worse than saying it is not ours to take.
        let dir = std::env::temp_dir().join(format!("ostraka-view-yours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let mut app = App::new(Workspace::at(&dir), Vec::new());
        app.remedy = Some(crate::remedy::Remedy::nothing_cloned(&dir));
        app.open(Dialog::Fix);

        let out = screen(&mut app, 100, 26);
        assert!(out.contains("yours to do"), "{out}");
        assert!(!out.contains("do this step"), "{out}");
        // And the sentence telling you what to do is whole rather than cut at
        // the frame, which is where the other half of the answer lives.
        assert!(out.contains("start one"), "{out}");
        // It also has to name keys that work. Single letters belong to the task
        // box on the work screen, so the bare `w` this used to offer typed a
        // letter into the box and opened nothing. Commands are behind the
        // leader, and the step says so.
        assert!(
            out.contains(&format!("{} w", label(Chord::Leader))),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_leaving_dialog_says_what_leaving_costs() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Leaving);
        let bare = screen(&mut app, 100, 20);
        assert!(bare.contains("Leave the browser?"), "{bare}");
        assert!(bare.contains("stay"), "{bare}");
        assert!(!bare.contains("A run is going"), "{bare}");

        app.pane_mut().prompt = "a task half written".into();
        working(&mut app, "something", vec![Step::Entered(Phase::Authoring)]);
        let costly = screen(&mut app, 100, 22);
        assert!(costly.contains("A run is going"), "{costly}");
        assert!(costly.contains("not written down anywhere"), "{costly}");
    }

    #[test]
    fn an_empty_thread_says_what_to_do_rather_than_showing_nothing() {
        let mut app = App::new(nowhere(), Vec::new());
        let out = screen(&mut app, 100, 18);
        assert!(out.contains("Ask anything about this repository"), "{out}");
        assert!(out.contains("reviewed by a different agent"), "{out}");
        // Nothing to summarise in a directory nothing has run in.
        assert!(!out.contains("Last asked here"), "{out}");
    }

    #[test]
    fn opening_a_project_with_a_history_summarises_it_rather_than_ignoring_it() {
        // "Nothing has been asked here yet" is true of the thread and false of
        // the directory, and in a repository with a history behind it that
        // reads as a browser that has lost it.
        let mut app = App::new(
            nowhere(),
            (0..8)
                .map(|i| {
                    summary(
                        &format!("t{i}-2026090{}T000300Z", i % 10),
                        &format!("task number {i}"),
                        Some(if i % 2 == 0 {
                            Outcome::Approved
                        } else {
                            Outcome::Rejected
                        }),
                    )
                })
                .collect(),
        );
        let out = screen(&mut app, 100, 26);
        assert!(out.contains("Last asked here"), "{out}");
        assert!(out.contains("task number 0"), "{out}");
        assert!(out.contains("approved"), "{out}");
        assert!(out.contains("rejected"), "{out}");
        // Enough to recognise where the work got to, and not a second list.
        assert!(
            !out.contains("task number 7"),
            "the whole history was listed:\n{out}"
        );
        assert!(out.contains("8 runs"), "{out}");
        assert!(out.contains("looks any of them up"), "{out}");
        // And what to do next is still said.
        assert!(out.contains("Ask anything about this repository"), "{out}");

        app.thread_mut().base_ref = "ostraka/t1-20260908T000100Z".into();
        assert!(screen(&mut app, 100, 26).contains("The next one starts from"));
    }

    #[test]
    fn the_regions_are_divided_by_rules_rather_than_by_space_alone() {
        // Four regions separated only by gaps read as one region with holes in
        // it, and the eye re-derives the boundaries every time it looks.
        let mut app = App::new(nowhere(), Vec::new());
        let out = screen(&mut app, 100, 18);
        let rules = out
            .lines()
            .filter(|line| line.trim_start().starts_with(theme::RULE))
            .count();
        assert!(
            rules >= 1,
            "nothing divides the breadcrumb from the work:\n{out}"
        );
        // And the box is a box, which is the other divider.
        assert!(out.contains('\u{256d}'), "{out}");
    }

    #[test]
    fn the_thread_shows_what_was_asked_and_what_each_agent_did() {
        let mut app = App::new(nowhere(), Vec::new());
        turn(
            &mut app,
            "add a wall-clock ceiling",
            vec![
                Step::Entered(Phase::Authoring),
                said(Phase::Authoring, "editing gate.rs"),
                Step::Entered(Phase::Gating),
                Step::Checked(check("format", 0, "")),
                Step::Entered(Phase::Reviewing),
                said(Phase::Reviewing, "the change does what was asked"),
            ],
            Some(finished(
                "t1-20260908T000100Z",
                Outcome::Approved,
                "approved \u{2014} nothing merged",
            )),
        );

        let out = screen(&mut app, 100, 24);
        for expected in [
            "add a wall-clock ceiling",
            "author",
            "editing gate.rs",
            "gate",
            "format",
            "review",
            "the change does what was asked",
            "approved",
        ] {
            assert!(out.contains(expected), "no {expected:?} on:\n{out}");
        }
    }

    #[test]
    fn a_turn_is_closed_by_a_rule_carrying_how_it_ended() {
        let mut app = App::new(nowhere(), Vec::new());
        turn(
            &mut app,
            "one",
            vec![Step::Entered(Phase::Authoring)],
            Some(finished(
                "t1-20260908T000100Z",
                Outcome::Rejected,
                "rejected",
            )),
        );
        let out = screen(&mut app, 100, 20);
        let closing = out
            .lines()
            .find(|line| line.contains("rejected") && line.contains(theme::RULE))
            .unwrap_or_default()
            .to_string();
        assert!(
            !closing.is_empty(),
            "the turn was not closed by a labelled rule:\n{out}"
        );
    }

    #[test]
    fn a_run_that_ran_out_of_tokens_says_so_where_the_outcome_goes() {
        // The vendor's own account of why it stopped is long, and the rule it
        // has to fit on is one line. Cut to fit rather than dropped: dropping
        // it left a bare rule exactly where the reason should have been.
        let mut app = App::new(nowhere(), Vec::new());
        turn(
            &mut app,
            "write a long thing",
            vec![
                Step::Entered(Phase::Authoring),
                Step::Said {
                    phase: Phase::Authoring,
                    event: Event::Error {
                        message: "author exited abnormally: Error: prompt is too long: \
                                  210000 tokens > 200000 maximum"
                            .into(),
                        raw: None,
                    },
                },
            ],
            Some(finished(
                "t1-20260908T000100Z",
                Outcome::Rejected,
                "rejected \u{2014} the author could not run (exit 1): Error: prompt is too \
                 long: 210000 tokens > 200000 maximum",
            )),
        );

        let out = screen(&mut app, 100, 22);
        // On the rule that closes the turn, cut but not lost.
        let closing = out
            .lines()
            .find(|line| line.contains(theme::RULE) && line.contains("the author could not run"))
            .unwrap_or_default();
        assert!(
            !closing.is_empty(),
            "the reason vanished from the rule:\n{out}"
        );
        // And in the transcript, where the vendor said it.
        assert!(out.contains("prompt is too long"), "{out}");
    }

    #[test]
    fn who_is_speaking_is_a_colour_rather_than_something_to_read() {
        // A transcript should be scannable for "what did the reviewer say"
        // without reading it.
        let mut app = App::new(nowhere(), Vec::new());
        turn(
            &mut app,
            "one",
            vec![
                Step::Entered(Phase::Authoring),
                Step::Entered(Phase::Gating),
                Step::Entered(Phase::Reviewing),
            ],
            None,
        );
        assert!(painted(&mut app, 100, 20, theme::ACCENT).contains("author"));
        assert!(painted(&mut app, 100, 20, theme::WARN).contains("gate"));
        assert!(painted(&mut app, 100, 20, theme::REVIEW).contains("review"));
    }

    #[test]
    fn the_transcript_marks_only_the_phase_that_is_still_going() {
        let mut app = App::new(nowhere(), Vec::new());
        working(
            &mut app,
            "do a thing",
            vec![
                Step::Entered(Phase::Authoring),
                Step::Entered(Phase::Gating),
            ],
        );
        let spinning = |app: &mut App| {
            screen(app, 100, 20)
                .lines()
                .filter(|line| theme::SPINNER.iter().any(|f| line.contains(f)))
                .count()
        };
        assert_eq!(spinning(&mut app), 1, "one phase is running, not two");

        let first = screen(&mut app, 100, 20);
        app.tick += 1;
        assert_ne!(first, screen(&mut app, 100, 20), "the mark did not move");
    }

    #[test]
    fn a_failing_check_shows_its_output_without_waiting_for_the_record() {
        let mut app = App::new(nowhere(), Vec::new());
        working(
            &mut app,
            "break the build",
            vec![
                Step::Entered(Phase::Gating),
                Step::Checked(check("format", 0, "")),
                Step::Checked(check("test", 101, "assertion failed: left == right")),
            ],
        );
        let out = screen(&mut app, 100, 22);
        assert!(out.contains("pass"), "{out}");
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("assertion failed"), "{out}");
    }

    #[test]
    fn a_run_that_never_started_is_not_reported_as_a_refusal() {
        // "Refused" is a verdict on a change. A run that could not be launched
        // never produced one.
        let mut app = App::new(nowhere(), Vec::new());
        app.thread_mut().turns.push(Turn {
            prompt: "do a thing".into(),
            steps: Vec::new(),
            finished: None,
            failed: Some("no adapter profile can run here".into()),
        });
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("did not finish"), "{out}");
        assert!(out.contains("no adapter profile"), "{out}");
        assert!(!out.contains("rejected"), "{out}");
    }

    #[test]
    fn a_live_transcript_follows_its_own_tail_until_somebody_scrolls_up() {
        let mut app = App::new(nowhere(), Vec::new());
        let mut steps = vec![Step::Entered(Phase::Authoring)];
        steps.extend((0..60).map(|i| said(Phase::Authoring, &format!("line {i}"))));
        working(&mut app, "a talkative agent", steps);

        let following = screen(&mut app, 100, 20);
        assert!(
            following.contains("line 59"),
            "the tail was not shown:\n{following}"
        );

        app.scroll_by(-30);
        assert!(!app.pane().follow, "scrolling up did not release the tail");
        let held = screen(&mut app, 100, 20);
        assert!(!held.contains("line 59"), "the view snapped back:\n{held}");
    }

    #[test]
    fn the_breadcrumb_says_what_the_next_run_will_stand_on() {
        let mut app = App::new(nowhere(), Vec::new());
        // Nothing to say while the chain is still standing on HEAD.
        assert!(!screen(&mut app, 100, 18).contains("ostraka/"));

        app.thread_mut().base_ref = "ostraka/t1-20260908T000100Z".into();
        let out = screen(&mut app, 100, 18);
        let breadcrumb = out.lines().next().unwrap_or_default();
        assert!(breadcrumb.contains(" on "), "{out}");
        assert!(breadcrumb.contains("ostraka/t1-"), "{out}");
    }

    #[test]
    fn the_box_says_where_typing_goes() {
        let mut app = App::new(nowhere(), Vec::new());
        // It opens with the keys, because the first thing anybody does here is
        // say what they want.
        let writing = screen(&mut app, 100, 18);
        assert!(
            writing.contains("ask anything about this repository"),
            "{writing}"
        );

        app.focus = Focus::Keys;
        let idle = screen(&mut app, 100, 18);
        assert!(idle.contains("write a task"), "{idle}");
        assert!(idle.contains(label(Chord::Commands)), "{idle}");
    }

    #[test]
    fn a_task_of_several_lines_makes_the_box_taller() {
        let mut app = App::new(nowhere(), Vec::new());
        let one = app.prompt_height();
        app.pane_mut().prompt = "first\nsecond\nthird".into();
        assert!(app.prompt_height() > one);

        let out = screen(&mut app, 100, 20);
        assert!(out.contains("first"), "{out}");
        assert!(out.contains("third"), "{out}");
    }

    #[test]
    fn the_runs_dialog_lists_what_was_run_and_narrows_as_it_is_typed_into() {
        let mut app = App::new(
            nowhere(),
            vec![
                summary("t1-20260907T000300Z", "add a test", Some(Outcome::Approved)),
                summary(
                    "t2-20260907T000100Z",
                    "rename a field",
                    Some(Outcome::Rejected),
                ),
            ],
        );
        // Something in the thread, so what is behind the dialog is a
        // transcript rather than the opening summary of the same runs.
        turn(
            &mut app,
            "working",
            vec![Step::Entered(Phase::Authoring)],
            None,
        );
        app.open(Dialog::Runs);
        let out = screen(&mut app, 110, 24);
        assert!(out.contains("add a test"), "{out}");
        assert!(out.contains("rename a field"), "{out}");
        assert!(out.contains("09-07 00:03"), "{out}");
        assert!(out.contains("enter opens it"), "{out}");

        app.filter = "rename".into();
        app.refilter();
        let narrowed = screen(&mut app, 110, 24);
        assert!(narrowed.contains("1 of 2 runs"), "{narrowed}");
        assert!(!narrowed.contains("add a test"), "{narrowed}");
    }

    #[test]
    fn a_filter_matching_nothing_says_so_instead_of_looking_broken() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "alpha",
                Some(Outcome::Approved),
            )],
        );
        app.open(Dialog::Runs);
        app.filter = "nothing matches this".into();
        app.refilter();
        let out = screen(&mut app, 110, 24);
        assert!(out.contains("Nothing matches that"), "{out}");
        assert!(out.contains("0 of 1 runs"), "{out}");
    }

    #[test]
    fn the_agents_dialog_says_who_is_ready_and_who_was_chosen() {
        let mut app = App::new(nowhere(), Vec::new());
        app.agents = vec![
            Agent {
                id: "claude-code".into(),
                ready: true,
                note: "2.1.263".into(),
                configured: true,
            },
            Agent {
                id: "codex".into(),
                ready: false,
                note: "codex not found on PATH".into(),
                configured: true,
            },
        ];
        app.open(Dialog::Agents);

        let out = screen(&mut app, 110, 24);
        assert!(out.contains("automatic"), "{out}");
        assert!(out.contains("claude-code"), "{out}");
        assert!(out.contains("not found on PATH"), "{out}");

        app.thread_mut().adapter = Some("claude-code".into());
        let chosen = screen(&mut app, 110, 24);
        assert!(chosen.contains("writes"), "{chosen}");
    }

    #[test]
    fn a_record_shows_the_failing_check_that_explains_it() {
        // The reason someone looks a run up. A passing check's output is
        // noise; a failing one's is the whole point.
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "break the build",
                Some(Outcome::Rejected),
            )],
        );
        app.screen = Screen::Record;
        let mut r = record(
            "t1-20260907T000300Z",
            "break the build",
            vec![
                CheckRecord {
                    stdout: "quiet success".into(),
                    ..check("format", 0, "")
                },
                check("test", 101, "assertion failed: left == right"),
            ],
        );
        r.approval = None;
        app.record = Some(r);

        let out = screen(&mut app, 100, 24);
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("assertion failed"), "{out}");
        assert!(
            !out.contains("quiet success"),
            "a passing check's output was shown:\n{out}"
        );
    }

    #[test]
    fn a_record_shows_the_reason_the_reviewer_gave() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "do a thing",
                Some(Outcome::Rejected),
            )],
        );
        app.screen = Screen::Record;
        let mut r = record("t1-20260907T000300Z", "do a thing", Vec::new());
        r.approval = Some(Approval {
            reviewer: ActorId::new("ephor"),
            verdict: Verdict::Reject {
                reason: "went outside the task".into(),
            },
        });
        app.record = Some(r);
        assert!(screen(&mut app, 100, 20).contains("went outside the task"));
    }

    #[test]
    fn the_record_tabs_name_every_pane_and_mark_the_one_showing() {
        let mut app = App::new(
            nowhere(),
            vec![summary("t1-20260907T000300Z", "a", Some(Outcome::Approved))],
        );
        app.screen = Screen::Record;
        let out = screen(&mut app, 100, 20);
        for pane in Detail::ALL {
            assert!(out.contains(pane.title()), "{pane:?} unnamed:\n{out}");
        }

        let marked = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("test terminal");
            terminal.draw(|frame| draw(frame, app)).expect("draws");
            let buffer = terminal.backend().buffer().clone();
            let mut found = String::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    let cell = &buffer[(x, y)];
                    if cell.fg == theme::ACCENT
                        && cell.modifier.contains(Modifier::UNDERLINED)
                        && cell.symbol() != " "
                    {
                        found.push_str(cell.symbol());
                    }
                }
            }
            found
        };
        assert_eq!(marked(&mut app), "checks");
        app.detail = app.detail.next();
        assert_eq!(marked(&mut app), "events");
        app.detail = app.detail.next();
        assert_eq!(marked(&mut app), "diff");
        assert_eq!(app.detail.next(), Detail::Checks);
    }

    #[test]
    fn the_diff_pane_shows_the_change_and_marks_its_sides() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "add a line",
                Some(Outcome::Approved),
            )],
        );
        app.screen = Screen::Record;
        app.detail = Detail::Diff;
        app.diff = Some(Some(
            "diff --git a/x b/x\n@@ -1 +1,2 @@\n context\n+added line\n-removed line\n".into(),
        ));
        let out = screen(&mut app, 100, 24);
        assert!(out.contains("+added line"), "{out}");
        assert!(out.contains("-removed line"), "{out}");
        assert!(out.contains("@@ -1 +1,2 @@"), "{out}");
    }

    #[test]
    fn a_run_with_no_commit_names_both_reasons_rather_than_showing_an_empty_pane() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "add a line",
                Some(Outcome::Approved),
            )],
        );
        app.screen = Screen::Record;
        app.detail = Detail::Diff;
        app.diff = Some(None);
        let out = screen(&mut app, 100, 22);
        assert!(out.contains("produced no commit"), "{out}");
        assert!(out.contains("merged away"), "{out}");
    }

    #[test]
    fn scrolling_cannot_run_off_the_end_of_the_content() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "look at a diff",
                Some(Outcome::Approved),
            )],
        );
        app.screen = Screen::Record;
        app.detail = Detail::Diff;
        app.diff = Some(Some((0..200).map(|i| format!("+line {i}\n")).collect()));

        app.scroll_by(10_000);
        let out = screen(&mut app, 100, 24);
        assert!(app.scroll < 250, "scroll ran away: {}", app.scroll);
        assert!(
            out.contains("line 199"),
            "the end was not reachable:\n{out}"
        );

        app.scroll_by(-10_000);
        assert_eq!(app.scroll, 0);
        assert!(screen(&mut app, 100, 24).contains("t1-20260907T000300Z"));
    }

    #[test]
    fn moving_stays_inside_the_list() {
        let mut app = App::new(
            nowhere(),
            vec![
                summary("t1-20260907T000300Z", "a", None),
                summary("t2-20260907T000100Z", "b", None),
            ],
        );
        app.move_by(-1);
        assert_eq!(app.selected, 0);
        app.move_by(5);
        assert_eq!(app.selected, 1);
        app.move_by(-5);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn moving_in_an_empty_list_does_nothing_rather_than_panicking() {
        let mut app = App::new(nowhere(), Vec::new());
        app.move_by(1);
        app.move_by(-1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_filter_that_hides_the_selection_moves_it_rather_than_dangling() {
        let mut app = App::new(
            nowhere(),
            vec![
                summary("t1-20260907T000300Z", "alpha", Some(Outcome::Approved)),
                summary("t2-20260907T000200Z", "beta", Some(Outcome::Approved)),
            ],
        );
        app.move_by(1);
        assert_eq!(app.current().map(|r| r.prompt.as_str()), Some("beta"));

        app.filter = "alpha".into();
        app.refilter();
        assert_eq!(app.current().map(|r| r.prompt.as_str()), Some("alpha"));
    }

    #[test]
    fn a_filter_matches_the_outcome_word_too() {
        let mut app = App::new(
            nowhere(),
            vec![
                summary("t1-20260907T000300Z", "one", Some(Outcome::Approved)),
                summary("t2-20260907T000200Z", "two", Some(Outcome::Rejected)),
            ],
        );
        app.filter = "rejected".into();
        app.refilter();
        assert_eq!(app.matching.len(), 1);
        assert_eq!(app.current().map(|r| r.prompt.as_str()), Some("two"));

        // The word this screen used before 1.1.0 still finds the same run.
        app.filter = "refused".into();
        app.refilter();
        assert_eq!(
            app.matching.len(),
            1,
            "the old word stopped finding anything"
        );
        assert_eq!(app.current().map(|r| r.prompt.as_str()), Some("two"));
    }

    /// A usage row shaped the way a vendor reports it: a split, or — when
    /// `output` is `None` — one combined total.
    fn with_usage(
        mut run: RunSummary,
        rows: &[(&str, &str, u64, Option<u64>, bool)],
    ) -> RunSummary {
        run.usage = rows
            .iter()
            .map(
                |(adapter, role, first, output, approximate)| ostraka_core::record::TokenUsage {
                    adapter: (*adapter).to_string(),
                    role: (*role).to_string(),
                    input: output.map(|_| *first),
                    output: *output,
                    total: output.map_or(Some(*first), |_| None),
                    approximate: *approximate,
                },
            )
            .collect();
        run
    }

    #[test]
    fn the_status_line_totals_each_backend_separately() {
        let mut app = App::new(
            nowhere(),
            vec![
                with_usage(
                    summary("t1-20260907T000300Z", "one", Some(Outcome::Approved)),
                    &[
                        ("claude-code", "author", 10_136, Some(1_258), false),
                        ("codex", "reviewer", 3_303, None, false),
                    ],
                ),
                with_usage(
                    summary("t2-20260907T000200Z", "two", Some(Outcome::Approved)),
                    &[("claude-code", "author", 20_000, Some(742), false)],
                ),
            ],
        );
        let out = screen(&mut app, 120, 18);
        assert!(out.contains("tokens"), "{out}");
        assert!(out.contains("claude-code 30.1k in / 2.0k out"), "{out}");
        assert!(out.contains("codex 3.3k total"), "{out}");
    }

    #[test]
    fn a_rounded_backend_is_marked_as_an_estimate() {
        // One vendor rounds before it reports. A total including it is an
        // estimate, and a status line that implied otherwise would be lying in
        // the one place this project cannot afford to.
        let mut app = App::new(
            nowhere(),
            vec![with_usage(
                summary("t1-20260907T000300Z", "one", Some(Outcome::Approved)),
                &[("copilot-cli", "reviewer", 10_600, Some(296), true)],
            )],
        );
        assert!(screen(&mut app, 120, 16).contains("copilot-cli ~10.6k in / ~296 out"));
    }

    #[test]
    fn a_backend_that_reports_nothing_is_absent_rather_than_zero() {
        // "Does not say" and "spent nothing" are different claims.
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "one",
                Some(Outcome::Approved),
            )],
        );
        let out = screen(&mut app, 120, 16);
        assert!(out.contains("no backend on these runs reported"), "{out}");
        assert!(!out.contains(" 0 in "), "{out}");
    }

    #[test]
    fn the_status_line_says_what_is_happening_while_a_run_is_going() {
        let mut app = App::new(nowhere(), Vec::new());
        working(&mut app, "do a thing", vec![Step::Entered(Phase::Gating)]);
        let out = screen(&mut app, 110, 18);
        assert!(out.contains("running"), "{out}");
        assert!(out.contains("gate"), "{out}");
        assert!(out.contains("41s"), "{out}");

        app.thread_mut().live.as_mut().expect("a session").stopping = true;
        assert!(screen(&mut app, 110, 18).contains("stopping"));
    }

    #[test]
    fn a_status_message_takes_the_line_from_the_totals_that_do_not_change() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "one",
                Some(Outcome::Approved),
            )],
        );
        app.status = Some("promoted to promoted/t1 \u{2014} nothing merged".into());
        let out = screen(&mut app, 110, 16);
        assert!(out.contains("nothing merged"), "{out}");
        assert!(
            !out.contains("tokens"),
            "the totals outlived the message:\n{out}"
        );
    }

    #[test]
    fn panes_sit_side_by_side_on_a_wide_terminal_and_take_turns_on_a_narrow_one() {
        let mut app = App::new(nowhere(), Vec::new());
        // Longer than the bar shows, so only a column can show all of it.
        turn(&mut app, "the task written in the first pane", vec![], None);
        app.open_pane();
        turn(
            &mut app,
            "the task written in the second pane",
            vec![],
            None,
        );

        let wide = screen(&mut app, 200, 30);
        assert!(
            wide.contains("the task written in the first pane"),
            "{wide}"
        );
        assert!(
            wide.contains("the task written in the second pane"),
            "{wide}"
        );
        assert_eq!(app.columns.len(), 2);
        assert!(app.situation().split);

        let narrow = screen(&mut app, 120, 30);
        assert!(
            !narrow.contains("the task written in the first pane"),
            "{narrow}"
        );
        assert!(
            narrow.contains("the task written in the second pane"),
            "{narrow}"
        );
        assert!(app.columns.is_empty());
        assert!(!app.situation().split);
    }

    #[test]
    fn the_screen_fits_a_wide_terminal_with_panes_side_by_side() {
        let mut app = App::new(nowhere(), Vec::new());
        for _ in 0..4 {
            app.open_pane();
        }
        for (width, height) in [(160, 24), (200, 30), (320, 50)] {
            let out = screen(&mut app, width, height);
            for line in out.lines() {
                assert!(line.chars().count() <= width as usize, "{line}");
            }
            for (_, column) in &app.columns {
                assert!(column.right() <= width, "a column ran off the screen");
            }
        }
    }

    #[test]
    fn a_wider_pane_takes_width_from_the_rest_and_none_goes_below_the_minimum() {
        assert_eq!(column_widths(200, &[3, 3]), vec![100, 100]);
        let lopsided = column_widths(200, &[pane::MAX_WEIGHT, 1]);
        assert!(lopsided[0] > lopsided[1]);
        assert!(lopsided[1] >= COLUMN_MIN, "{lopsided:?}");
        assert_eq!(lopsided.iter().sum::<u16>(), 200);
        assert_eq!(column_widths(241, &[3, 3, 3]).iter().sum::<u16>(), 241);
    }

    /// With the agents beside it, the work splits into columns only where the
    /// width left to it would have split on its own, and every column stays
    /// inside the work: none reaches under the agents.
    #[test]
    fn columns_count_the_width_the_agents_leave() {
        let taken = super::super::roster::WIDTH + SIDE_GAP;
        let mut app = with_agents();
        app.open_pane();
        screen(&mut app, SPLIT_WIDTH + taken, 30);
        assert!(app.side_shown);
        assert_eq!(app.columns.len(), 2, "{:?}", app.columns);
        let edge = SPLIT_WIDTH + taken - super::super::roster::WIDTH - SIDE_GAP;
        for (_, area) in &app.columns {
            assert!(
                area.x + area.width <= edge,
                "{area:?} runs under the agents"
            );
        }

        screen(&mut app, SPLIT_WIDTH + taken - 1, 30);
        assert!(app.side_shown);
        assert!(app.columns.is_empty(), "split one short of the threshold");
    }

    #[test]
    fn more_panes_than_columns_keep_the_one_in_front_on_screen() {
        let mut app = App::new(nowhere(), Vec::new());
        for _ in 0..4 {
            app.open_pane();
        }
        screen(&mut app, 200, 30);
        assert_eq!(app.columns.len(), 2);
        assert!(
            app.columns.iter().any(|(i, _)| *i == 4),
            "{:?}",
            app.columns
        );

        app.focus_pane(0);
        screen(&mut app, 200, 30);
        assert!(
            app.columns.iter().any(|(i, _)| *i == 0),
            "{:?}",
            app.columns
        );
    }

    #[test]
    fn reading_a_record_does_not_move_a_pane_it_was_not_about() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().scroll = 7;
        app.screen = Screen::Record;
        app.scroll = 40;
        app.open_pane();
        assert_eq!(
            app.panes[0].scroll, 7,
            "a record's scroll was parked in a pane"
        );
        // Opening a pane starts its screen at the top; the record is read again.
        app.scroll = 40;
        app.focus_pane(0);
        assert_eq!(app.scroll, 40, "a pane's scroll was put over the record");
    }

    #[test]
    fn moving_a_pane_swaps_it_with_its_neighbour_and_the_keys_go_with_it() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = "left".to_string();
        app.open_pane();
        app.pane_mut().prompt = "right".to_string();

        assert!(app.move_pane(-1));
        assert_eq!(app.at, 0);
        assert_eq!(app.pane().prompt, "right");
        assert_eq!(app.panes[1].prompt, "left");
        assert!(!app.move_pane(-1), "moved past the edge");
    }

    #[test]
    fn a_column_beside_the_one_in_front_keeps_its_place() {
        let mut app = App::new(nowhere(), Vec::new());
        let steps: Vec<Step> = (0..60).map(|_| Step::Entered(Phase::Authoring)).collect();
        turn(&mut app, "a long transcript", steps, None);
        screen(&mut app, 200, 30);
        app.scroll_by(-10);
        screen(&mut app, 200, 30);
        let place = app.scroll;

        app.open_pane();
        screen(&mut app, 200, 30);
        assert_eq!(app.panes[0].scroll, place, "the column jumped");
        assert!(!app.panes[0].follow);
    }

    #[test]
    fn a_cursor_inside_the_task_does_not_push_the_rest_of_it_aside() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = "hello world".to_string();
        let end = screen(&mut app, 100, 20);
        assert!(end.contains(&format!("world{}", theme::CURSOR)), "{end}");

        app.pane_mut().left();
        let inside = screen(&mut app, 100, 20);
        assert!(inside.contains("hello world"), "{inside}");
        assert!(
            !inside.contains(&format!("world{}", theme::CURSOR)),
            "the cursor stayed at the end: {inside}"
        );
    }

    #[test]
    fn the_box_scrolls_to_the_row_being_written() {
        let mut app = App::new(nowhere(), Vec::new());
        let rows: Vec<String> = (1..=10).map(|i| format!("row {i}")).collect();
        app.pane_mut().prompt = rows.join("\n");
        let out = screen(&mut app, 100, 30);
        assert!(
            out.contains("row 10"),
            "the row being written is hidden: {out}"
        );
    }

    #[test]
    fn a_pending_leader_says_what_it_is_waiting_for() {
        let mut app = App::new(nowhere(), Vec::new());
        app.leader = true;
        let out = screen(&mut app, 120, 16);
        assert!(out.contains(label(Chord::Leader)), "{out}");
        assert!(out.contains("write a task"), "{out}");
    }

    #[test]
    fn the_keys_dialog_lists_the_keys_rather_than_a_footer_doing_it_forever() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Keys);
        let out = screen(&mut app, 110, 26);
        assert!(out.contains(label(Chord::Leader)), "{out}");
        assert!(out.contains("alt-enter"), "{out}");
        assert!(out.contains("esc closes this"), "{out}");
    }

    /// Every command reaches the reference, because the reference is made from
    /// the same list the palette and the leader read.
    ///
    /// It was written out beside that list instead, and a command added to one
    /// did not reach the other — so the screen somebody opens when they cannot
    /// remember a key was the screen that did not know the key existed.
    #[test]
    fn the_keys_dialog_lists_every_command_there_is() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Keys);
        // Tall enough that nothing is below the fold, so this is about the
        // list and not about scrolling.
        let out = screen(&mut app, 110, 90);
        for command in Command::ALL {
            // The chord, not the description: it is short, it is unique to the
            // command, and it is the thing somebody came here to find. A long
            // description is clipped to the width of the dialog, which would
            // make this a test about line length.
            let chord = format!("{} {}", label(Chord::Leader), command.leader());
            assert!(
                out.contains(&chord),
                "{command:?} is not in the keys dialog:\n{out}"
            );
        }
    }

    #[test]
    fn the_command_palette_names_what_it_can_do_and_marks_the_pick() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Commands);
        let out = screen(&mut app, 110, 26);
        assert!(out.contains("promote run"), "{out}");
        assert!(out.contains("merges nothing"), "{out}");

        app.query = "promote".into();
        let narrowed = screen(&mut app, 110, 26);
        assert!(narrowed.contains("promote run"), "{narrowed}");
        assert!(!narrowed.contains("reload runs"), "{narrowed}");
    }

    #[test]
    fn a_palette_query_matching_nothing_says_so_rather_than_showing_an_empty_box() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Commands);
        app.query = "xyzzy".into();
        assert!(screen(&mut app, 110, 26).contains("no command matches that"));
        assert_eq!(app.picked(), None);
    }

    #[test]
    fn counts_are_compact_without_becoming_vague_below_a_thousand() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(10_136), "10.1k");
        assert_eq!(compact(2_500_000), "2.5M");
    }

    #[test]
    fn a_run_id_that_is_not_shaped_like_one_yields_no_time_rather_than_a_panic() {
        assert_eq!(when("t1-20260907T000300Z"), "09-07 00:03");
        assert_eq!(when("nonsense"), "");
        assert_eq!(when("a-b"), "");
        assert_eq!(when(""), "");
    }

    #[test]
    fn wrapping_breaks_on_spaces_and_never_loses_a_word() {
        let parts = wrap("one two three four five", 9);
        assert!(parts.iter().all(|p| p.chars().count() <= 9), "{parts:?}");
        assert_eq!(parts.join(" "), "one two three four five");
        assert_eq!(wrap("supercalifragilistic", 5), ["supercalifragilistic"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("anything", 0), ["anything"]);
    }

    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        assert_eq!(truncate("ééééé", 3), "éé…");
        assert_eq!(truncate("short", 40), "short");
        assert_eq!(truncate("anything", 0), "anything");
    }

    #[test]
    fn the_work_screen_says_what_is_in_the_way_of_a_run() {
        // It was said on the setup screen, and that screen goes the moment
        // setting up is done — leaving a workspace that is configured, has
        // nowhere to run anything, and says so only if you press enter.
        let mut app = App::new(nowhere(), Vec::new());
        assert!(!screen(&mut app, 100, 20).contains("walks through it"));

        app.blocked = Some("Nothing has been cloned into repositories/ yet.".into());
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("Nothing has been cloned"), "{out}");
        assert!(out.contains("walks through it"), "{out}");
        // And what to do next is still said under it.
        assert!(out.contains("Ask anything about this repository"), "{out}");
    }

    #[test]
    fn the_repositories_dialog_takes_a_name_for_a_new_one() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Repos);
        let out = screen(&mut app, 100, 22);
        assert!(out.contains("Nothing has been cloned"), "{out}");
        assert!(out.contains("start one here"), "{out}");

        app.editing = Some("fresh".into());
        let naming = screen(&mut app, 100, 22);
        assert!(naming.contains("fresh"), "{naming}");
        assert!(naming.contains("enter starts it"), "{naming}");
        assert!(!naming.contains("start one here"), "{naming}");
    }

    #[test]
    fn the_pane_bar_appears_only_when_there_is_something_to_navigate() {
        let mut app = App::new(nowhere(), Vec::new());
        // One line of work needs no bar saying which one it is.
        assert!(!screen(&mut app, 100, 20).contains(&format!("{} new", label(Chord::NewPane))));

        app.open_pane();
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("1 new"), "{out}");
        assert!(out.contains("2 new"), "{out}");
        assert!(
            out.contains(&format!("{} new", label(Chord::NewPane))),
            "{out}"
        );
    }

    #[test]
    fn a_pane_is_named_for_where_it_works_and_marked_while_it_runs() {
        // A run in a pane nobody is looking at is still a run, and a bar that
        // did not say so would be a bar that hid it.
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().repository = Some(Repository {
            name: "scratch".into(),
            path: PathBuf::from("/p/repositories/scratch"),
        });
        working(&mut app, "something", vec![Step::Entered(Phase::Authoring)]);
        app.open_pane();

        let out = screen(&mut app, 100, 20);
        let bar = out.lines().nth(1).unwrap_or_default();
        assert!(bar.contains("scratch"), "{out}");
        // The new pane carried the repository over, and the running one is
        // marked even though the screen is on the other.
        assert!(
            bar.contains(theme::CURSOR) || bar.contains('\u{b7}'),
            "{bar}"
        );
        assert_eq!(app.at, 1);
        assert!(app.anything_running(), "the run was lost by switching pane");
    }

    #[test]
    fn the_footer_grows_into_the_room_it_has_and_no_further() {
        // The commands are the row worth adding first — knowing what can be
        // done is worth more than knowing what it will be done with — and
        // neither is worth a row on a terminal that needs all of them for the
        // work.
        let mut app = App::new(nowhere(), Vec::new());

        let short = screen(&mut app, 92, 20);
        assert!(
            !short.contains(&format!("{}  n", label(Chord::Leader))),
            "a short terminal lost a row to the commands:\n{short}"
        );
        assert!(!short.contains("writes "), "{short}");

        let medium = screen(&mut app, 92, 30);
        assert!(
            medium.contains(&format!("{}  n", label(Chord::Leader))),
            "{medium}"
        );
        assert!(
            !medium.contains("writes "),
            "the routing row arrived before there was room:\n{medium}"
        );

        let tall = screen(&mut app, 92, 40);
        assert!(
            tall.contains(&format!("{}  n", label(Chord::Leader))),
            "{tall}"
        );
        assert!(tall.contains("writes automatic"), "{tall}");

        // And the status line survives all three, because it is the one that
        // says whether anything is running.
        for out in [&short, &medium, &tall] {
            assert!(out.contains("ostraka "), "the status line went:\n{out}");
        }
    }

    /// The bottom row of a drawn screen.
    fn bottom(out: &str) -> String {
        out.lines().last().unwrap_or_default().to_string()
    }

    fn two_runs() -> App {
        App::new(
            nowhere(),
            vec![
                summary("t2-20260907T000300Z", "rename", Some(Outcome::Rejected)),
                summary("t1-20260907T000100Z", "add a test", Some(Outcome::Approved)),
            ],
        )
    }

    /// Each suggestion on the bottom row, with the chord that takes it.
    #[test]
    fn the_footer_suggests_the_next_action_with_its_keys() {
        let leader = label(Chord::Leader);

        // The selected run is the approved one, not yet promoted.
        let mut app = two_runs();
        app.unpromoted.insert("t1-20260907T000100Z".into());
        app.selected = 1;
        let row = bottom(&screen(&mut app, 120, 24));
        assert!(
            row.contains(&format!("next  {leader} p promote the selected run")),
            "{row}"
        );

        // Selected elsewhere: `p` would promote the wrong run.
        app.selected = 0;
        let row = bottom(&screen(&mut app, 120, 24));
        assert!(
            row.contains(&format!("{leader} l 1 approved run to promote")),
            "{row}"
        );

        // The newest run was refused by a check.
        let mut app = two_runs();
        app.last_record = Some(record(
            "t2-20260907T000300Z",
            "rename",
            vec![check("test", 1, "")],
        ));
        let row = bottom(&screen(&mut app, 120, 24));
        assert!(row.contains(&format!("{leader} m loop mode")), "{row}");

        // Something in the way comes first.
        app.blocked = Some("not a git repository".into());
        let row = bottom(&screen(&mut app, 120, 24));
        assert!(
            row.contains(&format!("{leader} x fix what is in the way")),
            "{row}"
        );

        // Waiting tasks and nothing taking them.
        let mut app = App::new(nowhere(), Vec::new());
        app.task_groups = vec![super::super::roster::TaskGroup {
            repository: "only".into(),
            waiting: 2,
            going: 0,
            done: 0,
            rows: Vec::new(),
        }];
        let row = bottom(&screen(&mut app, 120, 24));
        assert!(
            row.contains("next  ostraka drain takes the 2 waiting tasks"),
            "{row}"
        );

        // And nothing worth doing says nothing.
        let mut app = App::new(nowhere(), Vec::new());
        assert!(!bottom(&screen(&mut app, 120, 24)).contains("next"));
    }

    /// With three rows the facts get the middle one: the counts, the last run
    /// in a phrase, and which check refused it.
    #[test]
    fn the_facts_row_counts_the_runs_and_says_how_the_last_one_ended() {
        let mut app = two_runs();
        app.last_record = Some(record(
            "t2-20260907T000300Z",
            "rename",
            vec![check("clippy", 1, "")],
        ));
        let out = screen(&mut app, 120, 40);
        let facts = out
            .lines()
            .find(|l| l.contains("1 approved"))
            .unwrap_or_else(|| panic!("no facts row:\n{out}"));
        assert!(facts.contains("1 approved, 1 rejected"), "{facts}");
        assert!(
            facts.contains("last run refused by the clippy check"),
            "{facts}"
        );
    }

    /// Narrow: nothing wraps or runs past the edge, what is dropped is dropped
    /// from the end, and the status message outlives everything else.
    #[test]
    fn a_narrow_footer_keeps_its_priorities_and_its_edge() {
        use unicode_width::UnicodeWidthStr;
        let mut app = two_runs();
        app.unpromoted.insert("t1-20260907T000100Z".into());
        app.selected = 1;
        for width in [30u16, 50, 70, 120] {
            let out = screen(&mut app, width, 24);
            for line in out.lines() {
                assert!(line.width() <= width as usize, "{width}: {line:?}");
            }
            let row = bottom(&out);
            // The suggestion goes before the counts do: a row with the counts
            // on it has the suggestion too.
            if row.contains("approved,") {
                assert!(row.contains("promote"), "{width}: {row}");
            }
        }
        // At fifty the suggestion is there and the version is what went.
        let row = bottom(&screen(&mut app, 50, 24));
        assert!(row.contains("promote the selected run"), "{row}");
        assert!(!row.contains("ostraka 1."), "{row}");
        let row = bottom(&screen(&mut app, 70, 24));
        assert!(!row.contains("tokens"), "tokens outlived the counts: {row}");

        app.status = Some("promoted to promoted/t1 \u{2014} nothing merged".into());
        let row = bottom(&screen(&mut app, 30, 24));
        assert!(
            row.contains("promoted to"),
            "the status message went: {row}"
        );
    }

    #[test]
    fn the_commands_row_names_the_chord_it_needs() {
        // Bare letters would be the defect this browser has already fixed once:
        // on the work screen the box has the keys, so `n` types an `n`. A row
        // of single letters is an invitation to find that out.
        let mut app = App::new(nowhere(), Vec::new());
        let out = screen(&mut app, 92, 30);
        let row = out
            .lines()
            .find(|l| l.contains(" write a task"))
            .expect("the commands row");
        assert!(
            row.contains(label(Chord::Leader)),
            "the chord is not named: {row}"
        );
    }

    #[test]
    fn a_narrow_footer_drops_commands_rather_than_running_past_the_edge() {
        let mut app = App::new(nowhere(), Vec::new());
        for width in [40u16, 60, 92] {
            let out = screen(&mut app, width, 30);
            for line in out.lines() {
                assert!(
                    line.chars().count() <= width as usize,
                    "a line ran past {width} columns:\n{out}"
                );
            }
        }
    }

    #[test]
    fn the_screen_fits_whatever_terminal_it_is_given() {
        let mut app = App::new(
            nowhere(),
            vec![summary(
                "t1-20260907T000300Z",
                "a task with a reasonably long description on it",
                Some(Outcome::Approved),
            )],
        );
        for (width, height) in [(46, 12), (60, 16), (80, 24), (120, 40)] {
            let out = screen(&mut app, width, height);
            for line in out.lines() {
                assert!(
                    line.chars().count() <= width as usize,
                    "a line ran past {width} columns:\n{out}"
                );
            }
            assert_eq!(out.lines().count(), height as usize, "{out}");
            // The one sentence somebody needs on first opening is on every
            // one of them, whole rather than cut in half by the frame.
            assert!(
                out.contains("Ask anything about this repository"),
                "at {width}x{height}:\n{out}"
            );
        }
    }

    #[test]
    fn a_dialog_taller_than_the_terminal_keeps_the_way_out_of_it() {
        // A dialog whose footer has been cut off the bottom is one somebody is
        // stuck in.
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Keys);
        let out = screen(&mut app, 70, 14);
        assert!(
            out.contains("esc closes this"),
            "the way out was cut:\n{out}"
        );
        assert!(
            out.contains("more line"),
            "nothing said what was hidden:\n{out}"
        );

        // And where it fits, nothing is hidden and nothing says it is. It
        // takes a tall terminal now: the commands half is every command, so
        // the reference is longer than it was when it was written by hand.
        let roomy = screen(&mut app, 70, 80);
        assert!(roomy.contains("esc closes this"), "{roomy}");
        assert!(!roomy.contains("more line"), "{roomy}");
    }

    #[test]
    fn the_box_does_not_eat_a_short_terminal() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = (0..6).map(|i| format!("line {i}\n")).collect();
        let tall = screen(&mut app, 80, 40);
        assert!(tall.contains("line 5"), "{tall}");

        // On twelve rows the box gives way to the work it is about. Counted
        // rather than asked about one row: the box scrolls to the row being
        // written, so which rows show is the cursor's business, and how many is
        // the screen's.
        let short = screen(&mut app, 80, 12);
        assert!(
            short.matches("line ").count() <= 2,
            "the box took the screen:\n{short}"
        );
        assert!(
            short.contains("Ask anything about this repository"),
            "{short}"
        );
    }

    #[test]
    fn a_terminal_too_short_for_the_chrome_says_so_rather_than_drawing_nothing() {
        let mut app = App::new(nowhere(), Vec::new());
        assert!(screen(&mut app, 60, 4).contains("too short"));
    }
}
