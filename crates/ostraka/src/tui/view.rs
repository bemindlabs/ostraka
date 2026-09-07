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

use ostraka_core::gate::Verdict;
use ostraka_core::record::{Event, Outcome, RunRecord};
use ostraka_runtime::index::{self, BackendUsage, RunSummary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::path::PathBuf;

/// Which part of a run the detail pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Checks,
    Events,
    Diff,
}

impl Detail {
    pub fn next(self) -> Self {
        match self {
            Self::Checks => Self::Events,
            Self::Events => Self::Diff,
            Self::Diff => Self::Checks,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Checks => "checks",
            Self::Events => "events",
            Self::Diff => "diff",
        }
    }
}

pub struct App {
    pub project: PathBuf,
    /// Every run, in the order the index gave them.
    pub runs: Vec<RunSummary>,
    /// Indices into `runs` that match the filter. The selection indexes this.
    pub matching: Vec<usize>,
    pub selected: usize,
    pub filter: String,
    pub filtering: bool,
    /// The full record of the selected run, loaded on demand: a listing holds
    /// summaries, and check output is the thing worth reading on a failure.
    pub record: Option<RunRecord>,
    pub events: Vec<Event>,
    /// `None` until asked for; `Some(None)` once asked and not found.
    pub diff: Option<Option<String>>,
    pub detail: Detail,
    pub scroll: u16,
    /// Height of the detail pane at the last draw, so a page key can move by a
    /// page rather than by a number somebody guessed.
    pub page: u16,
    pub status: Option<String>,
    pub quit: bool,
}

impl App {
    pub fn new(project: PathBuf, runs: Vec<RunSummary>) -> Self {
        let matching = (0..runs.len()).collect();
        Self {
            project,
            runs,
            matching,
            selected: 0,
            filter: String::new(),
            filtering: false,
            record: None,
            events: Vec::new(),
            diff: None,
            detail: Detail::Checks,
            scroll: 0,
            page: 10,
            status: None,
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
    }

    /// Everything that describes one run, dropped when a different one is shown.
    pub fn forget_detail(&mut self) {
        self.record = None;
        self.events.clear();
        self.diff = None;
        self.scroll = 0;
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    frame.render_widget(header(app), rows[0]);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);

    render_list(frame, app, columns[0]);
    render_detail(frame, app, columns[1]);
    frame.render_widget(tokens(app), rows[2]);
    frame.render_widget(footer(app), rows[3]);
}

fn header(app: &App) -> Paragraph<'static> {
    let shown = app.matching.len();
    let total = app.runs.len();
    let count = if shown == total {
        format!("{total} run{}", plural(total))
    } else {
        format!("{shown} of {total} runs")
    };
    Paragraph::new(Line::from(vec![
        Span::styled("ostraka", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {}  ·  {count}", app.project.display())),
    ]))
}

/// What each backend has cost, across the runs currently listed.
///
/// Every figure is a vendor's own accounting. A backend that reports nothing
/// does not appear here at all rather than appearing as a zero — "does not say"
/// and "spent nothing" are different claims, and only one of them is true.
fn tokens(app: &App) -> Paragraph<'static> {
    let listed: Vec<RunSummary> = app
        .matching
        .iter()
        .filter_map(|i| app.runs.get(*i))
        .cloned()
        .collect();
    let backends = index::by_backend(&listed);

