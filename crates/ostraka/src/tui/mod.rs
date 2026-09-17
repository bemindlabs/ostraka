//! `ostraka tui` — read the runs back.
//!
//! A browser over records that already existed. It holds no logic of its own:
//! the listing comes from `runtime::index`, the detail from `orchestrator::
//! replay`, and promotion goes through `promote::promote` like every other
//! caller — so the gate cannot be bypassed by pressing a key.
//!
//! Three ways reach the same seven actions: the bare key, the leader key, and
//! the palette. That is not three implementations — `command::Command` is the
//! list, and all three dispatch into `perform`, so an action cannot work one
//! way and be broken another.

mod command;
#[cfg(test)]
mod flows;
mod mention;
mod pane;
mod select;
mod session;
mod theme;
mod thread;
mod view;

use crate::chord::{self, Action as Chord, label};
use crate::init;
use crate::mode::Mode;
use crate::workspace::Workspace;
use command::Command;
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{index, orchestrator};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use view::{Agent, App, Detail, Dialog, Focus, Screen};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// How long a keypress waits before the loop looks around again.
///
/// Long enough that an idle browser is not a busy loop, short enough that a
/// keypress feels immediate.
const TICK: Duration = Duration::from_millis(250);

pub fn run(workspace: &Workspace) -> Outcome {
    // Said plainly, before anything is set up. Piped into `head`, the terminal
    // library's own failure is a panic and a backtrace naming a file inside a
    // dependency, which tells the operator nothing they can act on.
    if !std::io::stdout().is_terminal() {
        return Err("`ostraka tui` needs a terminal. For a pipe or a log, \
                    `ostraka runs` prints the same listing, and `--json` \
                    prints it for a machine."
            .into());
    }

    // Read before anything is drawn, and refused by name if it does not parse:
    // a browser that quietly fell back to other keys would be one whose chords
    // do not do what its owner wrote down.
    // Loaded again only to refuse a file that does not parse. When it does,
    // `main` has already installed the same bindings.
    chord::install(chord::Keys::load()?);

    let records_root = workspace.records();
    let mut app = open(workspace, &records_root)?;

    // Installs a panic hook that restores the terminal first. Without it a
    // panic leaves the operator staring at a shell with no echo and no prompt.
    let mut terminal = ratatui::try_init()?;
    // After ratatui's hook, so a panic gives the keyboard and the mouse back
    // before the terminal is restored, rather than leaving a shell that prints
    // an escape sequence for every key and every movement of the mouse.
    let restore = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        release_input();
        restore(info);
    }));
    capture_input();
    let result = event_loop(&mut terminal, &mut app, &records_root);
    release_input();
    ratatui::restore();
    result?;

    Ok(true)
}

/// The adapter profiles this project has, and whether each one can run.
///
/// Probing runs every vendor's binary, so it happens when somebody asks to
/// see the list rather than when the browser opens.
/// What this browser can offer to write with, configured or merely installed.
///
/// A workspace with no `adapters/` used to answer this with an empty list,
/// which reads as "there are no agents" and is never what is true — the CLIs
/// this binary knows how to drive are usually sitting on the PATH already. The
/// command line stopped saying that; the browser was still saying it.
fn agents(workspace: &Workspace) -> Vec<Agent> {
    use ostraka_adapter::{VendorAdapter, process::ProcessAdapter};
    let profiles = workspace.profiles().unwrap_or_default();
    let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();

    let mut agents: Vec<Agent> = profiles
        .into_iter()
        .map(|profile| {
            let adapter = ProcessAdapter::new(profile);
            let availability = adapter.probe();
            Agent {
                id: adapter.id().to_string(),
                ready: availability.is_ready(),
                note: crate::discover::describe(&availability),
                configured: true,
            }
        })
        .collect();
    agents.sort_by(|a, b| a.id.cmp(&b.id));

    // Installed and unwritten, after the configured ones: what the workspace
    // agreed to comes first, and what it could agree to follows. Only the ones
    // that answered — an absent CLI offered as a choice is a choice that fails
    // when it is taken.
    agents.extend(
        crate::discover::unconfigured(&ids)
            .into_iter()
            .filter(|f| f.ready())
            .map(|f| Agent {
                id: f.id,
                ready: true,
                note: crate::discover::describe(&f.availability),
                configured: false,
            }),
    );
    agents
}

/// The browser as it is when it opens.
///
/// Separated from `run` so the flow tests start where an operator starts,
/// rather than from an `App` assembled by hand that could drift from this one.
fn open(workspace: &Workspace, records_root: &Path) -> Result<App, Box<dyn std::error::Error>> {
    let mut app = App::new(workspace.clone(), index::list(records_root)?);
    // The one there is, where there is one. A workspace with several waits to
    // be told which, because picking would be picking.
    app.pane_mut().repository = workspace.repository(None).ok();
    // Opened somewhere that is not a project yet: say what is missing and offer
    // to write it, rather than showing an empty list that looks like a bug.
    //
    // `runnable`, not `complete`: the browser asks whether this directory can
    // be run, and `init` asks whether it has anything left to write. Those are
    // different questions, and asking the second one put "not an Ostraka
    // project yet" across a screen with three recorded runs behind it.
    app.setup = Some(init::plan(&workspace.root)).filter(|plan| !plan.runnable());
    // Asked once, here, because the screen that offers to set this directory
    // up is the one that has to say what setting it up will not fix.
    app.blocked = blocking(&app).map(|r| r.problem);
    load_detail(&mut app, records_root);
    Ok(app)
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    records_root: &Path,
) -> std::io::Result<()> {
    while !app.quit {
        // The tick is what the spinner turns on. Drawing stays a function of
        // state that way, and a screen nobody can assert on is a screen that
        // quietly stops saying what it used to.
        app.tick = app.tick.wrapping_add(1);
        take_stock(app, records_root);
        terminal.draw(|frame| view::draw(frame, app))?;
        if !event::poll(TICK)? {
            continue;
        }
        match event::read()? {
            TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                handle(app, key, records_root)
            }
            TermEvent::Mouse(event) => mouse(app, event),
            _ => {}
        }
    }

    // Never walk away from a run. Leaving here with a vendor still writing
    // into a worktree is the thing Ctrl-C was taught to prevent, and closing a
    // window is not a better reason to do it than pressing a key was.
    for pane in &mut app.panes {
        pane.thread.settle_worker();
    }
    Ok(())
}

/// Asks the terminal for what the browser reads besides plain keys: command as
/// a modifier where that is the chord, and the mouse, so a click can place the
/// cursor in the box.
fn capture_input() {
    chord::enter();
    let _ = ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture);
}

/// Gives back what [`capture_input`] asked for.
fn release_input() {
    let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    chord::leave();
}

/// The mouse. A click in the box puts the cursor where it landed, a drag over
/// a transcript selects it, and the wheel scrolls what is being read.
///
/// The wheel is here because the mouse is captured: a terminal that reports it
/// to a program no longer scrolls by itself, and many turned the wheel into
/// arrow keys in a full-screen program, where the up arrow walks history. The
/// selection is here for the other half of the same cost — a terminal that
/// reports the mouse stops selecting on a drag.
fn mouse(app: &mut App, event: MouseEvent) {
    if app.dialog.is_some() {
        return;
    }
    let column = app
        .columns
        .iter()
        .find(|(_, area)| area.contains(Position::new(event.column, event.row)))
        .map(|(index, _)| *index);
    // The transcript under the pointer, if it is over one. Asked first,
    // because a press there starts a selection as well as focusing the pane.
    let over = app
        .transcripts
        .iter()
        .find(|(_, area, _)| area.contains(Position::new(event.column, event.row)))
        .copied();
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) if over.is_some() => {
            let (index, area, scroll) = over.unwrap_or_default();
            app.focus_pane(index);
            app.focus = Focus::Prompt;
            let at = spot(app, index, area, scroll, event);
            app.selection = Some(select::Selection::new(index, at));
        }
        // Extending the selection the press started. Read against the pane it
        // began in, not the one the pointer is over: a drag that wandered into
        // the next column would otherwise select across two runs, which is the
        // thing the terminal's own selection gets wrong here.
        MouseEventKind::Drag(MouseButton::Left) => {
            let Some(mut selection) = app.selection else {
                return;
            };
            if !selection.dragging {
                return;
            }
            let Some((index, area, scroll)) = app
                .transcripts
                .iter()
                .find(|(at, _, _)| *at == selection.pane)
                .copied()
            else {
                return;
            };
            // Dragging past the top or bottom edge scrolls, so a selection can
            // reach what is not on the screen without letting go of it.
            //
            // The scroll this is read against moves with it. Reading the head
            // against the scroll captured before scrolling left the selection
            // a line behind the edge it was being dragged past, so it never
            // reached the line that had just been revealed — which is the only
            // line that drag was for. Raised in review.
            let mut scroll = scroll;
            if event.row < area.y {
                app.scroll_pane(index, -1);
                scroll = scroll.saturating_sub(1);
            } else if event.row >= area.y.saturating_add(area.height) {
                app.scroll_pane(index, 1);
                scroll = scroll.saturating_add(1);
            }
            selection.head = spot(app, index, area, scroll, event);
            app.selection = Some(selection);
        }
        MouseEventKind::Up(MouseButton::Left) => match app.selection {
            // A click is a drag of no distance, and it means "put the keys in
            // this pane" rather than "select nothing".
            Some(selection) if selection.is_empty() => app.selection = None,
            Some(mut selection) => {
                selection.dragging = false;
                app.selection = Some(selection);
                app.status = Some(format!(
                    "selected \u{2014} {} {} copies it",
                    chord::label(chord::Action::Leader),
                    Command::Copy.leader()
                ));
            }
            None => {}
        },
        // A click on a column gives that pane the keys, and nothing else: the
        // task somebody was writing in it is still there to go on with.
        MouseEventKind::Down(MouseButton::Left) if column.is_some() => {
            app.focus_pane(column.unwrap_or(app.at));
            app.focus = Focus::Prompt;
        }
        // The wheel scrolls the column under it, which may not be the one in
        // front.
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            if column.is_some_and(|index| index != app.at) =>
        {
            let up = matches!(event.kind, MouseEventKind::ScrollUp);
            let pane = &mut app.panes[column.unwrap_or(app.at)];
            pane.scroll = pane.scroll.saturating_add_signed(if up { -3 } else { 3 });
            if up {
                pane.follow = false;
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let area = app.prompt_text;
            if !area.contains(Position::new(event.column, event.row)) {
                return;
            }
            let row = usize::from(event.row - area.y) + usize::from(app.prompt_scroll);
            app.focus = Focus::Prompt;
            app.pane_mut().place(row, event.column - area.x);
        }
        MouseEventKind::ScrollUp => app.scroll_by(-3),
        MouseEventKind::ScrollDown => app.scroll_by(3),
        _ => {}
    }
}

