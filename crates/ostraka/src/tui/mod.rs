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
mod thread;
mod view;

use crate::init;
use crate::project;
use command::Command;
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{index, orchestrator};
use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
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

/// The adapter profiles this project has, and whether each one can run.
///
/// Probing runs every vendor's binary, so it happens when somebody asks to
/// see the list rather than when the browser opens.
fn agents(project: &Path) -> Vec<Agent> {
    use ostraka_adapter::{VendorAdapter, process::ProcessAdapter};
    let Ok(profiles) = project::load_profiles(project) else {
        return Vec::new();
    };
    let mut agents: Vec<Agent> = profiles
        .into_iter()
        .map(|profile| {
            let adapter = ProcessAdapter::new(profile);
            let availability = adapter.probe();
            Agent {
                id: adapter.id().to_string(),
                ready: availability.is_ready(),
                note: match availability {
                    ostraka_adapter::Availability::Ready { version } => version.unwrap_or_default(),
                    ostraka_adapter::Availability::NotFound { command } => {
                        format!("{command} not found on PATH")
                    }
                    ostraka_adapter::Availability::Unusable { reason } => reason,
                },
            }
        })
        .collect();
    agents.sort_by(|a, b| a.id.cmp(&b.id));
    agents
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
    app.thread.settle_worker();
    Ok(())
}

/// Takes whatever the run has said since the last frame.
///
/// Called before drawing rather than after a key, because a run says things
/// while nobody is pressing anything — which is most of the time it takes.
fn take_stock(app: &mut App, records_root: &Path) {
    if let Some(ended) = app.thread.settle() {
        finished_run(app, records_root, Some(ended));
    } else if !app.thread.running() && app.leaving {
        app.quit = true;
    }
    // Asked to leave while something was running: the browser stays up until
    // the run it started has actually stopped, so the last thing on screen is
    // what happened rather than a terminal that froze on its way out.
    if app.leaving && !app.situation().running {
        app.quit = true;
    }
}

/// What to do about a run that has just ended.
fn finished_run(app: &mut App, records_root: &Path, run_id: Option<String>) {
    let last = app.thread.turns.last();
    app.status = last.and_then(|turn| {
        turn.finished
            .as_ref()
            .map(|f| f.summary.clone())
            .or_else(|| turn.failed.clone())
    });
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
    // swallow the key that opens the commands. Then what is open over the
    // screen, then a half-finished chord, then whoever has the keyboard.
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('k') if control => return app.open(Dialog::Commands),
        KeyCode::Char('x') if control => {
            app.leader = true;
            return;
        }
        KeyCode::Char('c') if control => return leave(app),
        _ => {}
    }

    if app.dialog.is_some() {
        dialog_key(app, key, records_root);
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

    if app.focus == Focus::Prompt {
        prompt_key(app, key);
    } else {
        command_key(app, key, records_root);
    }
}

/// Keys while the box has them, which on the work screen is most of the time.
///
/// The whole point of this browser is that you say what you want, so what you
/// type is the task. Everything else is a chord or is behind escape.
fn prompt_key(app: &mut App, key: KeyEvent) {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // A task worth writing sometimes takes a paragraph, and a box that
        // could not hold one would push the work back out to the shell.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => app.prompt.push('\n'),
        KeyCode::Char('j') if control => app.prompt.push('\n'),
        KeyCode::Enter => start_run(app),
        // The task is kept. It took thought to write, and coming back to it
        // after looking something up is the normal way of working.
        KeyCode::Esc => app.focus = Focus::Keys,
        KeyCode::Backspace => {
            app.prompt.pop();
            app.history_at = None;
        }
        KeyCode::Up => app.recall(-1),
        KeyCode::Down => app.recall(1),
        KeyCode::PageUp => app.scroll_by(-(app.page as i16)),
        KeyCode::PageDown => app.scroll_by(app.page as i16),
        KeyCode::Char(c) => {
            app.prompt.push(c);
            app.history_at = None;
        }
        _ => {}
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
        KeyCode::Char('a') => perform(app, Command::Agents, records_root),
        KeyCode::Char('i') => perform(app, Command::Setup, records_root),
        KeyCode::Char('r') => perform(app, Command::Reload, records_root),
        KeyCode::Char('p') => perform(app, Command::Promote, records_root),
        KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
            perform(app, Command::NextPane, records_root)
        }
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
        _ => {}
    }
}

