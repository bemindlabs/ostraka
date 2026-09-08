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
use crate::tui::theme;
use crate::tui::thread::{Thread, Turn};
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
}

/// One adapter profile, as the agents dialog shows it.
pub struct Agent {
    pub id: String,
    pub ready: bool,
    pub note: String,
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
    /// The work being done here: a chain of runs, each on the one before it.
    pub thread: Thread,
    /// The adapter profiles this project has, probed when first asked for.
    /// Not at startup: probing runs every vendor's binary, and a browser that
    /// took three seconds to open would be a browser nobody left open.
    pub agents: Vec<Agent>,
    pub screen: Screen,
    pub focus: Focus,
    /// The task being written.
    pub prompt: String,
    /// How far back into what has been asked here the box has been walked.
    pub history_at: Option<usize>,
    pub scroll: u16,
    /// Height of the content at the last draw, so a page key can move by a
    /// page rather than by a number somebody guessed.
    pub page: u16,
    /// Whether the transcript sticks to the bottom as the run writes to it.
    /// Off the moment somebody scrolls up, on again at the end.
    pub follow: bool,
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
    /// Ticks of the event loop. What the spinner is a function of, so that
    /// drawing is a function of state rather than of the clock.
    pub tick: u64,
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
            record: None,
            events: Vec::new(),
            diff: None,
            detail: Detail::Checks,
            thread: Thread::default(),
            agents: Vec::new(),
            screen: Screen::Work,
            focus: Focus::Prompt,
            prompt: String::new(),
            history_at: None,
            scroll: 0,
            page: 10,
            follow: true,
            status: None,
            setup: None,
            dialog: None,
            query: String::new(),
            pick: 0,
            leader: false,
            leaving: false,
            tick: 0,
            quit: false,
        }
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
            running: self.thread.running(),
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
        let history = &self.thread.history;
        if history.is_empty() {
            return;
        }
        let at = match (self.history_at, delta) {
            (None, d) if d < 0 => history.len() - 1,
            (None, _) => return,
            (Some(at), d) => match at.checked_add_signed(d) {
                Some(next) if next < history.len() => next,
                // Forward past the newest is back to what was being written.
                Some(_) => {
                    self.history_at = None;
                    self.prompt.clear();
                    return;
                }
                None => 0,
            },
        };
        self.history_at = Some(at);
        self.prompt = history[at].clone();
    }

    /// How many rows the input box wants, border included.
    fn prompt_height(&self) -> u16 {
        let lines = self.prompt.lines().count().clamp(1, PROMPT_LINES);
        lines as u16 + 2
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let box_height = app.prompt_height();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(box_height),
            Constraint::Length(1),
        ])
        .split(screen);

    frame.render_widget(breadcrumb(app, screen.width), theme::inset(rows[0]));
    frame.render_widget(
        Paragraph::new(theme::rule(rows[1].width.saturating_sub(theme::GUTTER))),
        theme::inset(rows[1]),
    );

    let content = theme::inset(rows[2]);
    if app.setup.is_some() {
        render_setup(frame, app, content);
    } else {
        match app.screen {
            Screen::Work => render_work(frame, app, content),
            Screen::Record => render_record(frame, app, content),
        }
    }

    render_prompt(frame, app, rows[3]);
    frame.render_widget(
        Paragraph::new(status_bar(app, rows[4].width.saturating_sub(theme::GUTTER))),
        theme::inset(rows[4]),
    );

    match app.dialog {
        Some(Dialog::Keys) => render_keys(frame, screen),
        Some(Dialog::Commands) => render_palette(frame, app, screen),
        Some(Dialog::Runs) => render_runs(frame, app, screen),
        Some(Dialog::Agents) => render_agents(frame, app, screen),
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
    let room = if app.thread.continuing() {
        width.saturating_sub(38)
    } else {
        width.saturating_sub(2)
    };
    let mut spans = vec![Span::styled(
        truncate(&app.where_shown, room as usize),
        theme::muted(),
    )];
    // What the next run will stand on, where that is not simply `HEAD`. It
    // belongs beside the path because it is the same kind of fact: where you
    // are working.
    if app.thread.continuing() {
        spans.push(Span::styled("   on ", theme::muted()));
        spans.push(Span::styled(
            truncate(&app.thread.base_ref, 28),
            theme::accent(),
        ));
    }
    Paragraph::new(Line::from(spans))
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
    if app.thread.is_empty() {
        let empty = vec![
            dim("Nothing has been asked here yet.".to_string()),
            Line::from(""),
            Line::from(vec![
                Span::styled("Write a task below and press ", theme::muted()),
                Span::styled("enter", theme::accent()),
                Span::styled(". Each one is isolated, gated and", theme::muted()),
            ]),
            dim("reviewed by a different agent than the one that wrote it.".to_string()),
        ];
        frame.render_widget(Paragraph::new(empty), area);
        return;
    }

    let lines = thread_lines(&app.thread, area.width, app.tick);
    let overflow = lines.len().saturating_sub(area.height as usize) as u16;
    if app.follow {
        app.scroll = overflow;
    }
    app.scroll = app.scroll.min(overflow);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
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

    let text: Vec<Line<'static>> = if !writing && app.prompt.is_empty() {
        vec![Line::from(Span::styled(
            "n  write a task     l  runs     ctrl-k  commands     ?  keys",
            theme::muted(),
        ))]
    } else {
        let mut lines: Vec<Line<'static>> = app
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
            if app.prompt.is_empty() {
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

    let left = match app.thread.live.as_ref().filter(|s| s.live()) {
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
    frame.render_widget(Paragraph::new(lines), area);
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
        ("s", "ask a running agent to stop"),
        ("l", "the runs recorded here"),
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
        Paragraph::new(lines).block(theme::panel(true).padding(Padding::horizontal(2))),
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
        Paragraph::new(lines).block(theme::panel(true).padding(Padding::horizontal(2))),
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
    let (author, author_colour) = named(&app.thread.adapter);
    let (reviewer, reviewer_colour) = named(&app.thread.review_adapter);

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
        lines.push(dim("no adapter profiles in adapters/".to_string()));
    }
    for (i, agent) in app.agents.iter().enumerate() {
        let here = i == app.pick;
        let mut role = String::new();
        if app.thread.adapter.as_deref() == Some(agent.id.as_str()) {
            role.push_str("writes ");
        }
        if app.thread.review_adapter.as_deref() == Some(agent.id.as_str()) {
            role.push_str("reviews");
        }
        let (mark, colour) = if agent.ready {
            (theme::PASSED, theme::OK)
        } else {
            (theme::FAILED, theme::BAD)
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

    fn record(run_id: &str, prompt: &str, checks: Vec<CheckRecord>) -> RunRecord {
        RunRecord {
            run_id: run_id.into(),
            task_id: "t1".into(),
            prompt: prompt.into(),
            author: ActorId::new("archon"),
            adapter: "claude-code".into(),
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
        app.thread.turns.push(Turn {
            prompt: prompt.to_string(),
            steps,
            finished: ended,
            failed: None,
        });
    }

    fn working(app: &mut App, prompt: &str, steps: Vec<Step>) {
        app.thread.live = Some(Session::recorded(prompt, steps, None));
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
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").expect("write");

        let mut app = App::new(dir.clone(), Vec::new());
        app.setup = Some(crate::init::plan(&dir));

        let out = screen(&mut app, 100, 24);
        assert!(out.contains("not an Ostraka project yet"), "{out}");
        assert!(out.contains("a Rust project"), "{out}");
        assert!(out.contains("adapters/codex.toml"), "{out}");
        assert!(out.contains("i set this directory up"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_names_what_is_already_there_rather_than_offering_to_rewrite_it() {
        let dir = std::env::temp_dir().join(format!("ostraka-view-half-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("ostraka.toml"), "# mine\n").expect("write");

        let mut app = App::new(dir.clone(), Vec::new());
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
    fn an_empty_thread_says_what_to_do_rather_than_showing_nothing() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let out = screen(&mut app, 100, 18);
        assert!(out.contains("Nothing has been asked here yet"), "{out}");
        assert!(out.contains("reviewed by a different agent"), "{out}");
    }

    #[test]
    fn the_regions_are_divided_by_rules_rather_than_by_space_alone() {
        // Four regions separated only by gaps read as one region with holes in
        // it, and the eye re-derives the boundaries every time it looks.
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.thread.turns.push(Turn {
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let mut steps = vec![Step::Entered(Phase::Authoring)];
        steps.extend((0..60).map(|i| said(Phase::Authoring, &format!("line {i}"))));
        working(&mut app, "a talkative agent", steps);

        let following = screen(&mut app, 100, 20);
        assert!(
            following.contains("line 59"),
            "the tail was not shown:\n{following}"
        );

        app.scroll_by(-30);
        assert!(!app.follow, "scrolling up did not release the tail");
        let held = screen(&mut app, 100, 20);
        assert!(!held.contains("line 59"), "the view snapped back:\n{held}");
    }

    #[test]
    fn the_breadcrumb_says_what_the_next_run_will_stand_on() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        // Nothing to say while the chain is still standing on HEAD.
        assert!(!screen(&mut app, 100, 18).contains("ostraka/"));

        app.thread.base_ref = "ostraka/t1-20260908T000100Z".into();
        let out = screen(&mut app, 100, 18);
        let breadcrumb = out.lines().next().unwrap_or_default();
        assert!(breadcrumb.contains(" on "), "{out}");
        assert!(breadcrumb.contains("ostraka/t1-"), "{out}");
    }

    #[test]
    fn the_box_says_where_typing_goes() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        let one = app.prompt_height();
        app.prompt = "first\nsecond\nthird".into();
        assert!(app.prompt_height() > one);

        let out = screen(&mut app, 100, 20);
        assert!(out.contains("first"), "{out}");
        assert!(out.contains("third"), "{out}");
    }

    #[test]
    fn the_runs_dialog_lists_what_was_run_and_narrows_as_it_is_typed_into() {
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
            PathBuf::from("/p"),
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.agents = vec![
            Agent {
                id: "claude-code".into(),
                ready: true,
                note: "2.1.263".into(),
            },
            Agent {
                id: "codex".into(),
                ready: false,
                note: "codex not found on PATH".into(),
            },
        ];
        app.open(Dialog::Agents);

        let out = screen(&mut app, 110, 24);
        assert!(out.contains("automatic"), "{out}");
        assert!(out.contains("claude-code"), "{out}");
        assert!(out.contains("not found on PATH"), "{out}");

        app.thread.adapter = Some("claude-code".into());
        let chosen = screen(&mut app, 110, 24);
        assert!(chosen.contains("writes"), "{chosen}");
    }

    #[test]
    fn a_record_shows_the_failing_check_that_explains_it() {
        // The reason someone looks a run up. A passing check's output is
        // noise; a failing one's is the whole point.
        let mut app = App::new(
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
            PathBuf::from("/p"),
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        working(&mut app, "do a thing", vec![Step::Entered(Phase::Gating)]);
        let out = screen(&mut app, 110, 18);
        assert!(out.contains("running"), "{out}");
        assert!(out.contains("gate"), "{out}");
        assert!(out.contains("41s"), "{out}");

        app.thread.live.as_mut().expect("a session").stopping = true;
        assert!(screen(&mut app, 110, 18).contains("stopping"));
    }

    #[test]
    fn a_status_message_takes_the_line_from_the_totals_that_do_not_change() {
        let mut app = App::new(
            PathBuf::from("/p"),
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.leader = true;
        let out = screen(&mut app, 120, 16);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("write a task"), "{out}");
    }

    #[test]
    fn the_keys_dialog_lists_the_keys_rather_than_a_footer_doing_it_forever() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        app.open(Dialog::Keys);
        let out = screen(&mut app, 110, 26);
        assert!(out.contains("ctrl-x"), "{out}");
        assert!(out.contains("alt-enter"), "{out}");
        assert!(out.contains("esc closes this"), "{out}");
    }

    #[test]
    fn the_command_palette_names_what_it_can_do_and_marks_the_pick() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
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
    fn a_terminal_too_short_for_the_chrome_says_so_rather_than_drawing_nothing() {
        let mut app = App::new(PathBuf::from("/p"), Vec::new());
        assert!(screen(&mut app, 60, 4).contains("too short"));
    }
}