/// Where in a transcript a mouse event landed.
///
/// A screen row is a line only together with the scroll the transcript was
/// drawn at, and a screen cell is a display column only after the line has
/// been consulted about how wide its characters are.
fn spot(
    app: &App,
    index: usize,
    area: ratatui::layout::Rect,
    scroll: u16,
    event: MouseEvent,
) -> select::Spot {
    // Held to the rows the transcript was drawn on. A drag goes where the
    // pointer goes, including off the top and bottom of it, and a row outside
    // the area is a row that was never part of this transcript — the line it
    // wants is the first or last one that is.
    let last = area.y.saturating_add(area.height.saturating_sub(1));
    let row = event.row.clamp(area.y, last.max(area.y));
    let at = usize::from(row - area.y) + usize::from(scroll);
    let lines = view::transcript_of(app, index);
    let line = at.min(lines.len().saturating_sub(1));
    select::Spot {
        line,
        column: select::column_at(lines.get(line), event.column.saturating_sub(area.x)),
    }
}

/// Puts what is selected in a pane on the clipboard.
///
/// What is said afterwards is deliberately "asked the terminal" rather than
/// "copied". OSC 52 has no reply: a terminal that does not do it says nothing,
/// and Terminal.app and VTE are two that do not. Claiming success we cannot
/// observe is how somebody pastes the thing they copied ten minutes ago and
/// does not know why.
fn copy_selection(app: &mut App) {
    let Some(selection) = app.selection else {
        app.status = Some("nothing is selected \u{2014} drag over a transcript first".to_string());
        return;
    };
    let lines = view::transcript_of(app, selection.pane);
    let text = select::text(&lines, &selection);
    if text.is_empty() {
        app.status = Some("the selection is empty".to_string());
        return;
    }
    app.status = Some(match select::to_clipboard(&text) {
        Ok(()) => format!(
            "asked the terminal for the clipboard \u{2014} {} line(s), {} character(s)",
            text.lines().count(),
            text.chars().count()
        ),
        Err(e) => format!("could not copy: {e}"),
    });
}

/// Takes whatever the run has said since the last frame.
///
/// Called before drawing rather than after a key, because a run says things
/// while nobody is pressing anything — which is most of the time it takes.
fn take_stock(app: &mut App, records_root: &Path) {
    // Every pane, not the one on screen. A run keeps going in a pane somebody
    // has switched away from, and a transcript that stopped updating because
    // nobody was looking at it would be a transcript that lied about where the
    // run got to.
    let mut ended = None;
    for at in 0..app.panes.len() {
        if let Some(run_id) = app.panes[at].thread.settle() {
            ended = Some((at, run_id));
        }
    }
    if let Some((at, run_id)) = ended {
        finished_run(app, records_root, at, Some(run_id));
    }
    // A vendor that could not run, in the pane in front, offered another
    // profile. Never over a dialog somebody has open: the offer waits on the
    // thread until the screen is free, or until that pane is in front.
    if app.dialog.is_none() {
        if let Some(offer) = app.pane_mut().thread.offer.take() {
            offer_fallback(app, offer);
        }
    }
    if let Some(rx) = app.agents_loading.take() {
        match rx.try_recv() {
            Ok(found) => app.agents = found,
            Err(std::sync::mpsc::TryRecvError::Empty) => app.agents_loading = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }
    // The model listing, once every profile has answered.
    if let Some(rx) = app.models_loading.take() {
        match rx.try_recv() {
            Ok(catalogs) => {
                app.models_notes = catalogs
                    .iter()
                    .filter_map(|c| c.note.as_ref().map(|n| format!("{}: {n}", c.profile)))
                    .collect();
                app.models = crate::models::rows(&catalogs);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => app.models_loading = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.models_notes = vec!["the model listing stopped without an answer".to_string()];
            }
        }
    }
    // Asked to leave while something was running: the browser stays up until
    // the run it started has actually stopped, so the last thing on screen is
    // what happened rather than a terminal that froze on its way out.
    if app.leaving && !app.anything_running() {
        app.quit = true;
    }
}

/// What to do about a run that has just ended.
fn finished_run(app: &mut App, records_root: &Path, at: usize, run_id: Option<String>) {
    let last = app.panes[at].thread.turns.last();
    let said = last.and_then(|turn| {
        turn.finished
            .as_ref()
            .map(|f| f.summary.clone())
            .or_else(|| turn.failed.clone())
    });
    // Named where it happened, because it may not have been where you were.
    app.status = match (said, app.panes.len() > 1 && at != app.at) {
        (Some(said), true) => Some(format!("{}: {said}", app.panes[at].title())),
        (said, _) => said,
    };
    app.panes[at].follow = true;

    // The record exists now, so the listing can show it.
    if let Ok(runs) = index::list(records_root) {
        app.runs = runs;
        app.refilter();
    }
    if let Some(id) = run_id {
        if let Some(at) = app
            .matching
            .iter()
            .position(|i| app.runs.get(*i).is_some_and(|run| run.run_id == id))
        {
            // Selected, so that looking the run up afterwards lands on it
            // rather than on whatever was selected before it started.
            app.selected = at;
            app.forget_detail();
            load_detail(app, records_root);
        }
    }
}

fn handle(app: &mut App, key: KeyEvent, records_root: &Path) {
    // In the order a keystroke has to be read. The chords come first because
    // they are the way out of anywhere: a box with the keys must not be able to
    // swallow the key that opens the commands. Then a half-finished chord, then
    // what is open over the screen, then whoever has the keyboard.
    // Leaving is control-c everywhere and cannot be rebound, because command-c
    // is copy and a way out has to be the same way out on every terminal.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return leave(app);
    }
    // The chords, as `keys.toml` or the defaults bind them. Panes are switched
    // between while the box has the keys, so those are chords, not letters.
    match chord::action(&key) {
        Some(Chord::Commands) => return app.open(Dialog::Commands),
        Some(Chord::Leader) => {
            app.leader = true;
            return;
        }
        Some(Chord::NewPane) => return perform(app, Command::NewPane, records_root),
        Some(Chord::NextPane) => return perform(app, Command::NextPane, records_root),
        Some(Chord::PreviousPane) => {
            app.next_pane(-1);
            to_work(app);
            return;
        }
        None => {}
    }

    // The leader before the dialog, because it was armed after the dialog
    // opened. The other way round, a dialog swallowed the letter that was
    // meant to complete the chord and the leader stayed armed for ever.
    if app.leader {
        app.leader = false;
        if let KeyCode::Char(c) = key.code {
            // Looked up in the whole list rather than in what is on offer,
            // so a letter that names a command nobody can use right now says
            // why instead of doing nothing. A key that silently does nothing
            // is a key somebody presses twice.
            if let Some(command) = Command::ALL
                .into_iter()
                .find(|command| command.leader() == c)
            {
                perform(app, command, records_root);
            }
        }
        return;
    }
    if app.dialog.is_some() {
        dialog_key(app, key, records_root);
        return;
    }

    // Any keypress supersedes the last message. A status line that outlives
    // what it described is worse than no status line.
    app.status = None;

    // A selection is made with the mouse and dropped with the keyboard, and
    // escape is the key for "never mind" everywhere else on this screen. It
    // drops the selection before anything else escape does, because a
    // highlight left over a transcript somebody has moved on from is a
    // highlight they have to work out how to get rid of. Any other key leaves
    // it alone: typing a task while some of the transcript is selected is an
    // ordinary thing to be doing, and it is what copying is usually for.
    if key.code == KeyCode::Esc && app.selection.is_some() {
        app.selection = None;
        return;
    }

    if app.focus == Focus::Prompt {
        prompt_key(app, key, records_root);
    } else {
        command_key(app, key, records_root);
    }
}