/// Back to the work, with the box.
fn to_work(app: &mut App) {
    app.screen = Screen::Work;
    app.focus = Focus::Prompt;
    app.follow = true;
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
            app.thread.stop();
            app.status = Some("asked the agent to stop".to_string());
        }
        Command::Runs => app.open(Dialog::Runs),
        Command::Agents => {
            app.agents = agents(&app.project);
            app.open(Dialog::Agents);
        }
        Command::Fresh => {
            app.thread = thread::Thread::default();
            app.status = Some("a fresh thread \u{2014} the next run starts from HEAD".to_string());
            to_work(app);
        }
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
        Command::Quit => leave(app),
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
        // The keys dialog has nothing to type into, so any key closes it. A
        // reference someone has to work out how to dismiss is a poor reference.
        Some(Dialog::Keys) | None => app.close(),
    }
}

/// Keys while the agents are being chosen.
fn agents_key(app: &mut App, code: KeyCode) {
    let ids: Vec<String> = app.agents.iter().map(|a| a.id.clone()).collect();
    let here = ids.get(app.pick).cloned();
    match code {
        KeyCode::Esc | KeyCode::Enter => app.close(),
        KeyCode::Down => app.pick = (app.pick + 1).min(ids.len().saturating_sub(1)),
        KeyCode::Up => app.pick = app.pick.saturating_sub(1),
        KeyCode::Char('a') => app.thread.adapter = here,
        KeyCode::Char('r') => app.thread.review_adapter = here,
        KeyCode::Char('x') => {
            app.thread.adapter = None;
            app.thread.review_adapter = None;
        }
        _ => {}
    }
}

/// Starts the run that has been written into the box.
///
/// Through `run::execute`, which is what `ostraka run` calls: the gate, the
/// routing and the record are the same whether the task arrived from a shell
/// or from a keystroke.
fn start_run(app: &mut App) {
    let prompt = app.prompt.trim().to_string();
    if prompt.is_empty() {
        // An empty task would be a run whose diff nobody can explain. Say so
        // rather than starting one and refusing it two minutes later.
        app.status = Some("nothing to run \u{2014} write what the agent should do".to_string());
        return;
    }
    if app.thread.running() {
        app.status = Some(unavailable(app, Command::NewRun));
        return;
    }
    if app.setup.is_some() {
        app.status = Some(unavailable(app, Command::NewRun));
        return;
    }
    app.prompt.clear();
    app.history_at = None;
    app.status = None;
    app.follow = true;
    app.screen = Screen::Work;
    app.thread.start(app.project.clone(), prompt);
}

