//! What the browser shows, as a function of what it knows.
//!
//! Drawing is pure: state in, cells out. That is what lets it be tested against
//! a buffer rather than looked at — a screen nobody asserts on is a screen that
//! quietly stops saying what it used to.
//!
//! Detail lines are built as owned text rather than borrowed from the app. It
//! costs a few allocations per frame and buys the thing that matters: the lines
//! can be counted before they are drawn, which is how scrolling knows where the
//! bottom is.
//!
//! There is one box on this screen and it is the input line. Everything else is
//! separated by space and by a one-column gutter, because a browser whose every
//! region is boxed spends a quarter of a small terminal drawing lines around
//! nothing — and once the boxes are gone, the one that remains is unmistakably
//! where typing goes.

use crate::init::{Action, Plan};
use crate::tui::command::{Command, Situation};
use crate::tui::session::Session;
use crate::tui::theme;
use ostraka_core::gate::{CheckRecord, Verdict};
use ostraka_core::record::{Event, Outcome, RunRecord};
use ostraka_runtime::index::{self, BackendUsage, RunSummary};
use ostraka_runtime::progress::{Phase, Step};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};
use std::path::{Path, PathBuf};

/// Rows the chrome takes: the breadcrumb, a blank, the three of the input box
/// and the status line. Everything left over is content.
const CHROME: u16 = 6;

/// Rows one run takes in the list: what it was, how it went, and air.
const ENTRY: usize = 3;

/// Below this the list and the detail cannot both be read, so only one is
/// drawn and Enter moves between them.
const TWO_COLUMN: u16 = 72;

/// Which part of a run the detail pane is showing.
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

/// Which column the keys are talking to. Only ever consulted on a terminal too
/// narrow to show both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
}

/// The one thing that can be open over the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Keys,
    Commands,
}

/// What the input line is taking, when it is taking anything.
///
/// One box with two jobs, and it says which one it is doing. A filter narrows
/// what is already there; a prompt starts something that is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typing {
    Filter,
    Prompt,
}

pub struct App {
    pub project: PathBuf,
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
    /// First entry the list is showing, so a selection below the fold scrolls
    /// the list rather than disappearing off it.
    pub list_top: usize,
    pub filter: String,
    /// What is being written into the input line, if anything.
    pub typing: Option<Typing>,
    /// The task being composed, kept separately from the filter: abandoning a
    /// half-written task to look something up and coming back to it is the
    /// normal way of working, and one string for both would lose it.
    pub prompt: String,
    /// The run this browser started, while it is going and after it ends.
    pub session: Option<Session>,
    /// Ticks of the event loop. What the spinner is a function of, so that
    /// drawing is a function of state rather than of the clock.
    pub tick: u64,
    /// Whether the transcript sticks to the bottom as the run writes to it.
    /// Off the moment somebody scrolls up, on again at the end.
    pub follow: bool,
    /// The full record of the selected run, loaded on demand: a listing holds
    /// summaries, and check output is the thing worth reading on a failure.
    pub record: Option<RunRecord>,
    pub events: Vec<Event>,
    /// `None` until asked for; `Some(None)` once asked and not found.
    pub diff: Option<Option<String>>,
    pub detail: Detail,
    pub focus: Focus,
    pub scroll: u16,
    /// Height of the detail pane at the last draw, so a page key can move by a
    /// page rather than by a number somebody guessed.
    pub page: u16,
    pub status: Option<String>,
    /// Present when this directory is not a project yet: what `init` would
    /// write. `None` once there is nothing left to write.
    pub setup: Option<Plan>,
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
    pub quit: bool,
}

impl App {
    pub fn new(project: PathBuf, runs: Vec<RunSummary>) -> Self {
        let matching = (0..runs.len()).collect();
        Self {
            where_shown: where_we_are(&project),
            project,
            runs,
            matching,
            selected: 0,
            list_top: 0,
            filter: String::new(),
            typing: None,
            prompt: String::new(),
            session: None,
            tick: 0,
            follow: true,
            record: None,
            events: Vec::new(),
            diff: None,
            detail: Detail::Checks,
            focus: Focus::List,
            scroll: 0,
            page: 10,
            status: None,
            setup: None,
            dialog: None,
            query: String::new(),
            pick: 0,
            leader: false,
            leaving: false,
            quit: false,
        }
    }

    pub fn current(&self) -> Option<&RunSummary> {
        self.runs.get(*self.matching.get(self.selected)?)
    }

    /// Recomputes which runs match, keeping the selection in range.
    ///
    /// A filter that hides the selected run moves the selection rather than
    /// leaving it pointing at something the list no longer shows.
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
            self.follow = false;
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
            running: self.session.as_ref().is_some_and(|s| s.live()),
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

    /// Opens a dialog from a clean slate. A palette that reopened holding the
    /// last query would answer a question nobody had asked yet.
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
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(screen);

    frame.render_widget(breadcrumb(app, screen.width), theme::inset(rows[0]));

    if app.setup.is_some() {
        render_setup(frame, app, theme::inset(rows[2]));
    } else {
        render_content(frame, app, rows[2]);
    }

    render_prompt(frame, app, rows[3]);
    frame.render_widget(
        Paragraph::new(status_bar(app, rows[4].width.saturating_sub(theme::GUTTER))),
        theme::inset(rows[4]),
    );