/// Keys while the box has them, which on the work screen is most of the time.
///
/// The whole point of this browser is that you say what you want, so what you
/// type is the task. Everything else is a chord or is behind escape.
fn prompt_key(app: &mut App, key: KeyEvent, records_root: &Path) {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // A task worth writing sometimes takes a paragraph, and a box that
        // could not hold one would push the work back out to the shell.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => app.pane_mut().insert('\n'),
        KeyCode::Char('j') if control => app.pane_mut().insert('\n'),
        // A mode is not a command, and typing its name is how somebody who
        // knows it reaches for it. Read before the commands, because "ask" is
        // inside "write a task" and would otherwise start writing one.
        KeyCode::Enter if app.slashing() && Mode::named(&app.pane().prompt).is_some() => {
            let mode = Mode::named(&app.pane().prompt).unwrap_or_default();
            app.pane_mut().replace(String::new());
            app.pick = 0;
            set_mode(app, mode);
        }
        KeyCode::BackTab => {
            let next = app.thread().mode.next();
            set_mode(app, next);
        }
        KeyCode::Enter if app.slashing() => {
            let picked = app.slash_picked();
            app.pane_mut().replace(String::new());
            app.pick = 0;
            match picked {
                Some(command) => perform(app, command, records_root),
                None => app.status = Some("no command by that name".to_string()),
            }
        }
        KeyCode::Up if app.slashing() => app.pick = app.pick.saturating_sub(1),
        KeyCode::Down if app.slashing() => {
            app.pick = (app.pick + 1).min(app.slash_matches().len().saturating_sub(1));
        }
        // An `@` being written: tab completes it and the arrows pick. Enter
        // completes too, while what is picked is not already what is written,
        // because a half-typed path is not a task anybody meant to send.
        KeyCode::Tab if app.mentioning().is_some() => {
            app.complete_mention();
        }
        KeyCode::Up if !app.mention_matches().is_empty() => {
            app.pick = app.pick.saturating_sub(1);
        }
        KeyCode::Down if !app.mention_matches().is_empty() => {
            app.pick = (app.pick + 1).min(app.mention_matches().len().saturating_sub(1));
        }
        KeyCode::Enter
            if app
                .mention_picked()
                .is_some_and(|picked| Some(picked.text) != app.mentioning()) =>
        {
            app.complete_mention();
        }
        KeyCode::Enter => start_run(app),
        // The task is kept. It took thought to write, and coming back to it
        // after looking something up is the normal way of working.
        KeyCode::Esc => app.focus = Focus::Keys,
        KeyCode::Backspace => {
            app.pane_mut().backspace();
            app.pane_mut().history_at = None;
            app.pick = 0;
        }
        // Walking into another word is walking away from whatever was picked
        // in the last one.
        KeyCode::Left => {
            app.pane_mut().left();
            app.pick = 0;
        }
        KeyCode::Right => {
            app.pane_mut().right();
            app.pick = 0;
        }
        KeyCode::Up => app.recall(-1),
        KeyCode::Down => app.recall(1),
        KeyCode::PageUp => app.scroll_by(-(app.page as i16)),
        KeyCode::PageDown => app.scroll_by(app.page as i16),
        KeyCode::Char(c) => {
            app.pane_mut().insert(c);
            app.pane_mut().history_at = None;
            app.pick = 0;
        }
        _ => {}
    }
    // Read as a mention starts, so the menu drawn after this key has something
    // to offer.
    if app.mentioning().is_some() {
        app.load_mentionable();
    }
}

/// Keys when the box does not have them.
fn command_key(app: &mut App, key: KeyEvent, records_root: &Path) {
    let page = app.page as i16;
    match key.code {
        KeyCode::Char('?') => app.open(Dialog::Keys),
        KeyCode::Char('q') => leave(app),
        // Escape gives back whatever it can before it gives up the browser: a
        // record being read, then a filter, and only then the screen.
        KeyCode::Esc => {
            if app.screen == Screen::Record {
                to_work(app);
            } else if !app.filter.is_empty() {
                app.filter.clear();
                app.refilter();
            } else {
                leave(app);
            }
        }
        KeyCode::Char('n') | KeyCode::Enter => perform(app, Command::NewRun, records_root),
        KeyCode::Char('s') => perform(app, Command::Stop, records_root),
        KeyCode::Char('l') => perform(app, Command::Runs, records_root),
        KeyCode::Char('w') => perform(app, Command::Repos, records_root),
        KeyCode::Char('a') => perform(app, Command::Agents, records_root),
        KeyCode::Char('i') => perform(app, Command::Setup, records_root),
        KeyCode::Char('r') => perform(app, Command::Reload, records_root),
        KeyCode::Char('p') => perform(app, Command::Promote, records_root),
        KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
            perform(app, Command::NextDetail, records_root)
        }
        KeyCode::Char('t') => perform(app, Command::NewPane, records_root),
        KeyCode::Char(']') => perform(app, Command::NextPane, records_root),
        KeyCode::Char('X') => perform(app, Command::ClosePane, records_root),
        KeyCode::Char('<') => perform(app, Command::MovePaneLeft, records_root),
        KeyCode::Char('>') => perform(app, Command::MovePaneRight, records_root),
        KeyCode::Char('+') => perform(app, Command::WiderPane, records_root),
        KeyCode::Char('-') => perform(app, Command::NarrowerPane, records_root),
        KeyCode::Char('=') => perform(app, Command::EvenPanes, records_root),
        KeyCode::Char('m') | KeyCode::BackTab => perform(app, Command::Mode, records_root),
        KeyCode::Char('o') => perform(app, Command::Models, records_root),
        KeyCode::Char('j') | KeyCode::Down => {
            app.move_by(1);
            load_detail(app, records_root);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.move_by(-1);
            load_detail(app, records_root);
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.move_by(-(app.matching.len() as isize));
            load_detail(app, records_root);
        }
        KeyCode::Char('G') | KeyCode::End => {
            app.pane_mut().follow = true;
            app.move_by(app.matching.len() as isize);
            load_detail(app, records_root);
        }
        KeyCode::Char(' ') | KeyCode::PageDown => app.scroll_by(page),
        KeyCode::Char('b') | KeyCode::PageUp => app.scroll_by(-page),
        _ => {}
    }
}

/// Back to the work, with the box.
fn to_work(app: &mut App) {
    app.screen = Screen::Work;
    app.focus = Focus::Prompt;
    app.pane_mut().follow = true;
    app.scroll = 0;
}

/// Every action the browser has, in one place.
///
/// The bare key, the leader and the palette all arrive here, which is what
/// keeps them from drifting into three slightly different versions of promote.
fn perform(app: &mut App, command: Command, records_root: &Path) {
    app.status = None;

    // The list is the authority for the bare keys too, not only for the two
    // routes that read it to draw themselves. Without this, `n` during a run
    // would open the box the palette had already decided not to offer, and
    // enter would replace the session — abandoning the thread still writing
    // into a worktree, which is the one thing this browser must not do.
    if !Command::offered(app.situation()).contains(&command) {
        app.status = Some(unavailable(app, command));
        return;
    }

    match command {
        Command::NewRun => {
            app.screen = Screen::Work;
            app.focus = Focus::Prompt;
        }
        Command::Stop => {
            app.thread_mut().stop();
            app.status = Some("asked the agent to stop".to_string());
        }
        Command::Copy => copy_selection(app),
        Command::Fix => open_fix(app),
        Command::Settings => {
            app.project_facts = project_facts(app);
            app.editing = None;
            app.open(Dialog::Settings);
        }
        Command::Runs => app.open(Dialog::Runs),
        Command::Repos => {
            app.open(Dialog::Repos);
            app.editing = None;
            app.pick = app
                .workspace
                .repositories()
                .iter()
                .position(|r| Some(&r.name) == app.repository().map(|c| &c.name))
                .unwrap_or(0);
        }
        Command::Prune => {
            // The list is taken now and held, so the keystroke that removes
            // acts on what was actually shown rather than on whatever the disk
            // looks like a moment later.
            match crate::prune::leftovers(&app.workspace) {
                Ok(found) => {
                    app.leftovers = found;
                    app.open(Dialog::Prune);
                }
                Err(e) => app.status = Some(format!("could not look: {e}")),
            }
        }
        Command::Agents => {
            app.agents = agents(&app.workspace);
            app.open(Dialog::Agents);
        }
        Command::Fresh => {
            *app.thread_mut() = thread::Thread::default();
            app.status = Some("a fresh thread \u{2014} the next run starts from HEAD".to_string());
            to_work(app);
        }
        Command::NextDetail => {
            app.detail = app.detail.next();
            app.scroll = 0;
            load_detail(app, records_root);
        }
        Command::NewPane => {
            app.open_pane();
            to_work(app);
            app.status = Some(format!("pane {} of {}", app.at + 1, app.panes.len()));
        }
        Command::NextPane => {
            app.next_pane(1);
            to_work(app);
        }
        Command::ClosePane => {
            if let Some(why) = app.close_pane() {
                app.status = Some(why);
            } else {
                to_work(app);
            }
        }
        Command::MovePaneLeft | Command::MovePaneRight => {
            let left = command == Command::MovePaneLeft;
            if !app.move_pane(if left { -1 } else { 1 }) {
                let side = if left { "left" } else { "right" };
                app.status = Some(format!("this pane is already the {side}most"));
            }
        }
        Command::WiderPane | Command::NarrowerPane => {
            let wider = command == Command::WiderPane;
            if !app.resize_pane(if wider { 1 } else { -1 }) {
                let limit = if wider { "wide" } else { "narrow" };
                app.status = Some(format!("this pane is as {limit} as it goes"));
            }
        }
        Command::EvenPanes => app.even_panes(),
        Command::Promote => app.status = Some(promote_selected(app)),
        Command::Reload => match index::list(records_root) {
            Ok(runs) => {
                app.runs = runs;
                app.refilter();
                load_detail(app, records_root);
            }
            Err(e) => app.status = Some(format!("could not reload: {e}")),
        },
        Command::Setup => initialise(app, records_root),
        Command::Mode => {
            let next = app.thread().mode.next();
            set_mode(app, next);
        }
        Command::Models => {
            app.open(Dialog::Models);
            app.models.clear();
            app.models_notes.clear();
            let adapters = app.workspace.adapters();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::models::catalogs(&adapters));
            });
            app.models_loading = Some(rx);
        }
        Command::Keys => app.open(Dialog::Keys),
        Command::Quit => leave(app),
    }
}