/// Leaving, which waits for a run rather than abandoning one.
fn leave(app: &mut App) {
    if app.thread.running() {
        app.thread.stop();
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
                "wrote {} file(s) \u{2014} a task will run here now",
                written.len()
            ));
            app.setup = None;
            app.agents = agents(&app.project);
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

    fn app() -> App {
        App::new(PathBuf::from("/p"), Vec::new())
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            handle(app, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
    }

    fn running(app: &mut App) {
        app.thread.live = Some(Session::recorded(
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
        assert_eq!(a.prompt, "quit");
        assert!(!a.quit, "typing a command's name ran it");
    }

    #[test]
    fn the_chords_reach_out_of_the_box() {
        // A box that could swallow the key that opens the commands would be a
        // box with no way out of it.
        let mut a = app();
        typed(&mut a, "half a task");
        handle(&mut a, control('k'), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Commands));
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));

        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        assert!(a.leader);
        assert_eq!(a.prompt, "half a task", "the chords ate the task");
    }

    #[test]
    fn escape_hands_the_keys_back_and_keeps_the_task() {
        let mut a = app();
        typed(&mut a, "half a thought");
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::Keys);
        assert_eq!(a.prompt, "half a thought");

        // And n takes the box back with the task still in it.
        handle(&mut a, press(KeyCode::Char('n')), Path::new("/p/.ostraka"));
        assert_eq!(a.focus, Focus::Prompt);
        assert_eq!(a.prompt, "half a thought");
    }

    #[test]
    fn the_leader_acts_and_then_disarms() {
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert!(!a.leader, "the leader outlived the key that completed it");
        assert!(a.quit);
    }

    #[test]
    fn a_leader_letter_nobody_uses_disarms_rather_than_doing_something_else() {
        // The dangerous version is a chord that falls through to the box:
        // ctrl-x then a stray letter would then be typed into the task.
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('z')), Path::new("/p/.ostraka"));
        assert!(!a.leader);
        assert!(!a.quit);
        assert!(a.prompt.is_empty(), "the leader's letter reached the box");
    }

    #[test]
    fn a_dialog_takes_the_keys_before_anything_else_does() {
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('h')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Keys));

        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, None, "the dialog did not take the key");
        assert!(!a.quit, "q reached the browser through an open dialog");
        assert!(a.prompt.is_empty(), "q reached the box through a dialog");
    }

    #[test]
    fn the_palette_types_rather_than_running_commands_by_their_letters() {
        let mut a = app();
        handle(&mut a, control('k'), Path::new("/p/.ostraka"));
        typed(&mut a, "quit");
        assert_eq!(a.query, "quit");
        assert!(!a.quit, "typing a command's name ran it");
        assert_eq!(a.picked(), Some(Command::Quit));

        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
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
        assert_eq!(a.prompt, "first line\nsecond line");
        assert!(a.thread.live.is_none(), "a newline started the run");
    }

    #[test]
    fn the_box_offers_back_what_has_been_asked_here() {
        let mut a = app();
        a.thread.history = vec!["first task".into(), "second task".into()];
        handle(&mut a, press(KeyCode::Up), Path::new("/p/.ostraka"));
        assert_eq!(a.prompt, "second task");
        handle(&mut a, press(KeyCode::Up), Path::new("/p/.ostraka"));
        assert_eq!(a.prompt, "first task");
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        assert_eq!(a.prompt, "second task");
        // Forward past the newest is back to an empty box.
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        assert!(a.prompt.is_empty());
    }

    #[test]
    fn an_empty_task_is_not_run() {
        // A run with no task produces a diff nobody can explain, and the
        // reviewer would be asked what it thinks of it.
        let mut a = app();
        typed(&mut a, "   ");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.thread.live.is_none(), "an empty task started a run");
        assert!(a.status.is_some(), "and said nothing about why not");
    }

    #[test]
    fn a_second_run_cannot_be_started_over_a_live_one() {
        let mut a = app();
        running(&mut a);
        typed(&mut a, "the second task");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));

        assert_eq!(
            a.thread.live.as_ref().expect("a session").prompt,
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
        let mut a = app();
        running(&mut a);
        handle(&mut a, control('c'), Path::new("/p/.ostraka"));

        assert!(!a.quit, "the browser left while a run was going");
        assert!(a.leaving);
        assert!(a.thread.live.as_ref().expect("a session").stopping);
        ostraka_adapter::interrupt::clear();
    }

    #[test]
    fn stopping_when_nothing_is_running_says_so_rather_than_doing_nothing() {
        let mut a = app();
        a.focus = Focus::Keys;
        handle(&mut a, press(KeyCode::Char('s')), Path::new("/p/.ostraka"));
        assert_eq!(a.status.as_deref(), Some("nothing is running"));
    }

    #[test]
    fn choosing_an_agent_names_it_and_x_gives_the_choice_back_to_routing() {
        let mut a = app();
        a.agents = vec![
            view::Agent {
                id: "writer".into(),
                ready: true,
                note: String::new(),
            },
            view::Agent {
                id: "reader".into(),
                ready: true,
                note: String::new(),
            },
        ];
        a.open(Dialog::Agents);

        handle(&mut a, press(KeyCode::Char('a')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread.adapter.as_deref(), Some("writer"));
        handle(&mut a, press(KeyCode::Down), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('r')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread.review_adapter.as_deref(), Some("reader"));

        handle(&mut a, press(KeyCode::Char('x')), Path::new("/p/.ostraka"));
        assert_eq!(a.thread.adapter, None);
        assert_eq!(a.thread.review_adapter, None);
    }
}
