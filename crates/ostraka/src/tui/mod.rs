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
mod pane;
mod remedy;
mod session;
mod theme;
mod thread;
mod view;

use crate::init;
use crate::workspace::Workspace;
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

    let records_root = workspace.records();
    let mut app = open(workspace, &records_root)?;

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
    for pane in &mut app.panes {
        pane.thread.settle_worker();
    }
    Ok(())
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
    // Asked to leave while something was running: the browser stays up until
    // the run it started has actually stopped, so the last thing on screen is
    // what happened rather than a terminal that froze on its way out.
    if app.leaving && !app.situation().running {
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
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('k') if control => return app.open(Dialog::Commands),
        KeyCode::Char('x') if control => {
            app.leader = true;
            return;
        }
        KeyCode::Char('c') if control => return leave(app),
        // Panes are switched between while the box has the keys, so they are
        // chords rather than letters.
        KeyCode::Char('t') if control => return perform(app, Command::NewPane, records_root),
        KeyCode::Char(']') if control => return perform(app, Command::NextPane, records_root),
        KeyCode::Char('[') if control => {
            app.next_pane(-1);
            to_work(app);
            return;
        }
        _ => {}
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
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            app.pane_mut().prompt.push('\n')
        }
        KeyCode::Char('j') if control => app.pane_mut().prompt.push('\n'),
        KeyCode::Enter if app.slashing() => {
            let picked = app.slash_picked();
            app.pane_mut().prompt.clear();
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
        KeyCode::Enter => start_run(app),
        // The task is kept. It took thought to write, and coming back to it
        // after looking something up is the normal way of working.
        KeyCode::Esc => app.focus = Focus::Keys,
        KeyCode::Backspace => {
            app.pane_mut().prompt.pop();
            app.pane_mut().history_at = None;
            app.pick = 0;
        }
        KeyCode::Up => app.recall(-1),
        KeyCode::Down => app.recall(1),
        KeyCode::PageUp => app.scroll_by(-(app.page as i16)),
        KeyCode::PageDown => app.scroll_by(app.page as i16),
        KeyCode::Char(c) => {
            app.pane_mut().prompt.push(c);
            app.pane_mut().history_at = None;
            app.pick = 0;
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
        Command::Fix => "nothing is in the way".to_string(),
        Command::NextPane | Command::ClosePane => "this is the only pane".to_string(),
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
fn blocking(app: &App) -> Option<remedy::Remedy> {
    match app.repository() {
        Some(repo) => remedy::Remedy::diagnose(&repo.path),
        None => Some(remedy::Remedy::nothing_cloned(
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
    if prompt.is_empty() {
        // An empty task would be a run whose diff nobody can explain. Say so
        // rather than starting one and refusing it two minutes later.
        app.status = Some("nothing to run \u{2014} write what the agent should do".to_string());
        return;
    }
    if app.anything_running() {
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
    app.pane_mut().prompt.clear();
    app.pane_mut().history_at = None;
    app.status = None;
    app.pane_mut().follow = true;
    app.screen = Screen::Work;
    let repository = app.repository().map(|r| r.name.clone());
    let workspace = app.workspace.clone();
    app.thread_mut().start(workspace, repository, prompt);
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
        app.thread_mut().stop();
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
        handle(&mut a, control('k'), Path::new("/p/.ostraka"));
        assert_eq!(a.dialog, Some(Dialog::Commands));
        handle(&mut a, press(KeyCode::Esc), Path::new("/p/.ostraka"));

        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
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
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
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
        // ctrl-x then a stray letter would then be typed into the task.
        let mut a = app();
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
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
        a.remedy = Some(remedy::Remedy::nothing_cloned(&dir));
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

        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
        assert_eq!(a.panes.len(), 2);
        assert_eq!(a.at, 1);

        handle(&mut a, control(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.at, 0, "the next pane wrapped the wrong way");
        handle(&mut a, control('['), Path::new("/p/.ostraka"));
        assert_eq!(a.at, 1);
    }

    #[test]
    fn each_pane_keeps_its_own_task() {
        // The point of a second pane: a half-written thought survives going
        // and looking at something else.
        let mut a = app();
        typed(&mut a, "the first thought");
        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
        assert!(
            a.pane().prompt.is_empty(),
            "a new pane opened with the old task in it"
        );

        typed(&mut a, "the second");
        handle(&mut a, control(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "the first thought");
        handle(&mut a, control(']'), Path::new("/p/.ostraka"));
        assert_eq!(a.pane().prompt, "the second");
    }

    #[test]
    fn one_run_at_a_time_across_every_pane() {
        // Not a property of panes. The request to stop is a single flag,
        // because a signal is single, so two runs would both answer it.
        let mut a = app();
        running(&mut a);
        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
        assert!(!a.pane().running(), "the new pane inherited the run");

        typed(&mut a, "a second task somewhere else");
        handle(&mut a, press(KeyCode::Enter), Path::new("/p/.ostraka"));
        assert!(a.pane().thread.live.is_none(), "a second run was started");
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
    fn a_pane_is_not_closed_out_from_under_a_run() {
        // Closing it would abandon the thread writing into a worktree.
        let mut a = app();
        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
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
        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
        handle(&mut a, control('t'), Path::new("/p/.ostraka"));
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
        handle(&mut a, control('x'), Path::new("/p/.ostraka"));
        assert!(a.leader);
        handle(&mut a, press(KeyCode::Char('q')), Path::new("/p/.ostraka"));
        assert!(!a.leader, "the leader was swallowed and stayed armed");
        assert_eq!(a.dialog, Some(Dialog::Leaving));
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
        assert!(
            a.pane().prompt.is_empty(),
            "q reached the box through a dialog"
        );
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
        let mut a = app();
        running(&mut a);
        handle(&mut a, control('c'), Path::new("/p/.ostraka"));
        handle(&mut a, press(KeyCode::Char('y')), Path::new("/p/.ostraka"));

        assert!(!a.quit, "the browser left while a run was going");
        assert!(a.leaving);
        assert!(a.thread().live.as_ref().expect("a session").stopping);
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