/// Changes what enter does in this thread, and says so where it will be read.
fn set_mode(app: &mut App, mode: Mode) {
    app.thread_mut().mode = mode;
    app.status = Some(format!("{} \u{2014} {}", mode.word(), mode.about()));
}

/// Why a command is not on offer, said as the reason rather than as a refusal.
fn unavailable(app: &App, command: Command) -> String {
    let situation = app.situation();
    match command {
        Command::NewRun if situation.running => format!(
            "a run is already going in this pane \u{2014} s asks it to stop, {} opens another",
            label(Chord::NewPane)
        ),
        Command::NewRun => {
            "this directory cannot run anything yet \u{2014} i sets it up".to_string()
        }
        Command::Stop => "nothing is running in this pane".to_string(),
        Command::Fix => "nothing is in the way".to_string(),
        Command::NextPane | Command::ClosePane | Command::MovePaneLeft | Command::MovePaneRight => {
            "this is the only pane".to_string()
        }
        Command::WiderPane | Command::NarrowerPane | Command::EvenPanes => {
            "a width is a share of the screen \u{2014} panes share it from 160 columns".to_string()
        }
        Command::Setup => "this directory is already set up".to_string(),
        other => format!("{} is not available here", other.name()),
    }
}

/// Keys while something is open over the screen.
fn dialog_key(app: &mut App, key: KeyEvent, records_root: &Path) {
    match app.dialog {
        Some(Dialog::Commands) => match key.code {
            KeyCode::Esc => app.close(),
            KeyCode::Enter => {
                let picked = app.picked();
                app.close();
                if let Some(command) = picked {
                    perform(app, command, records_root);
                }
            }
            KeyCode::Down => app.move_pick(1),
            KeyCode::Up => app.move_pick(-1),
            KeyCode::Backspace => {
                app.query.pop();
                app.pick = 0;
            }
            KeyCode::Char(c) => {
                app.query.push(c);
                app.pick = 0;
            }
            _ => {}
        },
        Some(Dialog::Runs) => match key.code {
            KeyCode::Esc => app.close(),
            KeyCode::Enter => {
                if app.current().is_some() {
                    app.close();
                    app.screen = Screen::Record;
                    app.focus = Focus::Keys;
                    app.scroll = 0;
                    load_detail(app, records_root);
                }
            }
            KeyCode::Down => {
                app.move_by(1);
                load_detail(app, records_root);
            }
            KeyCode::Up => {
                app.move_by(-1);
                load_detail(app, records_root);
            }
            KeyCode::Backspace => {
                app.filter.pop();
                app.refilter();
            }
            KeyCode::Char(c) => {
                app.filter.push(c);
                app.refilter();
            }
            _ => {}
        },
        Some(Dialog::Agents) => agents_key(app, key.code),
        Some(Dialog::Models) => models_key(app, key.code),
        Some(Dialog::Fallback) => fallback_key(app, key.code),
        Some(Dialog::Fix) => fix_key(app, key.code),
        Some(Dialog::Settings) => settings_key(app, key.code),
        Some(Dialog::Repos) => repos_key(app, key.code, records_root),
        Some(Dialog::Prune) => prune_key(app, key.code),
        Some(Dialog::Leaving) => {
            let leaving = matches!(
                key.code,
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
            );
            app.close();
            if leaving {
                depart(app);
            }
        }
        // The keys dialog has nothing to type into, so any key closes it. A
        // reference someone has to work out how to dismiss is a poor reference.
        Some(Dialog::Keys) | None => app.close(),
    }
}

/// What this project is configured to do, for a screen that shows it.
///
/// Read when the settings are opened rather than held: `ostraka.toml` is a
/// file somebody edits, and a browser showing what it said an hour ago would
/// be showing the wrong thing precisely when they had just changed it.
fn project_facts(app: &App) -> Vec<(String, String)> {
    let Some(repo) = app.repository() else {
        return Vec::new();
    };
    let Ok(config) = app.workspace.config_for(repo) else {
        return Vec::new();
    };
    let said = |secs: Option<u64>| match secs {
        Some(s) => format!("{s}s"),
        None => "no ceiling".to_string(),
    };
    let mut facts = vec![(
        "gate".to_string(),
        config
            .gate
            .checks
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    )];
    facts.push(("agent stops".to_string(), said(config.policy.timeout_secs)));
    facts.push(("gate stops".to_string(), said(config.gate.timeout_secs)));
    facts.push(("worktrees".to_string(), config.worktree.base.clone()));
    facts.push((
        "config".to_string(),
        app.workspace
            .config_source(repo)
            .strip_prefix(&app.workspace.root)
            .unwrap_or(&app.workspace.config_source(repo))
            .display()
            .to_string(),
    ));
    facts
}

