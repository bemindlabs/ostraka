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
mod session;
mod theme;
mod view;

use crate::init;
use crate::project;
use command::Command;
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{index, orchestrator};
use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use session::Session;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use view::{App, Detail, Dialog, Focus, Typing};

type Outcome = Result<bool, Box<dyn std::error::Error>>;

/// How long a keypress waits before the loop looks around again.
///
/// Long enough that an idle browser is not a busy loop, short enough that a
/// keypress feels immediate.
const TICK: Duration = Duration::from_millis(250);

pub fn run(project_dir: &Path) -> Outcome {
    // Said plainly, before anything is set up. Piped into `head`, the terminal
    // library's own failure is a panic and a backtrace naming a file inside a
    // dependency, which tells the operator nothing they can act on.
    if !std::io::stdout().is_terminal() {
        return Err("`ostraka tui` needs a terminal. For a pipe or a log, \
                    `ostraka runs` prints the same listing, and `--json` \
                    prints it for a machine."
            .into());
    }

    let records_root = project_dir.join(".ostraka");
    let mut app = open(project_dir, &records_root)?;

    // Installs a panic hook that restores the terminal first. Without it a
    // panic leaves the operator staring at a shell with no echo and no prompt.
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &records_root);
    ratatui::restore();
    result?;

    Ok(true)
}

/// The browser as it is when it opens.
///
/// Separated from `run` so the flow tests start where an operator starts,
/// rather than from an `App` assembled by hand that could drift from this one.
fn open(project_dir: &Path, records_root: &Path) -> Result<App, Box<dyn std::error::Error>> {
    let mut app = App::new(project_dir.to_path_buf(), index::list(records_root)?);
    // Opened somewhere that is not a project yet: say what is missing and offer
    // to write it, rather than showing an empty list that looks like a bug.
    //
    // `runnable`, not `complete`: the browser asks whether this directory can
    // be run, and `init` asks whether it has anything left to write. Those are
    // different questions, and asking the second one put "not an Ostraka
    // project yet" across a screen with three recorded runs behind it.
    app.setup = Some(init::plan(project_dir)).filter(|plan| !plan.runnable());
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
        let TermEvent::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        handle(app, key, records_root);
    }

    // Never walk away from a run. Leaving here with a vendor still writing
    // into a worktree is the thing Ctrl-C was taught to prevent, and closing a
    // window is not a better reason to do it than pressing a key was.
    if let Some(session) = app.session.as_mut() {
        session.stop();
        session.settle();
    }
    Ok(())
}

/// Takes whatever the run has said since the last frame.
///
/// Called before drawing rather than after a key, because a run says things
/// while nobody is pressing anything — which is most of the time it takes.
fn take_stock(app: &mut App, records_root: &Path) {
    let ended = match app.session.as_mut() {
        Some(session) => {
            let was_live = session.live();
            session.drain();
            was_live && !session.live()
        }
        None => false,
    };
    if ended {
        finished_run(app, records_root);
    }
    // Asked to leave while something was running: the browser stays up until
    // the run it started has actually stopped, so the last thing on screen is
    // what happened rather than a terminal that froze on its way out.
    if app.leaving && !app.situation().running {
        app.quit = true;
    }
}

/// What to do about a run that has just ended.
fn finished_run(app: &mut App, records_root: &Path) {
    let (summary, run_id) = match app.session.as_ref() {
        Some(session) => (
            session
                .finished
                .as_ref()
                .map(|f| f.summary.clone())
                .or_else(|| session.failed.clone()),
            session.finished.as_ref().map(|f| f.run_id.clone()),
        ),
        None => (None, None),
    };
    app.status = summary;
    app.follow = true;

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
            // Selected, so that closing the transcript lands on the run it was
            // about rather than on whatever was selected before it started.
            app.selected = at;
            app.forget_detail();
            load_detail(app, records_root);
        }
    }
}

