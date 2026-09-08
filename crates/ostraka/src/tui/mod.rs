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
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use view::{App, Detail, Dialog, Focus};

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
    let mut app = App::new(project_dir.to_path_buf(), index::list(&records_root)?);
    // Opened somewhere that is not a project yet: say what is missing and offer
    // to write it, rather than showing an empty list that looks like a bug.
    app.setup = Some(init::plan(project_dir)).filter(|plan| !plan.complete());
    load_detail(&mut app, &records_root);

    // Installs a panic hook that restores the terminal first. Without it a
    // panic leaves the operator staring at a shell with no echo and no prompt.
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &records_root);
    ratatui::restore();
    result?;

    Ok(true)
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    records_root: &Path,
) -> std::io::Result<()> {
    while !app.quit {
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
    Ok(())
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
    if app.filtering {
        filter_key(app, key.code);
        load_detail(app, records_root);
        return;
    }
    if app.leader {
        app.leader = false;
        if let KeyCode::Char(c) = key.code {
            if let Some(command) = Command::offered(app.setup.is_some())
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
        KeyCode::Char('q') => app.quit = true,
        // Escape gives back whatever it can before it gives up the browser: a
        // filter, then a pane on a narrow terminal, and only then the screen.
        KeyCode::Esc => {
            if !app.filter.is_empty() {
                app.filter.clear();
                app.refilter();
                load_detail(app, records_root);
            } else if app.focus == Focus::Detail {
                app.focus = Focus::List;
            } else {
                app.quit = true;
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
            app.move_by(app.matching.len() as isize);
            load_detail(app, records_root);
        }
        KeyCode::Char(' ') | KeyCode::PageDown => app.scroll_by(page),
        KeyCode::Char('b') | KeyCode::PageUp => app.scroll_by(-page),
        KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
            perform(app, Command::NextPane, records_root)
        }
        KeyCode::Char('/') => perform(app, Command::Filter, records_root),
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
    match command {
        Command::Filter => app.filtering = true,
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

/// Keys while the filter is being typed.
///
/// Every command key is a filter character here, deliberately: a filter that
/// swallowed `q` and then quit on the next keystroke would be worse than one
/// that needs an explicit way out. Enter keeps it, Escape clears it.
fn filter_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.filter.clear();
            app.filtering = false;
            app.refilter();
        }
        KeyCode::Enter => app.filtering = false,
        KeyCode::Backspace => {
            app.filter.pop();
            app.refilter();
        }
        KeyCode::Char(c) => {
            app.filter.push(c);
            app.refilter();
        }
        _ => {}
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

    #[test]
    fn the_filter_swallows_every_command_key_while_it_is_open() {
        let mut a = app();
        handle(&mut a, press(KeyCode::Char('/')), Path::new("/p/.ostraka"));
        assert!(a.filtering);

        for c in "qp".chars() {
            handle(&mut a, press(KeyCode::Char(c)), Path::new("/p/.ostraka"));
        }
        assert_eq!(a.filter, "qp");
        assert!(!a.quit);
    }
}