/// Keys while the settings are open.
///
/// The thread's own rows can be typed into; the project's cannot. A gate is
/// what a repository agrees on, and a screen that quietly rewrote it would
/// change what everybody else's runs are judged by.
fn settings_key(app: &mut App, code: KeyCode) {
    let rows = view::settings_rows(app);
    if let Some(buffer) = app.editing.as_mut() {
        match code {
            KeyCode::Esc => app.editing = None,
            KeyCode::Enter => {
                let value = buffer.trim().to_string();
                app.editing = None;
                apply_setting(app, rows.get(app.pick).map(|r| r.0), value);
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Esc => app.close(),
        KeyCode::Down => app.pick = (app.pick + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Char('a') => {
            app.agents = agents(&app.workspace);
            app.open(Dialog::Agents);
        }
        KeyCode::Enter => {
            if let Some((_, value, editable)) = rows.get(app.pick) {
                if *editable {
                    // Starts from what is there, because most changes are edits.
                    app.editing = Some(if value == "\u{2014}" {
                        String::new()
                    } else {
                        value.clone()
                    });
                }
            }
        }
        _ => {}
    }
}

/// Puts a typed setting where it belongs.
fn apply_setting(app: &mut App, row: Option<&'static str>, value: String) {
    match row {
        // An empty identity would end up in a commit trailer as nothing at
        // all, so it is refused rather than written.
        Some("author") if !value.is_empty() => app.thread_mut().author = value,
        Some("reviewer") if !value.is_empty() => app.thread_mut().reviewer = value,
        Some("model") => {
            app.thread_mut().model = (!value.is_empty()).then_some(value);
        }
        _ => {}
    }
}

/// Opens the guided fix, worked out fresh: the directory may have been put
/// right in another window since anybody last looked.
/// What is in the way of a run here, if anything.
///
/// The workspace does not have to be a repository; the thing being worked on
/// does. So the question is asked of the repository, and where there is not
/// one yet the answer is to clone something in — which is a step only a person
/// can take, because nobody here knows the URL.
fn blocking(app: &App) -> Option<crate::remedy::Remedy> {
    match app.repository() {
        Some(repo) => crate::remedy::Remedy::diagnose(&repo.path),
        None => Some(crate::remedy::Remedy::nothing_cloned(
            &app.workspace.repositories_dir(),
        )),
    }
}

fn open_fix(app: &mut App) {
    app.remedy = blocking(app);
    app.blocked = app.remedy.as_ref().map(|r| r.problem.clone());
    match app.remedy {
        Some(_) => app.open(Dialog::Fix),
        None => app.status = Some("nothing is in the way".to_string()),
    }
}

/// Keys while the steps out of a problem are being taken.
///
/// One step per keypress, and only when asked for. The second of them commits
/// whatever is lying in the operator's directory, which is not a thing to do
/// quietly on somebody's behalf.
fn fix_key(app: &mut App, code: KeyCode) {
    let project = app
        .repository()
        .map(|r| r.path.clone())
        .unwrap_or_else(|| app.workspace.root.clone());
    let Some(remedy) = app.remedy.as_mut() else {
        app.close();
        return;
    };
    match code {
        KeyCode::Esc => {
            app.remedy = None;
            app.close();
        }
        // Only where there is something to run. A step with no commands is one
        // the browser cannot take, and taking it would mark it done and change
        // nothing at all.
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
            if !remedy.done()
                && remedy
                    .steps
                    .get(remedy.at)
                    .is_some_and(|step| !step.commands.is_empty()) =>
        {
            remedy.take_step(&project);
            // Asked again rather than assumed: the steps are what somebody
            // believed would work, and whether they did is a question for the
            // directory.
            if remedy.done() && !remedy.failed {
                let found = app.workspace.repository(None).ok();
                if found.is_some() {
                    app.pane_mut().repository = found;
                }
                app.blocked = blocking(app).map(|r| r.problem);
            }
        }
        KeyCode::Char('s') if !remedy.done() => remedy.at += 1,
        _ => {}
    }
}

/// Keys while the repository is being chosen.
fn repos_key(app: &mut App, code: KeyCode, records_root: &Path) {
    let repositories = app.workspace.repositories();

    // Naming a new one. Cloning needs a URL only the operator knows; starting
    // one needs a name, which is a thing this screen can take.
    if let Some(name) = app.editing.as_mut() {
        match code {
            KeyCode::Esc => app.editing = None,
            KeyCode::Enter => {
                let name = name.clone();
                app.editing = None;
                match app.workspace.start_repository(&name) {
                    Ok(repo) => {
                        work_in(app, repo);
                        app.close();
                    }
                    Err(e) => app.status = Some(e.to_string()),
                }
            }
            KeyCode::Backspace => {
                name.pop();
            }
            KeyCode::Char(c) => name.push(c),
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('n') => app.editing = Some(String::new()),
        KeyCode::Esc => app.close(),
        KeyCode::Down => app.pick = (app.pick + 1).min(repositories.len().saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Enter => {
            if let Some(repo) = repositories.get(app.pick).cloned() {
                work_in(app, repo);
            }
            let _ = records_root;
            app.close();
        }
        _ => {}
    }
}

/// Moves this pane's line of work into a repository.
///
/// A thread is a chain of runs in one repository, so moving to another starts
/// a new chain rather than continuing this one somewhere it was never made.
fn work_in(app: &mut App, repo: crate::workspace::Repository) {
    let name = repo.name.clone();
    app.pane_mut().repository = Some(repo);
    *app.thread_mut() = thread::Thread::default();
    app.blocked = blocking(app).map(|r| r.problem);
    app.status = Some(format!("working in {name}"));
}

/// Keys while the agents are being chosen.
/// Keys while the leftovers are on screen.
///
/// **`y` is the only key that removes anything.** Enter and escape both close
/// the dialog and take nothing with them, which is what somebody pressing
/// either of them meant — enter is how the other dialogs are dismissed, and a
/// destructive one is the worst place to make that keystroke mean something
/// new.
///
/// An earlier version of this comment said "`y` and nothing else", which read
/// as though enter were unhandled and invited the question of whether it might
/// remove. It cannot.
fn prune_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('y') if !app.leftovers.is_empty() => {
            let (removed, failed) = crate::prune::remove(&app.leftovers);
            app.leftovers.clear();
            app.status = Some(match failed.first() {
                Some(said) => said.clone(),
                None => format!("removed {removed} worktree(s); branches and records untouched"),
            });
            app.close();
        }
        KeyCode::Esc | KeyCode::Enter => app.close(),
        _ => {}
    }
}

fn agents_key(app: &mut App, code: KeyCode) {
    let count = app.agents.len();
    match code {
        KeyCode::Esc | KeyCode::Enter => app.close(),
        KeyCode::Down => app.pick = (app.pick + 1).min(count.saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Char('a') => {
            if let Some(id) = adopt(app) {
                app.thread_mut().adapter = Some(id);
            }
        }
        KeyCode::Char('r') => {
            if let Some(id) = adopt(app) {
                app.thread_mut().review_adapter = Some(id);
            }
        }
        KeyCode::Char('x') => {
            app.thread_mut().adapter = None;
            app.thread_mut().review_adapter = None;
        }
        _ => {}
    }
}

/// Opens the offer, and asks the other profiles whether they answer.
///
/// On a thread, because asking runs every CLI and one that hangs must not
/// freeze the screen. The profile that failed and the one in the other seat are
/// left out: the first just said it cannot, and routing refuses the second.
fn offer_fallback(app: &mut App, offer: session::Fallback) {
    let skip = [offer.failed.clone(), offer.other.clone()];
    let workspace = app.workspace.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found: Vec<Agent> = agents(&workspace)
            .into_iter()
            .filter(|agent| !skip.contains(&Some(agent.id.clone())))
            .collect();
        let _ = tx.send(found);
    });
    app.open(Dialog::Fallback);
    app.agents.clear();
    app.agents_loading = Some(rx);
    app.fallback = Some(offer);
}

/// Keys while another profile is offered for a vendor that could not run.
fn fallback_key(app: &mut App, code: KeyCode) {
    let count = app.agents.len();
    match code {
        KeyCode::Esc => {
            if let Some(offer) = app.fallback.take() {
                if app.pane().prompt.trim().is_empty() {
                    app.pane_mut().replace(offer.task);
                }
                app.status =
                    Some("the task is back in the box \u{2014} nothing ran again".to_string());
            }
            app.agents_loading = None;
            app.close();
        }
        KeyCode::Down => app.pick = (app.pick + 1).min(count.saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Enter => {
            let Some(agent) = app.agents.get(app.pick) else {
                return;
            };
            // Verified, not assumed: one that did not answer would fail the
            // same way, and a second failure is not a way out of the first.
            if !agent.ready {
                app.status = Some(format!(
                    "{} did not answer either \u{2014} pick one marked ready",
                    agent.id
                ));
                return;
            }
            if !app.pane().prompt.trim().is_empty() {
                app.status = Some(
                    "the box holds another task \u{2014} clear it to run this one again"
                        .to_string(),
                );
                return;
            }
            let Some(id) = adopt(app) else {
                return;
            };
            let Some(offer) = app.fallback.take() else {
                return;
            };
            app.agents_loading = None;
            app.close();
            // For the thread from here on, as if it had been picked in the
            // agents dialog: the profile that failed is likely to fail again.
            match offer.seat {
                session::Seat::Writes => app.thread_mut().adapter = Some(id),
                session::Seat::Reviews => app.thread_mut().review_adapter = Some(id),
            }
            // Without the `@` names, which would choose the failed profile
            // again over the one just picked.
            let names: Vec<String> = app
                .workspace
                .profiles()
                .unwrap_or_default()
                .into_iter()
                .map(|profile| profile.id)
                .collect();
            let (task, _, _) = mention::agents_named(&offer.task, &names);
            app.pane_mut().replace(task);
            start_run(app);
        }
        _ => {}
    }
}

/// Keys while the model picker is open.
fn models_key(app: &mut App, code: KeyCode) {
    let count = app.model_rows().len();
    match code {
        KeyCode::Esc => app.close(),
        // The profile and the model together: a model belongs to the profile
        // that listed it, and naming one without the other would hand a model
        // to whichever profile routing chose.
        KeyCode::Enter => {
            if let Some(row) = app.model_rows().get(app.pick).cloned() {
                app.thread_mut().adapter = Some(row.profile.clone());
                app.thread_mut().model = row.model.clone();
                app.status = Some(format!(
                    "{} writes, on {}",
                    row.profile,
                    row.model.as_deref().unwrap_or("its own default model")
                ));
                app.close();
            }
        }
        KeyCode::Down => app.pick = (app.pick + 1).min(count.saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Backspace => {
            app.query.pop();
            app.pick = 0;
        }
        KeyCode::Char(c) => {
            app.query.push(c);
            app.pick = 0;
        }
        _ => {}
    }
}

/// Names the highlighted agent, writing its profile first where the workspace
/// has none.
///
/// Choosing an agent the workspace has not agreed to would otherwise set a
/// name the run cannot resolve — the routing reads `adapters/`, and being
/// offered something that fails when it is taken is worse than not being
/// offered it. Writing the profile is the agreement, and it is the same bytes
/// `init` ships.
///
/// A failure to write is said and nothing is chosen: a status line naming the
/// reason is a better outcome than a selection that will fail later somewhere
/// less obviously connected to this keystroke.
fn adopt(app: &mut App) -> Option<String> {
    let agent = app.agents.get(app.pick)?;
    let id = agent.id.clone();
    if agent.configured {
        return Some(id);
    }
    match crate::init::write_profile(&app.workspace, &id) {
        Ok(()) => {
            app.status = Some(format!("wrote adapters/{id}.toml"));
            app.agents = agents(&app.workspace);
            Some(id)
        }
        Err(e) => {
            app.status = Some(format!("could not write adapters/{id}.toml: {e}"));
            None
        }
    }
}

/// Starts the run that has been written into the box.
///
/// Through `run::execute`, which is what `ostraka run` calls: the gate, the
/// routing and the record are the same whether the task arrived from a shell
/// or from a keystroke.
fn start_run(app: &mut App) {
    let prompt = app.pane_mut().prompt.trim().to_string();
    // Enter on an empty box, with a plan waiting, is agreeing to the plan.
    let planned = prompt.is_empty() && app.thread().pending_plan.is_some();
    // Agents named with `@` say who does it, not what is to be done, so a box
    // holding nothing but names is still an empty task.
    let agents: Vec<String> = app
        .workspace
        .profiles()
        .unwrap_or_default()
        .into_iter()
        .map(|profile| profile.id)
        .collect();
    let (task, _, _) = mention::agents_named(&prompt, &agents);
    if task.is_empty() && !planned {
        // An empty task would be a run whose diff nobody can explain. Say so
        // rather than starting one and refusing it two minutes later.
        app.status = Some("nothing to run \u{2014} write what the agent should do".to_string());
        return;
    }
    if app.pane().running() {
        app.status = Some(unavailable(app, Command::NewRun));
        return;
    }
    if app.setup.is_some() {
        app.status = Some(unavailable(app, Command::NewRun));
        return;
    }
    // Checked here rather than found out from inside the run. The run would
    // fail at `git worktree add` two layers down, in git's own words, having
    // already been started — and the task in the box would be gone.
    if let Some(remedy) = blocking(app) {
        app.blocked = Some(remedy.problem.clone());
        app.remedy = Some(remedy);
        app.open(Dialog::Fix);
        return;
    }
    app.blocked = None;
    app.pane_mut().replace(String::new());
    app.pane_mut().history_at = None;
    app.status = None;
    app.pane_mut().follow = true;
    app.screen = Screen::Work;
    let repository = app.repository().map(|r| r.name.clone());
    let workspace = app.workspace.clone();
    if planned {
        app.thread_mut().start_planned(workspace, repository);
    } else {
        app.thread_mut()
            .start_naming(workspace, repository, prompt, &agents);
    }
}

/// Leaving, asked rather than assumed.
///
/// Quitting is not free: it can discard a task that was being written, and it
/// can stop a run that is going. One key should not do both silently.
fn leave(app: &mut App) {
    // Already on the way out. Asking twice would be asking about the answer.
    if app.leaving {
        return;
    }
    app.open(Dialog::Leaving);
}

/// Going, now that it has been asked for twice.
///
/// Waits for a run rather than abandoning one: leaving with a vendor still
/// writing into a worktree is the thing Ctrl-C was taught to prevent.
fn depart(app: &mut App) {
    if app.anything_running() {
        // Every pane's run, not only the one in front. The browser is leaving,
        // and a run left going in a pane nobody is looking at is still a vendor
        // writing into a worktree.
        for pane in &mut app.panes {
            pane.thread.stop();
        }
        app.leaving = true;
        app.status = Some("stopping the run \u{2014} the browser closes when it has".to_string());
    } else {
        app.quit = true;
    }
}

/// Loads the full record for whatever is selected.
///
/// A run whose record cannot be read is not an error here: it is a run that was
/// interrupted, the listing already says so, and the browser should keep working
/// rather than fall over on the one entry someone is trying to look at.
fn load_detail(app: &mut App, records_root: &Path) {
    let Some(run) = app.current().map(|r| r.run_id.clone()) else {
        return;
    };
    if app.record.is_none() {
        if let Ok((record, events)) = orchestrator::replay(records_root, &run) {
            app.record = Some(record);
            app.events = events;
        }
    }
    // The diff costs a git call, so it is fetched only when someone asks to see
    // one — moving down a list of fifty runs should not shell out fifty times.
    if app.detail == Detail::Diff && app.diff.is_none() {
        let repo = app
            .current()
            .and_then(|summary| crate::promote::repository_for(&app.workspace, summary).ok());
        app.diff = Some(repo.and_then(|repo| index::diff(&repo.path, &run).ok().flatten()));
    }
}

/// Carries out the setup the opening screen offered.
fn initialise(app: &mut App, records_root: &Path) {
    let Some(plan) = &app.setup else {
        return;
    };
    match init::apply(plan, false) {
        Ok(written) => {
            app.status = Some(format!(
                "wrote {} file(s) \u{2014} a task will run here now",
                written.len()
            ));
            app.setup = None;
            app.agents = agents(&app.workspace);
            if let Ok(runs) = index::list(records_root) {
                app.runs = runs;
                app.refilter();
            }
        }
        Err(e) => app.status = Some(format!("could not write the project files: {e}")),
    }
}

fn promote_selected(app: &App) -> String {
    let Some(run) = app.current() else {
        return "nothing selected".to_string();
    };
    if !run.approved() {
        // The gate would refuse this anyway. Saying so without spending a git
        // call is the same answer, sooner.
        return format!("{} was not approved; nothing to promote", run.run_id);
    }

    let repo = match crate::promote::repository_for(&app.workspace, run) {
        Ok(repo) => repo,
        Err(e) => return format!("could not tell which repository: {e}"),
    };
    let config = match app.workspace.config_for(&repo) {
        Ok(config) => config,
        Err(e) => return format!("could not read the configuration: {e}"),
    };
    let records_root: PathBuf = app.workspace.records();

    match promote::promote(&repo.path, &records_root, &run.run_id, &config, None) {
        Ok(Ok(p)) => format!("promoted to {} \u{2014} nothing merged", p.branch),
        Ok(Err(NotPromoted::Refused(r))) => format!("the gate refuses this run: {r:?}"),
        Ok(Err(why)) => format!("not promoted \u{2014} {why}"),
        Err(e) => format!("not promoted \u{2014} {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::Session;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn control(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// A browser chord, held with control, which reaches every chord by default.
    fn chord(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn the_arrows_move_through_the_task_and_typing_lands_there() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        typed(&mut a, "hello world");
        for _ in 0.."world".len() {
            handle(&mut a, press(KeyCode::Left), root);
        }
        handle(&mut a, press(KeyCode::Backspace), root);
        typed(&mut a, ", ");
        assert_eq!(a.pane().prompt, "hello, world");

        for _ in 0..20 {
            handle(&mut a, press(KeyCode::Right), root);
        }
        typed(&mut a, "!");
        assert_eq!(a.pane().prompt, "hello, world!");
    }

    #[test]
    fn a_click_in_the_box_puts_the_cursor_on_the_character_under_it() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        typed(&mut a, "hello world");
        // The keys handed back: the click is what takes the box again.
        handle(&mut a, press(KeyCode::Esc), root);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).expect("terminal");
        terminal
            .draw(|frame| view::draw(frame, &mut a))
            .expect("draws");
        let area = a.prompt_text;
        // Read against the drawn screen, not against the sum that placed it:
        // the two agreeing is the whole point.
        assert_eq!(
            terminal.backend().buffer()[(area.x + 5, area.y)].symbol(),
            " "
        );

        mouse(&mut a, click(area.x + 5, area.y));
        assert_eq!(a.focus, Focus::Prompt);
        typed(&mut a, ",");
        assert_eq!(a.pane().prompt, "hello, world");
    }

    fn drawn(a: &mut App, width: u16, height: u16) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("terminal");
        terminal.draw(|frame| view::draw(frame, a)).expect("draws");
    }

    #[test]
    fn a_click_on_a_column_gives_that_pane_the_keys_and_keeps_both_tasks() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        typed(&mut a, "left task");
        handle(&mut a, chord('t'), root);
        typed(&mut a, "right task");
        drawn(&mut a, 200, 30);

        let (index, area) = a.columns[0];
        assert_eq!(index, 0);
        mouse(&mut a, click(area.x + 2, area.y + 3));
        assert_eq!(a.at, 0);
        typed(&mut a, "!");
        assert_eq!(a.pane().prompt, "left task!");
        assert_eq!(a.panes[1].prompt, "right task");
    }

    #[test]
    fn an_at_completes_a_path_and_enter_completes_before_it_runs() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        a.mentionable = mention::Mentionable {
            loaded: true,
            repository: None,
            agents: vec!["codex".into()],
            paths: vec!["src/".into(), "src/main.rs".into()],
        };
        typed(&mut a, "fix @s");
        handle(&mut a, press(KeyCode::Tab), root);
        assert_eq!(a.pane().prompt, "fix @src/");

        typed(&mut a, "m");
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.pane().prompt, "fix @src/main.rs ");
        assert!(
            a.thread().live.is_none(),
            "enter ran a task the menu was still completing"
        );
    }

    #[test]
    fn walking_back_into_a_mention_starts_its_menu_at_the_top() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        a.mentionable = mention::Mentionable {
            loaded: true,
            repository: None,
            agents: vec![],
            paths: vec!["src/".into(), "src/main.rs".into()],
        };
        typed(&mut a, "@src/ x");
        // A pick left over from a longer list, which the new word does not have.
        a.pick = 2;
        handle(&mut a, press(KeyCode::Left), root);
        handle(&mut a, press(KeyCode::Left), root);
        assert_eq!(a.mentioning().as_deref(), Some("src/"));
        assert_eq!(a.pick, 0);
        assert_eq!(
            a.mention_picked().map(|c| c.text),
            Some("src/main.rs".to_string())
        );
    }

    #[test]
    fn a_box_holding_only_an_agent_is_still_an_empty_task() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        typed(&mut a, "   ");
        handle(&mut a, press(KeyCode::Enter), root);
        assert!(
            a.status
                .as_deref()
                .is_some_and(|s| s.starts_with("nothing to run"))
        );
        assert!(a.thread().live.is_none());
    }

    #[test]
    fn a_vendor_that_could_not_run_is_offered_another_profile_that_answers() {
        use session::{Fallback, Seat};
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        a.thread_mut().offer = Some(Fallback {
            seat: Seat::Writes,
            failed: Some("spent".into()),
            other: None,
            task: "add a flag".into(),
            why: "the author could not run (exit 1): insufficient credit".into(),
        });
        take_stock(&mut a, root);
        assert_eq!(a.dialog, Some(Dialog::Fallback));
        assert!(a.thread().offer.is_none(), "the offer was made twice");

        // What the probe found, set here rather than read off this machine.
        a.agents_loading = None;
        a.agents = vec![
            Agent {
                id: "down".into(),
                ready: false,
                note: "not installed".into(),
                configured: true,
            },
            Agent {
                id: "spare".into(),
                ready: true,
                note: "1.0".into(),
                configured: true,
            },
        ];
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(
            a.thread().adapter,
            None,
            "a profile that did not answer was taken"
        );
        assert_eq!(a.dialog, Some(Dialog::Fallback));

        handle(&mut a, press(KeyCode::Down), root);
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.thread().adapter.as_deref(), Some("spare"));
        assert!(a.fallback.is_none());
        // Nothing can run in `/p`, so the task waits in the box rather than
        // being lost with the run that failed.
        assert_eq!(a.pane().prompt, "add a flag");
    }

    #[test]
    fn declining_the_offer_keeps_the_task_and_the_profile() {
        use session::{Fallback, Seat};
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        a.thread_mut().offer = Some(Fallback {
            seat: Seat::Reviews,
            failed: None,
            other: Some("writer".into()),
            task: "add a flag".into(),
            why: "reviewer could not run (exit 1): quota exceeded".into(),
        });
        take_stock(&mut a, root);
        a.agents_loading = None;
        handle(&mut a, press(KeyCode::Esc), root);
        assert_eq!(a.dialog, None);
        assert_eq!(a.pane().prompt, "add a flag");
        assert_eq!(a.thread().review_adapter, None);
    }

    #[test]
    fn a_model_is_picked_with_the_profile_that_listed_it() {
        use crate::models::Model;
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        a.open(Dialog::Models);
        a.models = vec![
            Model {
                profile: "local-profile".into(),
                model: None,
            },
            Model {
                profile: "local-profile".into(),
                model: Some("model-one".into()),
            },
            Model {
                profile: "hosted-profile".into(),
                model: Some("family/model-two".into()),
            },
        ];
        for c in "two".chars() {
            handle(&mut a, press(KeyCode::Char(c)), root);
        }
        assert_eq!(a.model_rows().len(), 1, "the filter did not narrow");
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.dialog, None);
        assert_eq!(a.thread().adapter.as_deref(), Some("hosted-profile"));
        assert_eq!(a.thread().model.as_deref(), Some("family/model-two"));

        // A profile's own default clears a model picked before.
        a.open(Dialog::Models);
        a.models = vec![Model {
            profile: "local-profile".into(),
            model: None,
        }];
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.thread().adapter.as_deref(), Some("local-profile"));
        assert_eq!(a.thread().model, None);
    }

    #[test]
    fn a_mode_is_chosen_by_name_or_by_shift_tab_and_is_never_run_as_a_task() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        assert_eq!(a.thread().mode, Mode::Auto);

        typed(&mut a, "/plan");
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.thread().mode, Mode::Plan);
        assert!(a.pane().prompt.is_empty());
        assert!(
            a.thread().live.is_none(),
            "the mode's name was run as a task"
        );

        // "ask" is inside "write a task", which is exactly the collision.
        typed(&mut a, "/ask");
        handle(&mut a, press(KeyCode::Enter), root);
        assert_eq!(a.thread().mode, Mode::Ask);
        assert_eq!(a.focus, Focus::Prompt, "the box was not kept");

        handle(&mut a, press(KeyCode::BackTab), root);
        assert_eq!(a.thread().mode, Mode::Plan);
        assert!(a.status.as_deref().is_some_and(|s| s.starts_with("plan")));
    }

    #[test]
    fn enter_on_an_empty_box_says_so_unless_a_plan_is_waiting() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        handle(&mut a, press(KeyCode::Enter), root);
        assert!(
            a.status
                .as_deref()
                .is_some_and(|s| s.starts_with("nothing to run"))
        );
        assert!(a.thread().pending_plan.is_none());
    }

    #[test]
    fn the_panes_are_arranged_from_the_keys() {
        let root = Path::new("/p/.ostraka");
        let mut a = app();
        handle(&mut a, chord('t'), root);
        assert_eq!(a.at, 1);

        handle(&mut a, chord('x'), root);
        handle(&mut a, press(KeyCode::Char('<')), root);
        assert_eq!(a.at, 0, "the pane did not move left");

        // Not side by side yet, so a width has nothing to be a share of.
        handle(&mut a, chord('x'), root);
        handle(&mut a, press(KeyCode::Char('+')), root);
        assert_eq!(a.pane().weight, pane::WEIGHT);
        assert!(a.status.as_deref().is_some_and(|s| s.contains("160")));

        drawn(&mut a, 200, 30);
        handle(&mut a, chord('x'), root);
        handle(&mut a, press(KeyCode::Char('+')), root);
        assert_eq!(a.pane().weight, pane::WEIGHT + 1);
        drawn(&mut a, 200, 30);
        assert!(a.columns[0].1.width > a.columns[1].1.width);

        handle(&mut a, chord('x'), root);
        handle(&mut a, press(KeyCode::Char('=')), root);
        assert_eq!(a.pane().weight, pane::WEIGHT);
    }

    #[test]
    fn a_click_outside_the_box_or_under_a_dialog_moves_nothing() {
        let mut a = app();
        typed(&mut a, "hello");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).expect("terminal");
        terminal
            .draw(|frame| view::draw(frame, &mut a))
            .expect("draws");
        let area = a.prompt_text;

        mouse(&mut a, click(area.x + 1, 0));
        assert_eq!(a.pane().cursor, None);

        a.open(Dialog::Keys);
        mouse(&mut a, click(area.x + 1, area.y));
        assert_eq!(a.pane().cursor, None, "a click reached through a dialog");
    }

    fn app() -> App {
        App::new(Workspace::at(Path::new("/p")), Vec::new())
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            handle(app, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
    }

    fn running(app: &mut App) {
        app.thread_mut().live = Some(Session::recorded(
            "a task",
            vec![ostraka_runtime::progress::Step::Entered(
                ostraka_runtime::progress::Phase::Authoring,
            )],
            None,
        ));
    }

    #[test]
    fn the_box_has_the_keys_and_what_is_typed_is_the_task() {
        // The whole point of this screen is that you say what you want. A
        // browser you have to unlock before you can type into it makes you
        // press a key to do the obvious thing.
        let mut a = app();
        assert_eq!(a.focus, Focus::Prompt);
        typed(&mut a, "quit");
        assert_eq!(a.pane().prompt, "quit");
        assert!(!a.quit, "typing a command's name ran it");
    }

    #[test]
    fn the_chords_reach_out_of_the_box() {
        // A box that could swallow the key that opens the commands would be a
        // box with no way out of it.
        let mut a = app();
        typed(&mut a, "half a task");
        handle(&mut a, chord('k'), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Commands));
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));

        handle(&mut a, chord('x'), Path::new("/p/.ostraka"));
        assert!(a.leader);
        assert_eq!(a.pane().prompt, "half a task", "the chords ate the task");
    }

    #[test]
    fn escape_hands_the_keys_back_and_keeps_the_task() {
        let mut a = app();
        typed(&mut a, "half a thought");
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::Keys);
        assert_eq!(a.pane().prompt, "half a thought");

        // And n takes the box back with the task still in it.
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::Prompt);
        assert_eq!(a.pane().prompt, "half a thought");
    }

    #[test]
    fn the_leader_acts_and_then_disarms() {
        let mut a = app();
        handle(&mut a, chord('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert!(!a.leader, "the leader outlived the key that completed it");
        assert_eq!(a.dialog, Some(Dialog::Leaving));
        handle(&mut a, press(KeyCode::Char('y')), Path::new("/p/.ostraka"));
        assert!(a.quit);
    }

    #[test]
    fn leaving_is_asked_before_it_happens() {
        // Quitting is not free: it discards a task that was being written and
        // it can stop a run. One key should not do both silently.
        let mut a = app();
        typed(&mut a, "a task in progress");
        handle(&mut a, control('c'), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Leaving));
        assert!(!a.quit);

        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, None, "answering no left the question open");
        assert!(!a.quit, "answering no left anyway");
        assert_eq!(
            a.pane().prompt,
            "a task in progress",
            "the task was lost by asking"
        );

        handle(&mut a, control('c'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert!(!a.quit, "escape left instead of staying");
    }

    #[test]
    fn a_leader_letter_nobody_uses_disarms_rather_than_doing_something_else() {
        // The dangerous version is a chord that falls through to the box:
        // the leader then a stray letter would then be typed into the task.
        let mut a = app();
        handle(&mut a, chord('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('z')), Path::new("/p/.ostraka"));
        assert!(!a.leader);
        assert!(!a.quit);
        assert!(
            a.pane().prompt.is_empty(),
            "the leader's letter reached the box"
        );
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn a_step_the_browser_cannot_take_is_not_marked_done_by_pressing_y() {
        let dir = std::env::temp_dir().join(format!("ostraka-yours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let mut a = App::new(Workspace::at(&dir), Vec::new());
        a.remedy = Some(crate::remedy::Remedy::nothing_cloned(&dir));
        a.open(Dialog::Fix);
        handle(&mut a, press(KeyCode::Char('y')), Path::new("/p/.ostraka"));

        let remedy = a.remedy.as_ref().expect("a remedy");
        assert_eq!(remedy.at, 0, "a step nobody took was counted as taken");
        assert!(!remedy.done());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repository_can_be_started_from_the_browser_when_there_is_nothing_to_clone() {
        // A workspace is often opened before there is anything in it, and not
        // every piece of work begins as somebody else's repository.
        let dir = std::env::temp_dir().join(format!("ostraka-newrepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("repositories")).expect("scratch");

        let mut a = App::new(Workspace::at(&dir), Vec::new());
        a.open(Dialog::Repos);
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.editing.as_deref(), Some(""), "n did not ask for a name");

        typed(&mut a, "fresh");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert!(
            dir.join("repositories/fresh/.git").is_dir(),
            "nothing was started"
        );
        assert_eq!(a.repository().map(|r| r.name.clone()), Some("fresh".into()));
        assert_eq!(a.dialog, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_the_browser_will_not_take_says_why_and_leaves_the_dialog_open() {
        let dir = std::env::temp_dir().join(format!("ostraka-badname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("repositories")).expect("scratch");

        let mut a = App::new(Workspace::at(&dir), Vec::new());
        a.open(Dialog::Repos);
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        typed(&mut a, "../escape");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert!(a.repository().is_none());
        assert!(a.status.is_some(), "it declined without saying why");
        assert!(!dir.parent().expect("a parent").join("escape").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pane_is_opened_and_moved_between_without_leaving_the_box() {
        // Switching lines of work happens while typing, so it is a chord
        // rather than a letter.
        let mut a = app();
        assert_eq!(a.panes.len(), 1);

        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        assert_eq!(a.panes.len(), 2);
        assert_eq!(a.at, 1);

        handle(&mut a, chord(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.at, 0, "the next pane wrapped the wrong way");
        handle(&mut a, chord('['), Path::new("/p/.ostraka"));
        assert_eq!(a.at, 1);
    }

    #[test]
    fn each_pane_keeps_its_own_task() {
        // The point of a second pane: a half-written thought survives going
        // and looking at something else.
        let mut a = app();
        typed(&mut a, "the first thought");
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        assert!(
            a.pane().prompt.is_empty(),
            "a new pane opened with the old task in it"
        );

        typed(&mut a, "the second");
        handle(&mut a, chord(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "the first thought");
        handle(&mut a, chord(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "the second");
    }

    #[test]
    fn a_run_in_one_pane_does_not_keep_another_from_starting_one() {
        // Each run has a stop of its own now, so a run in one pane is no
        // longer a reason to refuse one in another. The pane in front is what
        // is asked.
        let mut a = app();
        running(&mut a);
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        assert!(!a.pane().running(), "the new pane inherited the run");
        assert!(a.anything_running(), "the first pane's run went away");
        assert!(
            a.commands().contains(&Command::NewRun),
            "a new run was not offered beside a run in another pane"
        );

        typed(&mut a, "a second task somewhere else");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(
            !a.status
                .as_deref()
                .is_some_and(|s| s.contains("already going")),
            "refused because of a run in another pane: {:?}",
            a.status
        );
    }

    #[test]
    fn a_pane_is_not_closed_out_from_under_a_run() {
        // Closing it would abandon the thread writing into a worktree.
        let mut a = app();
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        running(&mut a);
        a.focus = Focus::Keys;
        handle(&mut a, press(KeyCode::Char('X')), Path::new("/p/.ostraka"));

        assert_eq!(a.panes.len(), 2, "a running pane was closed");
        assert!(
            a.status.as_deref().is_some_and(|s| s.contains("running")),
            "{:?}",
            a.status
        );
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn the_only_pane_is_not_closed() {
        // A browser with no panes is a browser with nothing to type into.
        let mut a = app();
        a.focus = Focus::Keys;
        handle(&mut a, press(KeyCode::Char('X')), Path::new("/p/.ostraka"));
        assert_eq!(a.panes.len(), 1);
        assert_eq!(a.status.as_deref(), Some("this is the only pane"));
    }

    #[test]
    fn a_closed_pane_leaves_the_screen_on_one_that_is_still_there() {
        let mut a = app();
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        assert_eq!(a.at, 2);

        a.focus = Focus::Keys;
        handle(&mut a, press(KeyCode::Char('X')), Path::new("/p/.ostraka"));
        assert_eq!(a.panes.len(), 2);
        assert_eq!(a.at, 1, "the screen landed on a pane that is gone");
        let _ = alt(KeyCode::Char('1'));
    }

    #[test]
    fn a_slash_command_typed_into_the_box_runs_the_command_not_a_task() {
        let mut a = app();
        typed(&mut a, "/settings");
        assert!(a.slashing());
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert_eq!(a.dialog, Some(Dialog::Settings));
        assert!(a.thread().live.is_none(), "a run was started for a command");
        assert!(
            a.pane().prompt.is_empty(),
            "the command was left in the box"
        );
    }

    #[test]
    fn a_setting_typed_into_the_thread_reaches_the_next_run() {
        let mut a = app();
        a.open(Dialog::Settings);
        // The first row is which repository, which is not this screen's to
        // change; the second is the author.
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.editing.is_some(), "enter did not open the row");
        for _ in 0.."author".len() {
            handle(&mut a, press(KeyCode::Backspace), Path::new("/p/.ostraka"));
        }
        typed(&mut a, "archon");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert_eq!(a.editing, None);
        assert_eq!(a.thread().author, "archon");
    }

    #[test]
    fn an_identity_cannot_be_emptied_into_a_commit_trailer() {
        let mut a = app();
        a.open(Dialog::Settings);
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        for _ in 0.."author".len() {
            handle(&mut a, press(KeyCode::Backspace), Path::new("/p/.ostraka"));
        }
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert_eq!(a.thread().author, "author", "an identity was emptied");
    }

    #[test]
    fn a_chord_armed_over_a_dialog_still_completes() {
        // The other way round, the dialog swallowed the letter meant to
        // complete the chord and the leader stayed armed for ever.
        let mut a = app();
        a.open(Dialog::Keys);
        handle(&mut a, chord('x'), Path::new("/p/.ostraka"));
        assert!(a.leader);
        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert!(!a.leader, "the leader was swallowed and stayed armed");
        assert_eq!(a.dialog, Some(Dialog::Leaving));
    }

    #[test]
    fn a_dialog_takes_the_keys_before_anything_else_does() {
        let mut a = app();
        handle(&mut a, chord('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('h')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Keys));

        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, None, "the dialog did not take the key");
        assert!(!a.quit, "q reached the browser through an open dialog");
        assert!(
            a.pane().prompt.is_empty(),
            "q reached the box through a dialog"
        );
    }

    #[test]
    fn the_palette_types_rather_than_running_commands_by_their_letters() {
        let mut a = app();
        handle(&mut a, chord('k'), Path::new("/p/.ostraka"));
        typed(&mut a, "quit");
        assert_eq!(a.query, "quit");
        assert!(!a.quit, "typing a command's name ran it");
        assert_eq!(a.picked(), Some(Command::Quit));

        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert_eq!(
            a.dialog,
            Some(Dialog::Leaving),
            "the palette left without asking"
        );
        handle(&mut a, press(KeyCode::Char('y')), Path::new("/p/.ostraka"));
        assert!(a.quit);
        assert_eq!(a.dialog, None);
    }

    #[test]
    fn a_task_can_take_more_than_one_line() {
        // A task worth writing sometimes takes a paragraph, and a box that
        // could not hold one would push the work back out to the shell.
        let mut a = app();
        typed(&mut a, "first line");
        handle(
            &mut a,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            Path::new("/p/.ostraka"),
        );
        typed(&mut a, "second line");
        assert_eq!(a.pane().prompt, "first line\nsecond line");
        assert!(a.thread().live.is_none(), "a newline started the run");
    }

    #[test]
    fn the_box_offers_back_what_has_been_asked_here() {
        let mut a = app();
        a.thread_mut().history = vec!["first task".into(), "second task".into()];
        handle(&mut a, press(KeyCode::Up), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "second task");
        handle(&mut a, press(KeyCode::Up), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "first task");
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "second task");
        // Forward past the newest is back to an empty box.
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        assert!(a.pane().prompt.is_empty());
    }

    #[test]
    fn an_empty_task_is_not_run() {
        // A run with no task produces a diff nobody can explain, and the
        // reviewer would be asked what it thinks of it.
        let mut a = app();
        typed(&mut a, "   ");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.thread().live.is_none(), "an empty task started a run");
        assert!(a.status.is_some(), "and said nothing about why not");
    }

    #[test]
    fn a_second_run_cannot_be_started_over_a_live_one() {
        let mut a = app();
        running(&mut a);
        typed(&mut a, "the second task");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert_eq!(
            a.thread().live.as_ref().expect("a session").prompt,
            "a task",
            "the live run was replaced"
        );
        assert!(
            a.status
                .as_deref()
                .is_some_and(|s| s.contains("already going")),
            "{:?}",
            a.status
        );
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn quitting_while_a_run_is_going_stops_it_before_leaving() {
        // Walking away here leaves a vendor writing into a worktree, which is
        // the thing Ctrl-C was taught to prevent.
        // In every pane, not only the one in front.
        let mut a = app();
        running(&mut a);
        handle(&mut a, chord('t'), Path::new("/p/.ostraka"));
        running(&mut a);
        handle(&mut a, control('c'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('y')), Path::new("/p/.ostraka"));

        assert!(!a.quit, "the browser left while a run was going");
        assert!(a.leaving);
        for (at, pane) in a.panes.iter().enumerate() {
            assert!(
                pane.thread.live.as_ref().expect("a session").stopping,
                "pane {at}'s run was left going"
            );
        }
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn stopping_when_nothing_is_running_says_so_rather_than_doing_nothing() {
        let mut a = app();
        a.focus = Focus::Keys;
        handle(&mut a, press(KeyCode::Char('s')), Path::new("/p/.ostraka"));
        assert_eq!(a.status.as_deref(), Some("nothing is running in this pane"));
    }

    #[test]
    fn choosing_an_agent_names_it_and_x_gives_the_choice_back_to_routing() {
        let mut a = app();
        a.agents = vec![
            view::Agent {
                id: "writer".into(),
                ready: true,
                note: String::new(),
                configured: true,
            },
            view::Agent {
                id: "reader".into(),
                ready: true,
                note: String::new(),
                configured: true,
            },
        ];
        a.open(Dialog::Agents);

        handle(&mut a, press(KeyCode::Char('a')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread().adapter.as_deref(), Some("writer"));
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('r')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread().review_adapter.as_deref(), Some("reader"));

        handle(&mut a, press(KeyCode::Char('x')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread().adapter, None);
        assert_eq!(a.thread().review_adapter, None);
    }
}