fn handle(app: &mut App, key: KeyEvent, records_root: &Path) {
    // In the order a keystroke has to be read: what is open over the screen
    // first, then what is being typed into, then a half-finished chord, and
    // only then the browser itself. Any other order lets an open dialog be
    // quit out from under the person reading it.
    if app.dialog.is_some() {
        dialog_key(app, key.code, records_root);
        return;
    }
    if app.typing.is_some() {
        typing_key(app, key.code, records_root);
        return;
    }
    if app.leader {
        app.leader = false;
        if let KeyCode::Char(c) = key.code {
            if let Some(command) = Command::offered(app.situation())
                .into_iter()
                .find(|command| command.leader() == c)
            {
                perform(app, command, records_root);
            }
        }
        return;
    }

    // Any keypress supersedes the last message. A status line that outlives
    // what it described is worse than no status line.
    app.status = None;
    let page = app.page as i16;
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('k') if control => app.open(Dialog::Commands),
        KeyCode::Char('x') if control => app.leader = true,
        KeyCode::Char('?') => app.open(Dialog::Keys),
        KeyCode::Char('q') => leave(app),
        // Escape gives back whatever it can before it gives up the browser: a
        // finished transcript, then a filter, then a pane on a narrow
        // terminal, and only then the screen.
        KeyCode::Esc => {
            if app.session.as_ref().is_some_and(|s| !s.live()) {
                app.session = None;
                app.scroll = 0;
            } else if !app.filter.is_empty() {
                app.filter.clear();
                app.refilter();
                load_detail(app, records_root);
            } else if app.focus == Focus::Detail {
                app.focus = Focus::List;
            } else {
                leave(app);
            }
        }
        KeyCode::Enter => app.focus = Focus::Detail,
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
            app.follow = true;
            app.move_by(app.matching.len() as isize);
            load_detail(app, records_root);
        }
        KeyCode::Char(' ') | KeyCode::PageDown => app.scroll_by(page),
        KeyCode::Char('b') | KeyCode::PageUp => app.scroll_by(-page),
        KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
            perform(app, Command::NextPane, records_root)
        }
        KeyCode::Char('/') => perform(app, Command::Filter, records_root),
        KeyCode::Char('n') => perform(app, Command::NewRun, records_root),
        KeyCode::Char('s') => perform(app, Command::Stop, records_root),
        KeyCode::Char('i') => perform(app, Command::Setup, records_root),
        KeyCode::Char('r') => perform(app, Command::Reload, records_root),
        KeyCode::Char('p') => perform(app, Command::Promote, records_root),
        _ => {}
    }
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
    // Enter would replace the session — abandoning the thread still writing
    // into a worktree, which is the one thing this browser must not do.
    if !Command::offered(app.situation()).contains(&command) {
        app.status = Some(unavailable(app, command));
        return;
    }

    match command {
        Command::NewRun => app.typing = Some(Typing::Prompt),
        Command::Stop => {
            if let Some(session) = app.session.as_mut() {
                session.stop();
            }
            app.status = Some("asked the agent to stop".to_string());
        }
        Command::Filter => app.typing = Some(Typing::Filter),
        Command::NextPane => {
            app.detail = app.detail.next();
            app.scroll = 0;
            load_detail(app, records_root);
        }
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
        Command::Keys => app.open(Dialog::Keys),
        Command::Quit => app.quit = true,
    }
}

/// Why a command is not on offer, said as the reason rather than as a refusal.
fn unavailable(app: &App, command: Command) -> String {
    let situation = app.situation();
    match command {
        Command::NewRun if situation.running => {
            "a run is already going \u{2014} s asks it to stop".to_string()
        }
        Command::NewRun => {
            "this directory cannot run anything yet \u{2014} i sets it up".to_string()
        }
        Command::Stop => "nothing is running".to_string(),
        Command::Setup => "this directory is already set up".to_string(),
        other => format!("{} is not available here", other.name()),
    }
}

/// Keys while something is open over the screen.
fn dialog_key(app: &mut App, code: KeyCode, records_root: &Path) {
    let palette = app.dialog == Some(Dialog::Commands);
    match code {
        KeyCode::Esc => app.close(),
        KeyCode::Enter if palette => {
            let picked = app.picked();
            app.close();
            if let Some(command) = picked {
                perform(app, command, records_root);
            }
        }
        KeyCode::Down => app.move_pick(1),
        KeyCode::Up => app.move_pick(-1),
        KeyCode::Backspace if palette => {
            app.query.pop();
            app.pick = 0;
        }
        KeyCode::Char(c) if palette => {
            app.query.push(c);
            app.pick = 0;
        }
        // The keys dialog has nothing to type into, so any key closes it. A
        // reference someone has to work out how to dismiss is a poor reference.
        _ => app.close(),
    }
}