    match app.dialog {
        Some(Dialog::Keys) => render_keys(frame, screen),
        Some(Dialog::Commands) => render_palette(frame, app, screen),
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
    Paragraph::new(Line::from(Span::styled(
        truncate(&app.where_shown, width.saturating_sub(2) as usize),
        theme::muted(),
    )))
}

/// A project path as a person would write it.
///
/// Resolved, because the default is `.` and a breadcrumb reading `.` tells
/// nobody which of their checkouts this is; and shortened to `~`, because the
/// first four segments of every path here are the same four.
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

fn render_content(frame: &mut Frame, app: &mut App, area: Rect) {
    // Said once, across the whole area, rather than once per column: an empty
    // list and an empty detail are the same fact, and printing it twice reads
    // as two different things having gone wrong.
    //
    // Not while a run is being watched. The first run in a repository is
    // started from an empty list, and "No runs yet" over the top of the run
    // that is fixing that would be the screen contradicting itself.
    if app.matching.is_empty() && app.session.is_none() {
        let message = if app.runs.is_empty() {
            "No runs yet. `ostraka run \"\u{2026}\"` makes one."
        } else {
            "Nothing matches this filter."
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(message, theme::muted()))),
            theme::inset(area),
        );
        return;
    }

    if app.matching.is_empty() {
        render_transcript(frame, app, theme::inset(area));
        return;
    }

    if area.width < TWO_COLUMN {
        match app.focus {
            Focus::List => render_list(frame, app, area),
            Focus::Detail => render_beside(frame, app, theme::inset(area)),
        }
        return;
    }

    // A fraction, then a ceiling: a list column wider than about forty-five
    // columns is holding whitespace, and the detail pane is where the reading
    // happens.
    let list_width = (area.width * 2 / 5).clamp(24, 46);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(list_width), Constraint::Min(10)])
        .split(area);

    render_list(frame, app, columns[0]);
    render_beside(frame, app, theme::inset(columns[1]));
}

/// The column next to the list: a run in progress if there is one, and
/// otherwise the record of the one selected.
///
/// A run being watched wins. It is the thing that is changing, it is the
/// reason the browser was left open, and the selected record will still be
/// there afterwards — with the new run in the list beside it.
fn render_beside(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.session.is_some() {
        render_transcript(frame, app, area);
    } else {
        render_detail(frame, app, area);
    }
}

/// The runs, two lines each: what was asked, and how it went.
///
/// Rendered as text rather than as a list widget because the selected entry is
/// marked in a gutter that spans both of its lines, and a widget that owns one
/// row per item cannot draw that.
fn render_list(frame: &mut Frame, app: &mut App, area: Rect) {
    // Clamped here because here is the only place that knows both how many
    // entries there are and how tall the column is.
    let visible = (area.height as usize / ENTRY).max(1);
    if app.selected < app.list_top {
        app.list_top = app.selected;
    }
    if app.selected >= app.list_top + visible {
        app.list_top = app.selected + 1 - visible;
    }

    let width = area.width.saturating_sub(theme::GUTTER) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, run) in app
        .matching
        .iter()
        .enumerate()
        .skip(app.list_top)
        .take(visible)
        .filter_map(|(i, index)| app.runs.get(*index).map(|run| (i, run)))
    {
        let here = i == app.selected;
        let (mark, colour) = marker(run.outcome);
        let cursor = if here { theme::CURSOR } else { " " };
        let title = if here {
            theme::text().add_modifier(Modifier::BOLD)
        } else {
            theme::text()
        };

        lines.push(Line::from(vec![
            Span::styled(cursor, theme::on(colour)),
            Span::styled(format!(" {mark}  "), theme::on(colour)),
            Span::styled(when(&run.run_id), theme::muted()),
            Span::raw("  "),
            Span::styled(
                truncate(&described(&run.prompt), width.saturating_sub(19)),
                title,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(cursor, theme::on(colour)),
            Span::styled(format!(" {}  ", theme::CONTINUE), theme::muted()),
            Span::styled(
                truncate(
                    &format!(
                        "{} of {} checks \u{b7} {}",
                        run.checks_passed,
                        run.checks_total,
                        outcome_word(run.outcome)
                    ),
                    width.saturating_sub(5),
                ),
                theme::muted(),
            ),
        ]));
        lines.push(Line::from(""));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    // Nothing selected is drawn by `render_content` before it splits the area,
    // so reaching here with no run means the two disagree. Drawing nothing is
    // the honest answer to that, and not a panic.
    let Some(run) = app.current().cloned() else {
        return;
    };

    app.page = area.height.saturating_sub(1).max(1);

    let mut lines = summary_lines(app, &run, area.width as usize);
    lines.push(tabs(app));
    match app.detail {
        Detail::Checks => lines.extend(check_lines(app, &run)),
        Detail::Events => lines.extend(event_lines(app)),
        Detail::Diff => lines.extend(diff_lines(app)),
    }

    // Clamped here because here is the only place that knows both how many
    // lines there are and how tall the pane is. Scrolling past the end shows an
    // empty box, which reads as a broken screen.
    let overflow = lines.len().saturating_sub(area.height as usize);
    app.scroll = app.scroll.min(overflow as u16);

    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
}

/// A run as it happens: what was asked, then how far it has got.
fn render_transcript(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(lines) = app
        .session
        .as_ref()
        .map(|session| transcript_lines(session, area.width as usize, app.tick))
    else {
        return;
    };
    app.page = area.height.saturating_sub(1).max(1);

    let overflow = lines.len().saturating_sub(area.height as usize) as u16;
    // Stuck to the bottom while the run writes to it, which is what a live log
    // has to do to be worth watching, and released the moment somebody scrolls
    // up to read something.
    if app.follow {
        app.scroll = overflow;
    }
    app.scroll = app.scroll.min(overflow);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
}

fn transcript_lines(session: &Session, width: usize, tick: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, part) in wrap(&session.prompt, width.saturating_sub(2))
        .into_iter()
        .enumerate()
    {
        lines.push(Line::from(vec![
            Span::styled(
                if i == 0 { theme::CURSOR } else { " " },
                theme::on(theme::ACCENT),
            ),
            Span::raw(" "),
            Span::styled(part, theme::bold()),
        ]));
    }
    lines.push(Line::from(""));

    for step in &session.steps {
        match step {
            Step::Entered(phase) => lines.push(phase_line(*phase, session.phase, tick)),
            Step::Said { event, .. } => {
                let (kind, text, colour) = describe_event(event);
                lines.push(Line::from(vec![
                    Span::styled(format!("{} {kind} ", theme::CONTINUE), theme::muted()),
                    Span::styled(truncate(&text, width.saturating_sub(8)), theme::on(colour)),
                ]));
            }
            Step::Checked(record) => lines.extend(checked_lines(record)),
        }
    }

    lines.push(Line::from(""));
    if let Some(finished) = &session.finished {
        let (_, colour) = marker(finished.outcome);
        lines.push(Line::from(Span::styled(
            finished.summary.clone(),
            theme::on(colour),
        )));
        lines.push(dim(format!("{}  \u{b7}  esc closes this", finished.run_id)));
    } else if let Some(why) = &session.failed {
        // Not a verdict on the change. The run did not get far enough to have
        // one, and reporting "refused" here would say it did.
        lines.push(Line::from(Span::styled(
            format!("the run did not finish \u{2014} {why}"),
            theme::on(theme::BAD),
        )));
        lines.push(dim("esc closes this".to_string()));
    } else if session.stopping {
        lines.push(dim(
            "stopping \u{2014} the agent is being asked to stop".to_string()
        ));
    }
    lines
}