    if backends.is_empty() {
        return Paragraph::new(Line::from(Span::styled(
            "tokens   no backend on these runs reported what it spent",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let mut spans = vec![Span::styled(
        "tokens   ",
        Style::default().fg(Color::DarkGray),
    )];
    for (i, backend) in backends.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            backend.adapter.clone(),
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::raw(format!(" {}", describe_backend(backend))));
    }
    Paragraph::new(Line::from(spans))
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
        format!("{split} · {about}{} total", compact(backend.total))
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

fn footer(app: &App) -> Paragraph<'static> {
    if app.filtering {
        return Paragraph::new(Line::from(vec![
            Span::styled("filter: ", Style::default().fg(Color::Yellow)),
            Span::raw(app.filter.clone()),
            Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            Span::styled(
                "   enter to keep · esc to clear",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    if let Some(status) = &app.status {
        return Paragraph::new(Line::from(Span::styled(
            status.clone(),
            Style::default().fg(Color::Yellow),
        )));
    }
    let filtered = if app.filter.is_empty() {
        String::new()
    } else {
        format!(" · filter {:?}", app.filter)
    };
    Paragraph::new(Line::from(Span::styled(
        format!(
            "j/k move · tab {} · space/b scroll · / filter · p promote · r reload · q quit{filtered}",
            app.detail.next().title()
        ),
        Style::default().fg(Color::DarkGray),
    )))
}

fn render_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(16) as usize;
    let items: Vec<ListItem> = app
        .matching
        .iter()
        .filter_map(|i| app.runs.get(*i))
        .map(|run| {
            let (mark, colour) = marker(run.outcome);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(colour)),
                Span::styled(when(&run.run_id), Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::raw(truncate(&described(&run.prompt), width)),
            ]))
        })
        .collect();

    let empty = items.is_empty();
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !empty {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let area = inset(area);
    let Some(run) = app.current().cloned() else {
        let message = if app.runs.is_empty() {
            "No runs yet. `ostraka run \"\u{2026}\"` makes one."
        } else {
            "Nothing matches this filter."
        };
        frame.render_widget(Paragraph::new(message), area);
        return;
    };

    app.page = area.height.saturating_sub(1).max(1);

    let mut lines = summary_lines(app, &run, area.width as usize);
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

fn summary_lines(app: &App, run: &RunSummary, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        run.run_id.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    let (_, colour) = marker(run.outcome);
    lines.push(Line::from(Span::styled(
        outcome_word(run.outcome).to_string(),
        Style::default().fg(colour),
    )));
    lines.push(Line::from(""));
    // Wrapped here rather than by the widget: `Paragraph::wrap` would make the
    // number of rendered lines differ from the number of logical ones, and the
    // scroll clamp counts logical lines. A task nobody can read in full is
    // worse than one that takes three rows.
    for (i, part) in wrap(&described(&run.prompt), width.saturating_sub(9))
        .into_iter()
        .enumerate()
    {
        lines.push(if i == 0 {
            field("task", &part)
        } else {
            Line::from(Span::raw(format!("{:<9}{part}", "")))
        });
    }
    if !run.adapter.is_empty() {
        lines.push(field(
            "author",
            &format!("{} ({})", run.author, run.adapter),
        ));
    }
    if let Some(reviewer) = &run.reviewer {
        let verdict = app
            .record
            .as_ref()
            .and_then(|r| r.approval.as_ref())
            .map(|a| describe_verdict(&a.verdict))
            .unwrap_or_default();
        lines.push(field("reviewer", &format!("{reviewer}{verdict}")));
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

    let mut lines = vec![bold("checks".to_string())];
    for check in &record.checks {
        let (word, colour) = if check.passed() {
            ("pass", Color::Green)
        } else {
            ("FAIL", Color::Red)
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(word, Style::default().fg(colour)),
            Span::raw(format!("  {:<10} {}ms", check.name, check.duration_ms)),
        ]));
        // Only for a failure. A passing check's output is noise, and a failing
        // one is the reason someone opened this screen.
        if !check.passed() {
            for line in tail(&check.stderr, 12)
                .into_iter()
                .chain(tail(&check.stdout, 12))
            {
                lines.push(dim(format!("        {line}")));
            }
        }
    }
    lines
}

fn event_lines(app: &App) -> Vec<Line<'static>> {
    if app.events.is_empty() {
        return vec![dim("no events recorded".to_string())];
    }
    let mut lines = vec![bold("events".to_string())];
    for event in &app.events {
        let (kind, text, colour) = match event {
            Event::Message { text, .. } => ("said", text.clone(), Color::Reset),
            Event::ToolUse { name, .. } => ("tool", name.clone(), Color::Cyan),
            Event::Error { message, .. } => ("err ", message.clone(), Color::Red),
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
                Color::DarkGray,
            ),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {kind} "), Style::default().fg(Color::DarkGray)),
            Span::styled(text.replace('\n', " "), Style::default().fg(colour)),
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
        Some(Some(text)) => {
            let mut lines = vec![bold("diff".to_string())];
            for line in text.lines() {
                let colour = match line.as_bytes().first() {
                    Some(b'+') if !line.starts_with("+++") => Color::Green,
                    Some(b'-') if !line.starts_with("---") => Color::Red,
                    Some(b'@') => Color::Cyan,
                    _ => Color::DarkGray,
                };
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(colour),
                )));
            }
            lines
        }
    }
}

fn dim(text: String) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)))
}

fn bold(text: String) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::BOLD),
    ))
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

fn field(name: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<9}"), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

fn describe_verdict(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Approve => " — approve".to_string(),
        Verdict::Reject { reason } => format!(" — reject: {reason}"),
    }
}

fn marker(outcome: Option<Outcome>) -> (char, Color) {
    match outcome {
        Some(Outcome::Approved) => ('+', Color::Green),
        Some(Outcome::Rejected) => ('-', Color::Red),
        Some(Outcome::Failed) => ('!', Color::Magenta),
        None => ('?', Color::DarkGray),
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

/// One column of breathing room on the left of the detail pane.
fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(1),
        ..area
    }
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
        let out = screen(&mut app, 100, 14);
        assert!(out.contains("2 runs"), "{out}");
        assert!(out.contains("add a test"), "{out}");
        assert!(out.contains("rename a field"), "{out}");
        assert!(out.contains("09-07 00:03"), "{out}");
        assert!(out.contains("approved"), "{out}");
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
        let out = screen(&mut app, 100, 16);
        assert!(out.contains("could not reach the model"), "{out}");
        assert!(out.contains("exit 1"), "{out}");
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
        let out = screen(&mut app, 100, 16);
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

        let out = screen(&mut app, 100, 14);
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
    fn the_panes_cycle_and_the_footer_names_the_next_one() {
        let mut app = App::new(
            PathBuf::from("/p"),
            vec![summary("t1-20260907T000300Z", "a", Some(Outcome::Approved))],
        );
        assert!(screen(&mut app, 100, 12).contains("tab events"));
        app.detail = app.detail.next();
        assert!(screen(&mut app, 100, 12).contains("tab diff"));
        app.detail = app.detail.next();
        assert!(screen(&mut app, 100, 12).contains("tab checks"));
        assert_eq!(app.detail.next(), Detail::Checks);
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
        assert!(out.contains("worktree placeholder"), "{out}");
        assert!(out.contains("changes nothing else"), "{out}");
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
}