/// Keys while something is being written into the input line.
///
/// Every command key is a character here, deliberately: a box that swallowed
/// `q` and then quit on the next keystroke would be worse than one that needs
/// an explicit way out.
fn typing_key(app: &mut App, code: KeyCode, records_root: &Path) {
    let Some(mode) = app.typing else { return };
    match (mode, code) {
        // A filter is cheap to retype and expensive to leave on by accident,
        // so escape clears it.
        (Typing::Filter, KeyCode::Esc) => {
            app.filter.clear();
            app.typing = None;
            app.refilter();
        }
        (Typing::Filter, KeyCode::Enter) => app.typing = None,
        (Typing::Filter, KeyCode::Backspace) => {
            app.filter.pop();
            app.refilter();
        }
        (Typing::Filter, KeyCode::Char(c)) => {
            app.filter.push(c);
            app.refilter();
        }
        // A task is the opposite: it took thought to write, so escape sets it
        // aside and `n` brings it back rather than starting from nothing.
        (Typing::Prompt, KeyCode::Esc) => app.typing = None,
        (Typing::Prompt, KeyCode::Enter) => start_run(app),
        (Typing::Prompt, KeyCode::Backspace) => {
            app.prompt.pop();
        }
        (Typing::Prompt, KeyCode::Char(c)) => app.prompt.push(c),
        _ => {}
    }
    if mode == Typing::Filter {
        load_detail(app, records_root);
    }
}

/// Starts the run that has been written into the input line.
///
/// Through `run::execute`, which is what `ostraka run` calls: the gate, the
/// routing and the record are the same whether the task arrived from a shell
/// or from a keystroke.
fn start_run(app: &mut App) {
    let prompt = app.prompt.trim().to_string();
    app.typing = None;
    if prompt.is_empty() {
        // An empty task would be a run whose diff nobody can explain. Say so
        // rather than starting one and refusing it two minutes later.
        app.status = Some("nothing to run — write what the agent should do".to_string());
        return;
    }
    app.prompt.clear();
    app.status = None;
    app.follow = true;
    app.scroll = 0;
    app.session = Some(Session::start(
        app.project.clone(),
        crate::run::Args::for_task(prompt),
    ));
}

