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

use crate::init::{Action, Plan};
use crate::tui::command::{Command, Situation};
use crate::tui::pane::Pane;
use crate::tui::remedy::Remedy;
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
}

impl App {
    pub fn new(workspace: Workspace, runs: Vec<RunSummary>) -> Self {
        let matching = (0..runs.len()).collect();
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

    /// Any pane at all. One run happens at a time across the whole browser,
    /// because a stop is one flag and two runs would both answer it.
    pub fn anything_running(&self) -> bool {
        self.panes.iter().any(Pane::running)
    }

    /// Opens another line of work, in the same repository as this one.
    ///
    /// The repository is carried over because a second pane is usually a
    /// second thing to do in the same place; `w` moves it somewhere else.
    pub fn open_pane(&mut self) {
        let repository = self.pane().repository.clone();
        self.panes.push(Pane::new(repository));
        self.at = self.panes.len() - 1;
        self.scroll = 0;
    }

    /// Moves to the next pane, wrapping.
    pub fn next_pane(&mut self, delta: isize) {
        if self.panes.len() < 2 {
            return;
        }
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
            running: self.anything_running(),
            blocked: self.blocked.is_some(),
            panes: self.panes.len(),
        }
    }

    /// The commands the palette is currently offering.
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
                    self.pane_mut().prompt.clear();
                    return;
                }
                None => 0,
            },
        };
        self.pane_mut().history_at = Some(at);
        let said = history[at].clone();
        self.pane_mut().prompt = said;
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

    /// How many rows the input box wants, border included.
    fn prompt_height(&self) -> u16 {
        let lines = self.pane().prompt.lines().count().clamp(1, PROMPT_LINES);
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
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(bar),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(box_height),
            Constraint::Length(1),
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
    if app.setup.is_some() {
        render_setup(frame, app, content);
    } else {
        match app.screen {
            Screen::Work => render_work(frame, app, content),
            Screen::Record => render_record(frame, app, content),
        }
    }

    render_prompt(frame, app, rows[4]);
    if app.slashing() {
        render_slash(frame, app, rows[4]);
    }
    frame.render_widget(
        Paragraph::new(status_bar(app, rows[5].width.saturating_sub(theme::GUTTER))),
        theme::inset(rows[5]),
    );

    match app.dialog {
        Some(Dialog::Keys) => render_keys(frame, screen),
        Some(Dialog::Commands) => render_palette(frame, app, screen),
        Some(Dialog::Runs) => render_runs(frame, app, screen),
        Some(Dialog::Agents) => render_agents(frame, app, screen),
        Some(Dialog::Leaving) => render_leaving(frame, app, screen),
        Some(Dialog::Fix) => render_fix(frame, app, screen),
        Some(Dialog::Settings) => render_settings(frame, app, screen),
        Some(Dialog::Repos) => render_repos(frame, app, screen),
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
        spans.push(Span::styled("    ctrl-t new", theme::muted()));
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

/// The thread: everything asked here, and what came of it.
fn render_work(frame: &mut Frame, app: &mut App, area: Rect) {
    app.page = area.height.saturating_sub(1).max(1);
    if app.thread().is_empty() {
        frame.render_widget(Paragraph::new(opening(app, area.width, area.height)), area);
        return;
    }

    let lines = thread_lines(app.thread(), area.width, app.tick);
    let overflow = lines.len().saturating_sub(area.height as usize) as u16;
    if app.pane().follow {
        app.scroll = overflow;
    }
    app.scroll = app.scroll.min(overflow);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
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

    lines.push(Line::from(vec![
        Span::styled("Write a task below and press ", theme::muted()),
        Span::styled("enter", theme::accent()),
        Span::styled(".", theme::muted()),
    ]));
    for part in wrap(
        "Each one is isolated, gated and reviewed by a different agent than the \
         one that wrote it.",
        width as usize,
    ) {
        lines.push(dim(part));
    }
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
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", theme::CONTINUE),
                        theme::on(phase_colour(phase)),
                    ),
                    Span::styled(format!("{kind} "), theme::muted()),
                    Span::styled(
                        truncate(&text, width.saturating_sub(8) as usize),
                        theme::on(colour),
                    ),
                ]));
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
        Event::Message { text, .. } => ("said", text.replace('\n', " "), theme::TEXT),
        Event::ToolUse { name, .. } => ("tool", name.clone(), theme::ACCENT),
        Event::Error { message, .. } => ("err ", message.replace('\n', " "), theme::BAD),
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
        Detail::Events => lines.extend(event_lines(app)),
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

