//! What the browser shows, as a function of what it knows.
//!
//! Drawing is pure: state in, cells out. That is what lets it be tested against
//! a buffer rather than looked at — a screen nobody asserts on is a screen that
//! quietly stops saying what it used to.

use ostraka_core::gate::Verdict;
use ostraka_core::record::{Event, Outcome, RunRecord};
use ostraka_runtime::index::RunSummary;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::path::PathBuf;

/// Which half of a run the detail pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Checks,
    Events,
}

pub struct App {
    pub project: PathBuf,
    pub runs: Vec<RunSummary>,
    pub selected: usize,
    /// The full record of the selected run, loaded on demand: a listing holds
    /// summaries, and check output is the thing worth reading on a failure.
    pub record: Option<RunRecord>,
    pub events: Vec<Event>,
    pub detail: Detail,
    pub status: Option<String>,
    pub quit: bool,
}

impl App {
    pub fn new(project: PathBuf, runs: Vec<RunSummary>) -> Self {
        Self {
            project,
            runs,
            selected: 0,
            record: None,
            events: Vec::new(),
            detail: Detail::Checks,
            status: None,
            quit: false,
        }
    }

    pub fn current(&self) -> Option<&RunSummary> {
        self.runs.get(self.selected)
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.runs.is_empty() {
            return;
        }
        let last = self.runs.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.record = None;
        self.events.clear();
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
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
    frame.render_widget(footer(app), rows[2]);
}

fn header(app: &App) -> Paragraph<'_> {
    let runs = app.runs.len();
    let plural = if runs == 1 { "run" } else { "runs" };
    Paragraph::new(Line::from(vec![
        Span::styled("ostraka", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {}  ·  {runs} {plural}", app.project.display())),
    ]))
}

fn footer(app: &App) -> Paragraph<'_> {
    match &app.status {
        Some(status) => Paragraph::new(Line::from(Span::styled(
            status.clone(),
            Style::default().fg(Color::Yellow),
        ))),
        None => Paragraph::new(Line::from(Span::styled(
            "j/k move · tab checks|events · p promote · r reload · q quit",
            Style::default().fg(Color::DarkGray),
        ))),
    }
}

fn render_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(16) as usize;
    let items: Vec<ListItem> = app
        .runs
        .iter()
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

    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.runs.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(run) = app.current() else {
        frame.render_widget(
            Paragraph::new("No runs yet. `ostraka run \"…\"` makes one.")
                .block(Block::default().borders(Borders::NONE)),
            area,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        run.run_id.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let (_, colour) = marker(run.outcome);
    lines.push(Line::from(Span::styled(
        outcome_word(run.outcome).to_string(),
        Style::default().fg(colour),
    )));
    lines.push(Line::from(""));
    lines.push(field("task", &described(&run.prompt)));
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

    match app.detail {
        Detail::Checks => lines.extend(check_lines(app, run)),
        Detail::Events => lines.extend(event_lines(app)),
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inset(area),
    );
}

fn check_lines<'a>(app: &'a App, run: &'a RunSummary) -> Vec<Line<'a>> {
    let Some(record) = &app.record else {
        return vec![Line::from(Span::styled(
            format!("checks: {}/{} passed", run.checks_passed, run.checks_total),
            Style::default().fg(Color::DarkGray),
        ))];
    };
    if record.checks.is_empty() {
        return vec![Line::from(Span::styled(
            "no checks ran",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let mut lines = vec![Line::from(Span::styled(
        "checks",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
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
            for line in tail(&check.stderr, 6)
                .into_iter()
                .chain(tail(&check.stdout, 6))
            {
                lines.push(Line::from(Span::styled(
                    format!("        {line}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }
    lines
}

fn event_lines(app: &App) -> Vec<Line<'_>> {
    if app.events.is_empty() {
        return vec![Line::from(Span::styled(
            "no events recorded",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    let mut lines = vec![Line::from(Span::styled(
        "events",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
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

fn field<'a>(name: &'a str, value: &str) -> Line<'a> {
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
        None => "no record — the run did not finish",
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
    format!("{kept}…")
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
    fn a_run_id_that_is_not_shaped_like_one_yields_no_time_rather_than_a_panic() {
        assert_eq!(when("t1-20260907T000300Z"), "09-07 00:03");
        assert_eq!(when("nonsense"), "");
        assert_eq!(when("a-b"), "");
        assert_eq!(when(""), "");
    }

    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        // Slicing by byte would panic in the middle of a multi-byte character.
        assert_eq!(truncate("ééééé", 3), "éé…");
        assert_eq!(truncate("short", 40), "short");
        assert_eq!(truncate("anything", 0), "anything");
    }
}