/// A phase heading, marked while it is the one happening.
fn phase_line(phase: Phase, current: Option<Phase>, tick: u64) -> Line<'static> {
    let mut spans = vec![Span::styled(
        phase.title(),
        theme::accent().add_modifier(Modifier::BOLD),
    )];
    if current == Some(phase) {
        let frame = theme::SPINNER[tick as usize % theme::SPINNER.len()];
        spans.push(Span::styled(format!("  {frame}"), theme::accent()));
    }
    Line::from(spans)
}

/// One finished check, under the rail, with its output if it failed.
fn checked_lines(record: &CheckRecord) -> Vec<Line<'static>> {
    let (mark, word, colour) = check_mark(record);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", theme::CONTINUE), theme::muted()),
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

/// The three panes, named, with the one you are in marked.
///
/// A row of names rather than a hint saying which key shows the next one: the
/// names are the same width either way, and this way the screen says what it
/// has instead of what to press to find out.
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
    let block = theme::panel(app.typing.is_some());
    let text = match app.typing {
        // A prompt and a filter are different enough that the box says which
        // it is taking. One starts something; the other narrows what is there,
        // and typing a task into a filter would be a quiet way to lose it.
        Some(Typing::Prompt) => Line::from(vec![
            Span::styled("\u{203a}  ", theme::accent()),
            Span::styled(app.prompt.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
            Span::styled("     enter to run \u{b7} esc sets it aside", theme::muted()),
        ]),
        Some(Typing::Filter) => Line::from(vec![
            Span::styled("/  ", theme::accent()),
            Span::styled(app.filter.clone(), theme::text()),
            Span::styled(theme::CURSOR, theme::accent()),
        ]),
        None if !app.filter.is_empty() => Line::from(vec![
            Span::styled("/  ", theme::accent()),
            Span::styled(app.filter.clone(), theme::text()),
            Span::styled("     esc to clear", theme::muted()),
        ]),
        None => Line::from(Span::styled(
            "n  new run     /  filter runs     ctrl-k  commands     ?  keys",
            theme::muted(),
        )),
    };
    frame.render_widget(
        Paragraph::new(text).block(block.padding(Padding::horizontal(1))),
        inner,
    );
}

/// The bottom line: who you are running, and what it has cost.
///
/// One row carrying three different things at different times, in the order
/// they matter. A message about what just happened wins, because it is the
/// answer to the key that was just pressed; a pending leader wins next,
/// because a terminal that has swallowed a keystroke and says nothing is a
/// terminal that looks broken.
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

    let left = match app.session.as_ref().filter(|s| s.live()) {
        // While something is happening, the left of this line is what is
        // happening. The counts it displaces cannot change until the run ends.
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
        // No room for both. The counts on the left are the ones that change
        // as you move, so what is left of the line goes to the totals.
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
    // Said the way the vendor said it. One reports a split, another a single
    // combined figure, and printing the second as "14.7k in / 0 out" would put
    // a number in its mouth it never gave.
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
///
/// Lists exactly what would be written and what is already there, because a
/// setup step that writes into someone's repository should say what it is about
/// to do before it does it.
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
    frame.render_widget(Paragraph::new(lines), area);
}