/// The line that says where typing goes.
fn render_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let inner = Rect {
        x: area.x.saturating_add(theme::GUTTER),
        width: area.width.saturating_sub(theme::GUTTER * 2),
        ..area
    };
    let writing = app.focus == Focus::Prompt && app.dialog.is_none();
    let block = theme::panel(writing).padding(Padding::horizontal(1));

    let text: Vec<Line<'static>> = if !writing && app.pane().prompt.is_empty() {
        vec![Line::from(Span::styled(
            "n  write a task     l  runs     ctrl-k  commands     ?  keys",
            theme::muted(),
        ))]
    } else {
        let mut lines: Vec<Line<'static>> = app
            .pane()
            .prompt
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), theme::text())))
            .collect();
        if lines.is_empty() {
            lines.push(Line::from(""));
        }
        if writing {
            // The cursor goes on the last line, which is where typing lands.
            let last = lines.len() - 1;
            let mut spans = lines[last].spans.clone();
            spans.push(Span::styled(theme::CURSOR, theme::accent()));
            if app.pane().prompt.is_empty() {
                spans.push(Span::styled(
                    "  say what the agent should do \u{b7} enter runs it",
                    theme::muted(),
                ));
            }
            lines[last] = Line::from(spans);
        }
        lines
    };

    let mark = if writing {
        Span::styled("\u{203a}", theme::accent().add_modifier(Modifier::BOLD))
    } else {
        Span::styled("\u{203a}", theme::muted())
    };
    frame.render_widget(
        Paragraph::new(text).block(block.title(Line::from(vec![
            Span::raw(" "),
            mark,
            Span::raw(" "),
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

/// The repositories this workspace works on.
fn render_repos(frame: &mut Frame, app: &App, screen: Rect) {
    let width = 78u16.min(screen.width);
    let repositories = app.workspace.repositories();
    let mut lines = vec![
        Line::from(Span::styled("Repositories", theme::bold())),
        theme::rule(width.saturating_sub(6)),
    ];
    if repositories.is_empty() {
        lines.push(dim(
            "Nothing has been cloned into repositories/ yet.".to_string()
        ));
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

/// The bottom line: what is happening, and what it has cost.
fn status_bar(app: &App, width: u16) -> Line<'static> {
    if let Some(status) = &app.status {
        return Line::from(Span::styled(status.clone(), theme::on(theme::WARN)));
    }
    if app.leader {
        let mut spans = vec![Span::styled("ctrl-x  ", theme::on(theme::WARN))];
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

    let left = match app.thread().live.as_ref().filter(|s| s.live()) {
        Some(session) => vec![
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
        ],
        None => vec![
            Span::styled("ostraka", theme::bold()),
            Span::styled(format!(" {}", env!("CARGO_PKG_VERSION")), theme::muted()),
            Span::styled(format!("  \u{b7}  {}", counted(app)), theme::muted()),
        ],
    };
    let right = tokens(app);

    let left_width: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let right_width: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let width = width as usize;

    let mut spans = left;
    if left_width + right_width + 2 <= width {
        spans.push(Span::raw(" ".repeat(width - left_width - right_width)));
        spans.extend(right);
    } else if left_width + 10 < width {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            truncate(
                &right.iter().map(|s| s.content.as_ref()).collect::<String>(),
                width - left_width - 2,
            ),
            theme::muted(),
        ));
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
    lines.push(Line::from(""));
    lines.push(dim(
        "Nothing already on disk is overwritten. `ostraka init` does the same.".to_string(),
    ));

    // What setting up will not fix, said before the offer is taken rather than
    // by a run failing later in somebody else's words.
    for (mark, said) in warnings(app, plan) {
        lines.push(Line::from(""));
        // Wrapped, because these are sentences rather than labels and the one
        // that overflows is the one explaining what will not work.
        for (i, part) in wrap(&said, area.width.saturating_sub(4) as usize)
            .into_iter()
            .enumerate()
        {
            lines.push(Line::from(vec![
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
    frame.render_widget(Paragraph::new(lines), area);
}

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
fn render_keys(frame: &mut Frame, screen: Rect) {
    let rows: Vec<(&str, &str)> = vec![
        ("type", "the box has the keys; what you type is the task"),
        ("enter", "run it"),
        ("alt-enter", "another line, for a task that needs one"),
        ("up / down", "what you have asked here before"),
        ("esc", "put the task aside, and take the keys back"),
        ("n", "take the box back"),
        ("ctrl-t", "another line of work, open beside this one"),
        ("ctrl-] / ctrl-[", "move between them"),
        ("s", "ask a running agent to stop"),
        ("l", "the runs recorded here"),
        ("w", "the repositories, and n starts one"),
        ("tab", "checks, events, diff \u{2014} on a record"),
        ("p", "promote a record; merges nothing"),
        ("pgup / pgdn", "scroll"),
        ("ctrl-k", "commands"),
        ("ctrl-x", "leader: the same commands, one key away"),
        ("?", "this list"),
        ("q", "quit"),
    ];

    let mut lines = vec![
        Line::from(Span::styled("keys", theme::bold())),
        Line::from(""),
    ];
    for (key, what) in &rows {
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<13}"), theme::accent()),
            Span::styled((*what).to_string(), theme::text()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(dim("esc closes this".to_string()));

    let area = theme::centred(screen, 72, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(theme::fit(lines, area.height))
            .block(theme::panel(true).padding(Padding::horizontal(2))),
        area,
    );
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
        if !agent.configured {
            role = "not configured — choosing writes it".to_string();
        }
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
            Span::styled(truncate(&agent.note, 40), theme::muted()),
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

fn event_lines(app: &App) -> Vec<Line<'static>> {
    if app.events.is_empty() {
        return vec![dim("no events recorded".to_string())];
    }
    let mut lines = Vec::new();
    for event in &app.events {
        let (kind, text, colour) = describe_event(event);
        lines.push(Line::from(vec![
            Span::styled(format!("{kind} "), theme::muted()),
            Span::styled(text, theme::on(colour)),
        ]));
    }
    lines
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
    match outcome {
        Some(Outcome::Approved) => "approved",
        Some(Outcome::Rejected) => "refused",
        Some(Outcome::Failed) => "failed",
        None => "unfinished",
    }
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
        }
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
        app.remedy = crate::tui::remedy::Remedy::diagnose(&dir);
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
        app.remedy = Some(crate::tui::remedy::Remedy::nothing_cloned(&dir));
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
        assert!(out.contains("ctrl-x w"), "{out}");
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
        assert!(out.contains("Write a task below"), "{out}");
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
        assert!(out.contains("refused"), "{out}");
        // Enough to recognise where the work got to, and not a second list.
        assert!(
            !out.contains("task number 7"),
            "the whole history was listed:\n{out}"
        );
        assert!(out.contains("8 runs"), "{out}");
        assert!(out.contains("looks any of them up"), "{out}");
        // And what to do next is still said.
        assert!(out.contains("Write a task below"), "{out}");

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
                "refused",
            )),
        );
        let out = screen(&mut app, 100, 20);
        let closing = out
            .lines()
            .find(|line| line.contains("refused") && line.contains(theme::RULE))
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
                "refused \u{2014} the author could not run (exit 1): Error: prompt is too \
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
        assert!(!out.contains("refused"), "{out}");
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
            writing.contains("say what the agent should do"),
            "{writing}"
        );

        app.focus = Focus::Keys;
        let idle = screen(&mut app, 100, 18);
        assert!(idle.contains("write a task"), "{idle}");
        assert!(idle.contains("ctrl-k"), "{idle}");
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
        app.filter = "refused".into();
        app.refilter();
        assert_eq!(app.matching.len(), 1);
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
    fn a_pending_leader_says_what_it_is_waiting_for() {
        let mut app = App::new(nowhere(), Vec::new());
        app.leader = true;
        let out = screen(&mut app, 120, 16);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("write a task"), "{out}");
    }

    #[test]
    fn the_keys_dialog_lists_the_keys_rather_than_a_footer_doing_it_forever() {
        let mut app = App::new(nowhere(), Vec::new());
        app.open(Dialog::Keys);
        let out = screen(&mut app, 110, 26);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("alt-enter"), "{out}");
        assert!(out.contains("esc closes this"), "{out}");
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
        assert!(out.contains("Write a task below"), "{out}");
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
        assert!(!screen(&mut app, 100, 20).contains("ctrl-t new"));

        app.open_pane();
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("1 new"), "{out}");
        assert!(out.contains("2 new"), "{out}");
        assert!(out.contains("ctrl-t new"), "{out}");
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
                out.contains("Write a task below"),
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

        // And where it fits, nothing is trimmed and nothing says it was.
        let roomy = screen(&mut app, 70, 30);
        assert!(roomy.contains("esc closes this"), "{roomy}");
        assert!(!roomy.contains("more line"), "{roomy}");
    }

    #[test]
    fn the_box_does_not_eat_a_short_terminal() {
        let mut app = App::new(nowhere(), Vec::new());
        app.pane_mut().prompt = (0..6).map(|i| format!("line {i}\n")).collect();
        let tall = screen(&mut app, 80, 40);
        assert!(tall.contains("line 5"), "{tall}");

        // On twelve rows the box gives way to the work it is about.
        let short = screen(&mut app, 80, 12);
        assert!(
            !short.contains("line 5"),
            "the box took the screen:\n{short}"
        );
        assert!(short.contains("Write a task below"), "{short}");
    }

    #[test]
    fn a_terminal_too_short_for_the_chrome_says_so_rather_than_drawing_nothing() {
        let mut app = App::new(nowhere(), Vec::new());
        assert!(screen(&mut app, 60, 4).contains("too short"));
    }
}
