//! `ostraka tui` — read the runs back.
//!
//! A browser over records that already existed. It holds no logic of its own:
//! the listing comes from `runtime::index`, the detail from `orchestrator::
//! replay`, and promotion goes through `promote::promote` like every other
//! caller — so the gate cannot be bypassed by pressing a key.

mod view;

use crate::project;
use ostraka_runtime::promote::{self, NotPromoted};
use ostraka_runtime::{index, orchestrator};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use view::{App, Detail};

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
        handle(app, key.code, records_root);
    }
    Ok(())
}

fn handle(app: &mut App, code: KeyCode, records_root: &Path) {
    if app.filtering {
        filter_key(app, code);
        load_detail(app, records_root);
        return;
    }

    // Any keypress supersedes the last message. A status line that outlives
    // what it described is worse than no status line.
    app.status = None;
    let page = app.page as i16;
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
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
            app.detail = app.detail.next();
            app.scroll = 0;
            load_detail(app, records_root);
        }
        KeyCode::Char('/') => app.filtering = true,
        KeyCode::Char('r') => match index::list(records_root) {
            Ok(runs) => {
                app.runs = runs;
                app.refilter();
                load_detail(app, records_root);
            }
            Err(e) => app.status = Some(format!("could not reload: {e}")),
        },
        KeyCode::Char('p') => app.status = Some(promote_selected(app)),
        _ => {}
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