/// Every key the screen answers to, in one place someone can read.
fn render_keys(frame: &mut Frame, screen: Rect) {
    let rows: Vec<(&str, &str)> = vec![
        ("n", "write a task and run it"),
        ("s", "ask a running agent to stop"),
        ("j / k", "move between runs"),
        ("g / G", "first run / last run"),
        ("space / b", "scroll the pane"),
        ("tab", "checks, events, diff"),
        (
            "enter / esc",
            "into the pane and back, on a narrow terminal",
        ),
        ("/", "filter runs"),
        ("p", "promote the selected run"),
        ("r", "reload the run records"),
        ("ctrl-k", "commands"),
        ("ctrl-x", "leader: the same commands, one key away"),
        ("?", "this list"),
        ("q", "quit"),
    ];

    let mut lines = vec![Line::from(Span::styled("keys", theme::bold()))];
    lines.push(Line::from(""));
    for (key, what) in &rows {
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<13}"), theme::accent()),
            Span::styled((*what).to_string(), theme::text()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(dim("esc closes this".to_string()));

    let area = theme::centred(screen, 66, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(theme::panel(true).padding(Padding::horizontal(2))),
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
        Line::from(""),
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
        Paragraph::new(lines).block(theme::panel(true).padding(Padding::horizontal(2))),
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
        // Not an error. Once neither the run's branch nor its promotion exists,
        // the change was merged or discarded — which is an answer.
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
///
/// Blank is worse than a sentence saying why it is blank: one looks like a bug
/// in the screen, the other like an old record, which is what it is.
fn described(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        "(recorded before runs kept the task text)".to_string()
    } else {
        prompt.to_string()
    }
}

/// A labelled value, wrapped under its own label.
///
/// Wrapped here rather than by the widget: `Paragraph::wrap` would make the
/// number of rendered lines differ from the number of logical ones, and the
/// scroll clamp counts logical lines. Every field wraps, not just the task —
/// the value most likely to be long is a rejection's reason, and cutting that
/// off loses the sentence someone opened this pane to read.
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
    use ostraka_core::gate::{Approval, CheckRecord};
    use ostraka_core::identity::ActorId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn summary(run_id: &str, prompt: &str, outcome: Option<Outcome>) -> RunSummary {
        RunSummary {
            run_id: run_id.to_string(),
            started_at: "2026-09-07T00:03:00Z".to_string(),
            prompt: prompt.to_string(),
            author: ActorId::new("archon"),
            adapter: "claude-code".to_string(),
            reviewer: Some(ActorId::new("ephor")),
            outcome,
            checks_passed: 3,
            checks_total: 4,
            usage: Vec::new(),
        }
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draws");
        let buffer = terminal.backend().buffer().clone();
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_directory_that_is_not_a_project_says_what_would_be_written() {
        // Rather than an empty run list, which reads as a broken screen.
        let dir = std::env::temp_dir().join(format!("ostraka-tui-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").expect("write");

        let mut app = App::new(dir.clone(), Vec::new());
        app.setup = Some(crate::init::plan(&dir));

        let out = screen(&mut app, 100, 22);
        assert!(out.contains("not an Ostraka project yet"), "{out}");
        assert!(out.contains("a Rust project"), "{out}");
        assert!(out.contains("ostraka.toml"), "{out}");
        assert!(out.contains("adapters/codex.toml"), "{out}");
        assert!(out.contains("i set this directory up"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_names_what_is_already_there_rather_than_offering_to_rewrite_it() {
        let dir = std::env::temp_dir().join(format!("ostraka-tui-half-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("ostraka.toml"), "# mine\n").expect("write");

        let mut app = App::new(dir.clone(), Vec::new());
        app.setup = Some(crate::init::plan(&dir));
        let out = screen(&mut app, 100, 22);
        assert!(out.contains("already there"), "{out}");
        assert!(
            out.contains("Nothing already on disk is overwritten"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_with_no_runs_says_so_rather_than_showing_an_empty_frame() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let out = screen(&mut app, 90, 12);
        assert!(out.contains("0 runs"), "{out}");
        assert!(out.contains("No runs yet"), "{out}");
    }

    #[test]
    fn the_list_shows_what_each_run_was_for_and_how_it_ended() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![
                summary("t1-20260907T000300Z", "add a test", Some(Outcome::Approved)),
                summary(
                    "t2-20260907T000100Z",
                    "rename a field",
                    Some(Outcome::Rejected),
                ),
            ],
        );
        let out = screen(&mut app, 100, 16);
        assert!(out.contains("2 runs"), "{out}");
        assert!(out.contains("add a test"), "{out}");
        assert!(out.contains("rename a field"), "{out}");
        assert!(out.contains("09-07 00:03"), "{out}");
        assert!(out.contains("approved"), "{out}");
        // Each entry carries how far it got under what it was for, so the list
        // answers "did it pass" without anybody selecting the run.
        assert!(out.contains("3 of 4 checks"), "{out}");
    }

    #[test]
    fn the_selected_run_is_marked_down_its_whole_entry() {
        // Two lines, one mark: a highlight on only the first of them reads as
        // a different run from the second.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![
                summary("t1-20260907T000300Z", "first", Some(Outcome::Approved)),
                summary("t2-20260907T000100Z", "second", Some(Outcome::Approved)),
            ],
        );
        app.move_by(1);
        let out = screen(&mut app, 100, 16);
        let marked: Vec<&str> = out
            .lines()
            .filter(|line| line.starts_with(CURSOR_AT_LEFT))
            .collect();
        assert_eq!(marked.len(), 2, "the cursor did not span the entry:\n{out}");
        assert!(marked[0].contains("second"), "{out}");
        assert!(marked[1].contains("checks"), "{out}");
    }

    /// The cursor as it appears at the very left of a rendered row. The list
    /// column starts at the frame's edge, so the bar is the first cell.
    const CURSOR_AT_LEFT: &str = "\u{258c}";

    #[test]
    fn a_list_taller_than_its_column_scrolls_to_keep_the_selection_in_view() {
        let mut app = App::new(
            PathBuf::from("/p"),
            (0..20)
                .map(|i| {
                    summary(
                        &format!("t{i}-2026090{}T000300Z", i % 10),
                        &format!("task number {i}"),
                        Some(Outcome::Approved),
                    )
                })
                .collect(),
        );
        app.move_by(19);
        let out = screen(&mut app, 100, 16);
        assert!(
            out.contains("task number 19"),
            "the end is unreachable:\n{out}"
        );
        assert!(
            !out.contains("task number 0 "),
            "the top did not scroll:\n{out}"
        );
    }

    #[test]
    fn a_narrow_terminal_shows_one_column_and_enter_moves_between_them() {
        // Both columns on forty-eight columns means neither can be read.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "narrow",
                Some(Outcome::Approved),
            )],
        );
        let list = screen(&mut app, 48, 16);
        assert!(list.contains("narrow"), "{list}");
        assert!(
            !list.contains("checks   events"),
            "both columns were drawn:\n{list}"
        );

        app.focus = Focus::Detail;
        let detail = screen(&mut app, 48, 16);
        assert!(detail.contains("checks"), "{detail}");
        assert!(detail.contains("author"), "{detail}");
    }

    #[test]
    fn a_failing_check_shows_the_output_that_explains_it() {
        // The reason someone opens this screen. A passing check's output is
        // noise; a failing one's is the whole point.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "break the build",
                Some(Outcome::Rejected),
            )],
        );
        app.record = Some(RunRecord {
            run_id: "t1-20260907T000300Z".into(),
            task_id: "t1".into(),
            prompt: "break the build".into(),
            author: ActorId::new("archon"),
            adapter: "claude-code".into(),
            started_at: String::new(),
            finished_at: None,
            checks: vec![
                CheckRecord {
                    name: "format".into(),
                    cmd: "fmt".into(),
                    exit_code: Some(0),
                    stdout: "quiet success".into(),
                    stderr: String::new(),
                    duration_ms: 12,
                },
                CheckRecord {
                    name: "test".into(),
                    cmd: "test".into(),
                    exit_code: Some(101),
                    stdout: String::new(),
                    stderr: "assertion failed: left == right".into(),
                    duration_ms: 340,
                },
            ],
            approval: None,
            usage: Vec::new(),
            outcome: Some(Outcome::Rejected),
        });

        let out = screen(&mut app, 100, 20);
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("assertion failed"), "{out}");
        assert!(
            !out.contains("quiet success"),
            "a passing check's output was shown:\n{out}"
        );
    }

    #[test]
    fn the_detail_pane_switches_to_what_the_agents_said() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "do a thing",
                Some(Outcome::Rejected),
            )],
        );
        app.events = vec![
            Event::Message {
                text: "I could not reach the model".into(),
                raw: None,
            },
            Event::Finished {
                exit_code: Some(1),
                files_touched: vec![],
            },
        ];
        app.detail = Detail::Events;
        let out = screen(&mut app, 100, 18);
        assert!(out.contains("could not reach the model"), "{out}");
        assert!(out.contains("exit 1"), "{out}");
    }

    #[test]
    fn the_detail_tabs_name_every_pane_and_mark_the_one_showing() {
        // The names, not a hint about the key that reveals the next one: the
        // row costs the same width either way and says more.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary("t1-20260907T000300Z", "a", Some(Outcome::Approved))],
        );
        let out = screen(&mut app, 100, 18);
        for pane in Detail::ALL {
            assert!(out.contains(pane.title()), "{pane:?} unnamed:\n{out}");
        }
        assert_eq!(marked_tab(&mut app, 100, 18), Some("checks".to_string()));

        app.detail = app.detail.next();
        assert_eq!(marked_tab(&mut app, 100, 18), Some("events".to_string()));
        app.detail = app.detail.next();
        assert_eq!(marked_tab(&mut app, 100, 18), Some("diff".to_string()));
        assert_eq!(app.detail.next(), Detail::Checks);
    }

    /// The tab drawn in the accent colour, read back off the rendered cells.
    ///
    /// Asserted on the styling rather than on the text because all three names
    /// are on screen either way: which one is marked is the whole claim.
    fn marked_tab(app: &mut App, width: u16, height: u16) -> Option<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
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
        (!found.is_empty()).then_some(found)
    }

    #[test]
    fn a_rejection_shows_the_reason_the_reviewer_gave() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "do a thing",
                Some(Outcome::Rejected),
            )],
        );
        app.record = Some(RunRecord {
            run_id: "t1-20260907T000300Z".into(),
            task_id: "t1".into(),
            prompt: "do a thing".into(),
            author: ActorId::new("archon"),
            adapter: "claude-code".into(),
            started_at: String::new(),
            finished_at: None,
            checks: Vec::new(),
            approval: Some(Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Reject {
                    reason: "went outside the task".into(),
                },
            }),
            usage: Vec::new(),
            outcome: Some(Outcome::Rejected),
        });
        let out = screen(&mut app, 100, 16);
        assert!(out.contains("went outside the task"), "{out}");
    }

    #[test]
    fn a_long_rejection_reason_wraps_rather_than_running_off_the_pane() {
        // The value most likely to be too long for one row is the sentence
        // explaining why a change was refused, which is the sentence someone
        // opened this pane to read.
        let reason = "The file is not newline-terminated, so it comes out of \
                      the checkout differently from every other file in this \
                      repository and the format check would fail on it next time";
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "write a file",
                Some(Outcome::Rejected),
            )],
        );
        app.record = Some(RunRecord {
            run_id: "t1-20260907T000300Z".into(),
            task_id: "t1".into(),
            prompt: "write a file".into(),
            author: ActorId::new("archon"),
            adapter: "claude-code".into(),
            started_at: String::new(),
            finished_at: None,
            checks: Vec::new(),
            approval: Some(Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Reject {
                    reason: reason.into(),
                },
            }),
            usage: Vec::new(),
            outcome: Some(Outcome::Rejected),
        });

        let out = screen(&mut app, 100, 24);
        // The last words of the reason, which is what a cut-off line loses.
        // Asserted on a fragment short enough to survive any wrap point: a
        // longer phrase would straddle a break the moment the pane changed
        // width by a column, and fail for a reason that is not the claim.
        assert!(out.contains("on it next time"), "{out}");
        assert!(
            out.lines().all(|line| line.chars().count() <= 100),
            "a line ran past the frame:\n{out}"
        );
    }

    #[test]
    fn moving_stays_inside_the_list() {
        let mut app = App::new(
            PathBuf::from("/p"),
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.move_by(1);
        app.move_by(-1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn the_diff_pane_shows_the_change_and_marks_its_sides() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "add a line",
                Some(Outcome::Approved),
            )],
        );
        app.detail = Detail::Diff;
        app.diff = Some(Some(
            "diff --git a/x b/x\n@@ -1 +1,2 @@\n context\n+added line\n-removed line\n".into(),
        ));
        let out = screen(&mut app, 100, 20);
        assert!(out.contains("+added line"), "{out}");
        assert!(out.contains("-removed line"), "{out}");
        assert!(out.contains("@@ -1 +1,2 @@"), "{out}");
    }

    #[test]
    fn a_run_with_no_commit_names_both_reasons_rather_than_showing_an_empty_pane() {
        // A refused run never committed; an old approved one may have been
        // merged away. Same appearance, two causes, and guessing between them
        // would be wrong half the time.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "add a line",
                Some(Outcome::Approved),
            )],
        );
        app.detail = Detail::Diff;
        app.diff = Some(None);
        let out = screen(&mut app, 100, 18);
        assert!(out.contains("produced no commit"), "{out}");
        assert!(out.contains("merged away"), "{out}");
    }

    #[test]
    fn scrolling_cannot_run_off_the_end_of_the_content() {
        // Past the end is an empty box, which reads as a broken screen.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "look at a diff",
                Some(Outcome::Approved),
            )],
        );
        app.detail = Detail::Diff;
        app.diff = Some(Some((0..200).map(|i| format!("+line {i}\n")).collect()));

        app.scroll_by(10_000);
        let out = screen(&mut app, 100, 20);
        assert!(app.scroll < 250, "scroll ran away: {}", app.scroll);
        assert!(
            out.contains("line 199"),
            "the end was not reachable:\n{out}"
        );

        app.scroll_by(-10_000);
        let top = screen(&mut app, 100, 20);
        assert_eq!(app.scroll, 0);
        assert!(top.contains("t1-20260907T000300Z"), "{top}");
    }

    #[test]
    fn a_filter_narrows_the_list_and_says_how_far() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![
                summary(
                    "t1-20260907T000300Z",
                    "rename a field",
                    Some(Outcome::Approved),
                ),
                summary("t2-20260907T000200Z", "add a test", Some(Outcome::Rejected)),
                summary(
                    "t3-20260907T000100Z",
                    "add a check",
                    Some(Outcome::Approved),
                ),
            ],
        );
        app.filter = "add".into();
        app.refilter();
        assert_eq!(app.matching.len(), 2);

        let out = screen(&mut app, 100, 16);
        assert!(out.contains("2 of 3 runs"), "{out}");
        assert!(!out.contains("rename a field"), "{out}");
        assert!(out.contains("add a test"), "{out}");
    }

    #[test]
    fn a_filter_matches_the_outcome_word_too() {
        let mut app = App::new(
            PathBuf::from("/p"),
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

    #[test]
    fn a_filter_that_hides_the_selection_moves_it_rather_than_dangling() {
        let mut app = App::new(
            PathBuf::from("/p"),
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
    fn a_filter_matching_nothing_says_so_instead_of_looking_broken() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "alpha",
                Some(Outcome::Approved),
            )],
        );
        app.filter = "nothing matches this".into();
        app.refilter();
        let out = screen(&mut app, 100, 12);
        assert!(out.contains("Nothing matches this filter"), "{out}");
        assert!(out.contains("0 of 1 runs"), "{out}");
    }

    #[test]
    fn the_input_line_says_where_typing_goes_before_anybody_types() {
        // The one box on the screen, and the only place a keystroke becomes
        // text. A placeholder is what makes that legible without a legend.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let idle = screen(&mut app, 100, 12);
        assert!(idle.contains("new run"), "{idle}");
        assert!(idle.contains("filter runs"), "{idle}");
        assert!(idle.contains("ctrl-k"), "{idle}");
        assert!(idle.contains("\u{256d}"), "the box is not drawn:\n{idle}");

        app.typing = Some(Typing::Filter);
        app.filter = "typed".into();
        let filtering = screen(&mut app, 100, 12);
        assert!(filtering.contains("typed"), "{filtering}");
        assert!(!filtering.contains("new run"), "{filtering}");
    }

    #[test]
    fn the_box_says_whether_it_is_taking_a_task_or_a_filter() {
        // Two jobs, one box. Typing a task into a filter would narrow the list
        // to nothing and lose the sentence, which is a quiet way to fail.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.typing = Some(Typing::Prompt);
        app.prompt = "add a wall-clock ceiling".into();
        let out = screen(&mut app, 100, 12);
        assert!(out.contains("add a wall-clock ceiling"), "{out}");
        assert!(out.contains("enter to run"), "{out}");
        // Set aside, not dropped: escape keeps the text, and the box has to
        // say the thing it actually does.
        assert!(out.contains("sets it aside"), "{out}");
        assert!(!out.contains("filter runs"), "{out}");
    }

    #[test]
    fn the_command_palette_names_what_it_can_do_and_marks_the_pick() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.open(Dialog::Commands);
        let out = screen(&mut app, 110, 24);
        assert!(out.contains("promote run"), "{out}");
        assert!(out.contains("reload runs"), "{out}");
        // The key beside the name, so the palette teaches its own shortcut.
        assert!(out.contains("merges nothing"), "{out}");

        app.query = "promote".into();
        let narrowed = screen(&mut app, 110, 24);
        assert!(narrowed.contains("promote run"), "{narrowed}");
        assert!(!narrowed.contains("reload runs"), "{narrowed}");
    }

    #[test]
    fn a_palette_query_matching_nothing_says_so_rather_than_showing_an_empty_box() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.open(Dialog::Commands);
        app.query = "xyzzy".into();
        let out = screen(&mut app, 110, 24);
        assert!(out.contains("no command matches that"), "{out}");
        assert_eq!(app.picked(), None);
    }

    #[test]
    fn the_keys_dialog_lists_the_keys_rather_than_a_footer_doing_it_forever() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.open(Dialog::Keys);
        let out = screen(&mut app, 110, 24);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("promote the selected run"), "{out}");
        assert!(out.contains("esc closes this"), "{out}");
    }

    #[test]
    fn a_pending_leader_says_what_it_is_waiting_for() {
        // A terminal that has swallowed a keystroke and shows nothing is a
        // terminal that looks broken.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.leader = true;
        let out = screen(&mut app, 110, 14);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("promote run"), "{out}");
    }

    use crate::tui::session::Finished;

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

    #[test]
    fn a_run_in_progress_takes_the_column_beside_the_list() {
        // What is changing wins the pane. The selected record will still be
        // there afterwards; the run will not.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "an older run",
                Some(Outcome::Approved),
            )],
        );
        app.session = Some(Session::recorded(
            "add a wall-clock ceiling to the gate",
            vec![
                Step::Entered(Phase::Isolating),
                Step::Entered(Phase::Authoring),
                said(
                    Phase::Authoring,
                    "editing crates/ostraka-runtime/src/gate.rs",
                ),
            ],
            None,
        ));
        let out = screen(&mut app, 110, 22);
        assert!(out.contains("add a wall-clock ceiling"), "{out}");
        assert!(out.contains("isolate"), "{out}");
        assert!(out.contains("editing crates"), "{out}");
        // The list is still there beside it.
        assert!(out.contains("an older run"), "{out}");
    }

    #[test]
    fn the_transcript_marks_only_the_phase_that_is_still_going() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.session = Some(Session::recorded(
            "do a thing",
            vec![
                Step::Entered(Phase::Authoring),
                Step::Entered(Phase::Gating),
            ],
            None,
        ));
        let spinning = |app: &mut App| {
            screen(app, 110, 22)
                .lines()
                .filter(|line| theme::SPINNER.iter().any(|f| line.contains(f)))
                .count()
        };
        assert_eq!(spinning(&mut app), 1, "one phase is running, not two");

        // And it turns, so a long phase does not look like a hung screen.
        let first = screen(&mut app, 110, 22);
        app.tick += 1;
        assert_ne!(first, screen(&mut app, 110, 22), "the mark did not move");
    }

    #[test]
    fn a_failing_check_shows_its_output_without_waiting_for_the_record() {
        // The whole point of streaming the gate: the record does not exist
        // until the run ends, and the failing check is what someone is waiting
        // to read.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.session = Some(Session::recorded(
            "break the build",
            vec![
                Step::Entered(Phase::Gating),
                Step::Checked(check("format", 0, "")),
                Step::Checked(check("test", 101, "assertion failed: left == right")),
            ],
            None,
        ));
        let out = screen(&mut app, 110, 22);
        assert!(out.contains("pass"), "{out}");
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("assertion failed"), "{out}");
    }

    #[test]
    fn a_finished_run_says_how_it_ended_and_how_to_leave_it() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.session = Some(Session::recorded(
            "do a thing",
            vec![Step::Entered(Phase::Reviewing)],
            Some(Finished {
                run_id: "t1-20260907T000300Z".into(),
                outcome: Some(Outcome::Approved),
                summary: "approved \u{2014} nothing merged".into(),
            }),
        ));
        let out = screen(&mut app, 110, 22);
        assert!(out.contains("nothing merged"), "{out}");
        assert!(out.contains("t1-20260907T000300Z"), "{out}");
        assert!(out.contains("esc closes this"), "{out}");
    }

    #[test]
    fn a_run_that_never_started_is_not_reported_as_a_refusal() {
        // "Refused" is a verdict on a change. A run that could not be launched
        // never produced one, and saying it was refused would put a judgement
        // in the record's mouth that nobody made.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let mut session = Session::recorded("do a thing", Vec::new(), None);
        session.failed = Some("no adapter profile can run here".into());
        app.session = Some(session);

        let out = screen(&mut app, 110, 22);
        assert!(out.contains("did not finish"), "{out}");
        assert!(out.contains("no adapter profile"), "{out}");
        assert!(!out.contains("refused"), "{out}");
    }

    #[test]
    fn the_first_run_in_an_empty_repository_is_not_hidden_by_no_runs_yet() {
        // The list is empty precisely because this run has not finished. The
        // screen contradicting itself there would be its first impression.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.session = Some(Session::recorded(
            "the very first task",
            vec![Step::Entered(Phase::Authoring)],
            None,
        ));
        let out = screen(&mut app, 100, 16);
        assert!(out.contains("the very first task"), "{out}");
        assert!(!out.contains("No runs yet"), "{out}");
    }

    #[test]
    fn the_status_line_says_what_is_happening_while_a_run_is_going() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.session = Some(Session::recorded(
            "do a thing",
            vec![Step::Entered(Phase::Gating)],
            None,
        ));
        let out = screen(&mut app, 110, 16);
        assert!(out.contains("running"), "{out}");
        assert!(out.contains("gate"), "{out}");
        assert!(out.contains("41s"), "{out}");

        app.session.as_mut().expect("a session").stopping = true;
        assert!(screen(&mut app, 110, 16).contains("stopping"));
    }

    #[test]
    fn a_live_transcript_follows_its_own_tail_until_somebody_scrolls_up() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let mut steps = vec![Step::Entered(Phase::Authoring)];
        steps.extend((0..60).map(|i| said(Phase::Authoring, &format!("line {i}"))));
        app.session = Some(Session::recorded("a talkative agent", steps, None));

        let following = screen(&mut app, 100, 20);
        assert!(
            following.contains("line 59"),
            "the tail was not shown:\n{following}"
        );

        app.scroll_by(-30);
        assert!(!app.follow, "scrolling up did not release the tail");
        let held = screen(&mut app, 100, 20);
        assert!(
            !held.contains("line 59"),
            "the view snapped back to the tail:\n{held}"
        );
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
            PathBuf::from("/p"),
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
        let out = screen(&mut app, 120, 16);
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
            PathBuf::from("/p"),
            vec![with_usage(
                summary("t1-20260907T000300Z", "one", Some(Outcome::Approved)),
                &[("copilot-cli", "reviewer", 10_600, Some(296), true)],
            )],
        );
        let out = screen(&mut app, 120, 14);
        assert!(out.contains("copilot-cli ~10.6k in / ~296 out"), "{out}");
    }

    #[test]
    fn a_backend_that_reports_nothing_is_absent_rather_than_zero() {
        // "Does not say" and "spent nothing" are different claims.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "one",
                Some(Outcome::Approved),
            )],
        );
        let out = screen(&mut app, 120, 14);
        assert!(out.contains("no backend on these runs reported"), "{out}");
        assert!(!out.contains(" 0 in "), "{out}");
    }

    #[test]
    fn the_status_line_follows_the_filter() {
        // It totals what is listed, so narrowing the list narrows the total.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![
                with_usage(
                    summary("t1-20260907T000300Z", "keep me", Some(Outcome::Approved)),
                    &[("claude-code", "author", 1_000, Some(100), false)],
                ),
                with_usage(
                    summary("t2-20260907T000200Z", "hide me", Some(Outcome::Approved)),
                    &[("claude-code", "author", 9_000, Some(900), false)],
                ),
            ],
        );
        assert!(screen(&mut app, 120, 14).contains("10.0k in"));
        app.filter = "keep".into();
        app.refilter();
        assert!(screen(&mut app, 120, 14).contains("1.0k in / 100 out"));
    }

    #[test]
    fn a_status_message_takes_the_line_from_the_totals_that_do_not_change() {
        // What just happened is the answer to the key that was just pressed.
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                "one",
                Some(Outcome::Approved),
            )],
        );
        app.status = Some("promoted to promoted/t1 \u{2014} nothing merged".into());
        let out = screen(&mut app, 110, 14);
        assert!(out.contains("nothing merged"), "{out}");
        assert!(
            !out.contains("tokens"),
            "the totals outlived the message:\n{out}"
        );
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
    fn a_long_task_is_wrapped_rather_than_cut_off() {
        // The detail pane is where someone reads what was actually asked. The
        // list already truncates; losing it here as well would lose it entirely.
        let long = "Add a unit test to the adapter profile module asserting that a \
                    profile whose arguments contain the worktree placeholder renders \
                    it as the worktree path and changes nothing else";
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary(
                "t1-20260907T000300Z",
                long,
                Some(Outcome::Approved),
            )],
        );
        let out = screen(&mut app, 100, 20);
        // Asserted on the words the wrap must not lose, one per rendered line,
        // rather than on a phrase that would straddle a break the moment the
        // pane's width changed by a column.
        for word in ["placeholder", "worktree", "changes", "nothing"] {
            assert!(out.contains(word), "{word} was lost:\n{out}");
        }
    }

    #[test]
    fn wrapping_breaks_on_spaces_and_never_loses_a_word() {
        let parts = wrap("one two three four five", 9);
        assert!(parts.iter().all(|p| p.chars().count() <= 9), "{parts:?}");
        assert_eq!(parts.join(" "), "one two three four five");
        // A word longer than the width goes on its own line rather than hanging
        // the loop or being silently dropped.
        assert_eq!(wrap("supercalifragilistic", 5), ["supercalifragilistic"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("anything", 0), ["anything"]);
    }

    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        // Slicing by byte would panic in the middle of a multi-byte character.
        assert_eq!(truncate("ééééé", 3), "éé…");
        assert_eq!(truncate("short", 40), "short");
        assert_eq!(truncate("anything", 0), "anything");
    }

    #[test]
    fn a_terminal_too_short_for_the_chrome_says_so_rather_than_drawing_nothing() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let out = screen(&mut app, 60, 4);
        assert!(out.contains("too short"), "{out}");
    }
}