/// Leaving, which waits for a run rather than abandoning one.
fn leave(app: &mut App) {
    match app.session.as_mut().filter(|s| s.live()) {
        Some(session) => {
            session.stop();
            app.leaving = true;
            app.status = Some("stopping the run — the browser closes when it has".to_string());
        }
        None => app.quit = true,
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
        app.diff = Some(index::diff(&app.project, &run).ok().flatten());
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
                "wrote {} file(s) — `ostraka run` will work here now",
                written.len()
            ));
            app.setup = None;
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

    let config = match project::load_config(&app.project) {
        Ok(config) => config,
        Err(e) => return format!("could not read ostraka.toml: {e}"),
    };
    let records_root: PathBuf = app.project.join(".ostraka");

    match promote::promote(&app.project, &records_root, &run.run_id, &config, None) {
        Ok(Ok(p)) => format!("promoted to {} — nothing merged", p.branch),
        Ok(Err(NotPromoted::Refused(r))) => format!("the gate refuses this run: {r:?}"),
        Ok(Err(why)) => format!("not promoted — {why}"),
        Err(e) => format!("not promoted — {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn control(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn app() -> App {
        App::new(PathBuf::from("/p"), Vec::new())
    }

    #[test]
    fn the_leader_waits_for_one_key_and_then_acts() {
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        assert!(a.leader, "the leader was not armed");

        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert!(!a.leader, "the leader outlived the key that completed it");
        assert!(a.quit);
    }

    #[test]
    fn a_leader_letter_nobody_uses_disarms_rather_than_doing_something_else() {
        // The dangerous version of this is a chord that falls through to the
        // bare key: ctrl-x then a stray letter would then promote a run.
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('z')), Path::new("/p/.ostraka"));
        assert!(!a.leader);
        assert!(!a.quit);
        assert!(a.status.is_none());
    }

    #[test]
    fn a_dialog_takes_the_keys_before_the_browser_does() {
        // Otherwise q closes the browser from under someone reading the keys.
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('?')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Keys));

        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, None, "the dialog did not take the key");
        assert!(!a.quit, "q reached the browser through an open dialog");
    }

    #[test]
    fn the_palette_types_rather_than_running_commands_by_their_letters() {
        let mut a = app();
        handle(&mut a, control('k'), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Commands));

        for c in "quit".chars() {
            handle(&mut a, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
        assert_eq!(a.query, "quit");
        assert!(!a.quit, "typing a command's name ran it");
        assert_eq!(a.picked(), Some(Command::Quit));

        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.quit);
        assert_eq!(a.dialog, None);
    }

    #[test]
    fn escape_gives_back_the_filter_before_it_gives_up_the_screen() {
        let mut a = app();
        a.filter = "something".into();
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert!(a.filter.is_empty());
        assert!(
            !a.quit,
            "escape quit while there was still a filter to clear"
        );

        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert!(a.quit);
    }

    #[test]
    fn escape_leaves_the_pane_before_it_leaves_the_browser() {
        let mut a = app();
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::Detail);

        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::List);
        assert!(!a.quit);
    }

    fn live(app: &mut App) {
        app.session = Some(Session::recorded(
            "do a thing",
            vec![ostraka_runtime::progress::Step::Entered(
                ostraka_runtime::progress::Phase::Authoring,
            )],
            None,
        ));
    }

    #[test]
    fn typing_a_task_does_not_run_commands_by_their_letters() {
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.typing, Some(Typing::Prompt));

        for c in "quit".chars() {
            handle(&mut a, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
        assert_eq!(a.prompt, "quit");
        assert!(!a.quit, "typing a task quit the browser");
    }

    #[test]
    fn escape_sets_a_task_aside_rather_than_losing_it() {
        // It took thought to write. Coming back to it is the normal way of
        // working, and a box that cleared on escape would punish looking
        // something up mid-sentence.
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        for c in "half a thought".chars() {
            handle(&mut a, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert_eq!(a.typing, None);
        assert_eq!(a.prompt, "half a thought");

        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.prompt, "half a thought", "the task was lost");
    }

    #[test]
    fn an_empty_task_is_not_run() {
        // A run with no task produces a diff nobody can explain, and the
        // reviewer would be asked what it thinks of it.
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char(' ')), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.session.is_none(), "an empty task started a run");
        assert!(a.status.is_some(), "and said nothing about why not");
    }

    #[test]
    fn quitting_while_a_run_is_going_stops_it_before_leaving() {
        // Walking away here leaves a vendor writing into a worktree, which is
        // the thing Ctrl-C was taught to prevent.
        let mut a = app();
        live(&mut a);
        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));

        assert!(!a.quit, "the browser left while a run was going");
        assert!(a.leaving);
        assert!(a.session.as_ref().expect("a session").stopping);
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn escape_closes_a_finished_transcript_before_anything_else() {
        let mut a = app();
        a.session = Some(Session::recorded(
            "do a thing",
            Vec::new(),
            Some(session::Finished {
                run_id: "t1-20260907T000300Z".into(),
                outcome: None,
                summary: "refused".into(),
            }),
        ));
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert!(a.session.is_none());
        assert!(!a.quit, "escape quit while there was a transcript to close");
    }

    #[test]
    fn a_second_run_cannot_be_started_over_the_first_by_any_route() {
        // The palette and the leader read the command list, so they were never
        // going to offer this. The bare key does not read anything, and it is
        // the one that would have replaced a live session and walked away from
        // the thread behind it.
        let mut a = app();
        live(&mut a);
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));

        assert_eq!(a.typing, None, "the box opened for a run that cannot start");
        assert!(a.session.as_ref().expect("a session").live());
        assert!(
            a.status
                .as_deref()
                .is_some_and(|s| s.contains("already going")),
            "{:?}",
            a.status
        );
        assert!(!Command::offered(a.situation()).contains(&Command::NewRun));
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn stopping_when_nothing_is_running_says_so_rather_than_doing_nothing() {
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('s')), Path::new("/p/.ostraka"));
        assert_eq!(a.status.as_deref(), Some("nothing is running"));
    }

    #[test]
    fn the_filter_swallows_every_command_key_while_it_is_open() {
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('/')), Path::new("/p/.ostraka"));
        assert_eq!(a.typing, Some(Typing::Filter));

        for c in "qp".chars() {
            handle(&mut a, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
        assert_eq!(a.filter, "qp");
        assert!(!a.quit);
    }
}
