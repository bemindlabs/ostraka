//! Whole journeys through the browser: keys in, screen out.
//!
//! The other tests in this module are about one thing each — a list that
//! scrolls, a status line that totals correctly. These are about getting
//! somewhere: opening a directory that is not a project yet and leaving it set
//! up, typing a task and watching it through to a verdict, changing your mind
//! halfway and having the browser stop rather than walk away.
//!
//! They drive the real key handler and the real drawing, and the run flows
//! drive the real orchestrator — the "vendors" are shell scripts, which is the
//! point, and the same point `milestone_one` makes one crate down: the runtime
//! knows nothing about any particular CLI, so a script is as valid an adapter
//! as a commercial tool. Nothing is stubbed but the agent, and only because
//! spending a vendor to find out whether a key works would be absurd.
//!
//! What they cannot cover is the terminal itself. A pty is not something `std`
//! can open and `script(1)` takes different arguments on every platform this
//! releases for, so the boundary these stop at is `handle` and `draw`. The one
//! thing on the far side of it — that a browser with no terminal says so
//! rather than panicking somewhere inside a dependency — is asserted directly.

use super::view::{App, Dialog, Focus, Typing};
use super::{handle, take_stock};
use ostraka_runtime::progress::Phase;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The interrupt flag is one global for the process, so the flows that read or
/// write it take this first. Without it, stopping one run stops the next.
static SIGNALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    let guard = SIGNALS.lock().unwrap_or_else(|e| e.into_inner());
    ostraka_adapter::interrupt::clear();
    guard
}

/// A temporary directory that cleans up after itself.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ostraka-flow-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A project the browser can actually run something in.
///
/// `pause` is what the author sleeps for before it writes: zero for the flows
/// that want a verdict, and long enough to press a key for the flows that want
/// to interrupt one.
fn project(name: &str, pause: u32) -> Scratch {
    let scratch = Scratch::new(name);
    let dir = scratch.path();
    std::fs::create_dir_all(dir.join("adapters")).expect("adapters");

    // One script, two profiles. It answers a review by reading the marker out
    // of the prompt it was handed — which is the mechanism, not a shortcut —
    // and otherwise writes a file.
    let body = format!(
        "#!/bin/sh\n\
         case \"$1\" in --probe) echo ok; exit 0;; esac\n\
         marker=$(printf '%s' \"$1\" | grep -o 'VERDICT-[0-9a-f]*:' | head -1)\n\
         if [ -n \"$marker\" ]; then\n\
         \x20 echo \"the change does what the task asked\"\n\
         \x20 echo \"$marker APPROVE\"\n\
         else\n\
         \x20 echo 'reading the repository'\n\
         \x20 sleep {pause}\n\
         \x20 printf 'written by the agent\\n' > wrote.txt\n\
         \x20 echo 'done'\n\
         fi\n"
    );
    for id in ["writer", "reader"] {
        let script = dir.join(format!("{id}.sh"));
        std::fs::write(&script, &body).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        std::fs::write(
            dir.join(format!("adapters/{id}.toml")),
            format!(
                "id = \"{id}\"\ncommand = \"{}\"\nargs = [\"{{{{prompt}}}}\"]\nprobe_args = [\"--probe\"]\n",
                script.display()
            ),
        )
        .expect("profile");
    }

    std::fs::write(
        dir.join("ostraka.toml"),
        "[gate]\n\
         checks = [{ name = \"check\", cmd = \"true\", required = true }]\n\n\
         [gate.review]\n\
         must_differ_from_author = true\n",
    )
    .expect("config");
    std::fs::write(dir.join(".gitignore"), "/.ostraka/\n/worktrees/\n").expect("gitignore");
    std::fs::write(dir.join("seed.txt"), "seed\n").expect("seed");

    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "flow@example.invalid"]);
    git(dir, &["config", "user.name", "flow"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "seed"]);
    scratch
}

/// The browser, driven the way an operator drives it.
struct Driver {
    app: App,
    records_root: PathBuf,
    width: u16,
    height: u16,
}

impl Driver {
    /// Opens where `ostraka tui` opens, through the same function.
    fn open(project: &Path) -> Self {
        let records_root = project.join(".ostraka");
        let app = super::open(project, &records_root).expect("the browser opens");
        Self {
            app,
            records_root,
            width: 110,
            height: 26,
        }
    }

    fn key(&mut self, code: KeyCode) -> &mut Self {
        handle(
            &mut self.app,
            KeyEvent::new(code, KeyModifiers::NONE),
            &self.records_root,
        );
        self
    }

    fn ctrl(&mut self, c: char) -> &mut Self {
        handle(
            &mut self.app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            &self.records_root,
        );
        self
    }

    fn typed(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
        self
    }

    /// One turn of the event loop, minus the keyboard.
    fn tick(&mut self) -> &mut Self {
        self.app.tick = self.app.tick.wrapping_add(1);
        take_stock(&mut self.app, &self.records_root);
        self
    }

    /// Turns the loop until something is true, or gives up saying what it was
    /// still looking at.
    fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) -> &mut Self {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done(&self.app) {
            assert!(
                Instant::now() < deadline,
                "waited a minute for {what}, and the screen said:\n{}",
                self.screen()
            );
            self.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        self
    }

    fn screen(&mut self) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(self.width, self.height)).expect("test terminal");
        let app = &mut self.app;
        terminal
            .draw(|frame| super::view::draw(frame, app))
            .expect("draws");
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

    fn shows(&mut self, text: &str) {
        let screen = self.screen();
        assert!(screen.contains(text), "expected {text:?} on:\n{screen}");
    }

    fn hides(&mut self, text: &str) {
        let screen = self.screen();
        assert!(
            !screen.contains(text),
            "did not expect {text:?} on:\n{screen}"
        );
    }
}

#[test]
fn a_browser_with_no_terminal_says_so_rather_than_panicking() {
    // Piped into `head`, the terminal library's own failure is a panic and a
    // backtrace naming a file inside a dependency, which tells the operator
    // nothing they can act on. A test's stdout is not a terminal, so this is
    // the real path.
    let scratch = Scratch::new("no-tty");
    let error = super::run(scratch.path()).expect_err("a pipe is not a terminal");
    let said = error.to_string();
    assert!(said.contains("needs a terminal"), "{said}");
    assert!(
        said.contains("ostraka runs"),
        "no way out was offered: {said}"
    );
}

#[test]
fn setting_up_a_directory_leaves_a_browser_that_can_run_something() {
    // The first five minutes: open somewhere that is not a project, read what
    // would be written, take the offer, and end up somewhere `n` works.
    let scratch = Scratch::new("setup");
    std::fs::write(scratch.path().join("Cargo.toml"), "[package]\n").expect("write");
    let mut d = Driver::open(scratch.path());

    d.shows("not an Ostraka project yet");
    d.shows("a Rust project");
    d.shows("i set this directory up");
    // Not offered here, and saying why beats a key that quietly does nothing.
    d.key(KeyCode::Char('n'));
    d.shows("cannot run anything yet");

    d.key(KeyCode::Char('i'));
    d.hides("not an Ostraka project yet");
    d.shows("No runs yet");
    assert!(scratch.path().join("adapters/codex.toml").is_file());

    // And now a task is something this directory could take.
    d.key(KeyCode::Char('n'));
    assert_eq!(d.app.typing, Some(Typing::Prompt));
}

#[test]
fn a_task_typed_into_the_box_runs_and_ends_up_in_the_list() {
    let _guard = exclusive();
    let scratch = project("run", 0);
    let mut d = Driver::open(scratch.path());
    d.shows("No runs yet");

    d.key(KeyCode::Char('n'))
        .typed("write a file")
        .key(KeyCode::Enter);

    // The transcript replaces the empty list rather than arguing with it.
    d.shows("write a file");
    d.hides("No runs yet");

    d.until("the run to finish", |app| {
        app.session.as_ref().is_some_and(|s| !s.live())
    });

    let screen = d.screen();
    // Every phase it went through, the agent's own words, the check that ran,
    // the verdict, and how it ended.
    for expected in [
        "isolate",
        "prepare",
        "author",
        "reading the repository",
        "gate",
        "check",
        "review",
        "APPROVE",
        "approved",
    ] {
        assert!(screen.contains(expected), "no {expected:?} on:\n{screen}");
    }

    // The record exists, the listing has it, and it is what the selection
    // lands on when the transcript is closed.
    let run_id = d
        .app
        .session
        .as_ref()
        .and_then(|s| s.finished.as_ref())
        .map(|f| f.run_id.clone())
        .expect("a finished run");
    assert!(
        scratch
            .path()
            .join(".ostraka/runs")
            .join(&run_id)
            .join("record.json")
            .is_file()
    );

    d.key(KeyCode::Esc);
    assert!(
        d.app.session.is_none(),
        "escape did not close the transcript"
    );
    assert_eq!(d.app.current().map(|r| r.run_id.clone()), Some(run_id));
    d.shows("1 run");
    d.shows("write a file");
}

#[test]
fn a_run_can_be_stopped_from_the_browser_and_is_not_called_a_verdict() {
    let _guard = exclusive();
    let scratch = project("stop", 30);
    let mut d = Driver::open(scratch.path());

    d.key(KeyCode::Char('n'))
        .typed("a task nobody wants finished")
        .key(KeyCode::Enter);
    // In the same breath as starting it, which is the ordering that used to
    // lose the request to the run clearing the flag behind it.
    d.key(KeyCode::Char('s'));
    assert!(d.app.session.as_ref().expect("a session").stopping);
    d.shows("stopping");

    d.until("the run to stop", |app| {
        app.session.as_ref().is_some_and(|s| !s.live())
    });

    // Its own outcome, and not a verdict on a change nobody ever saw.
    d.shows("stopped by the operator");
    ostraka_adapter::interrupt::clear();
}

#[test]
fn quitting_during_a_run_waits_for_it_rather_than_walking_away() {
    // A process that exited here would leave a vendor writing into a worktree.
    let _guard = exclusive();
    let scratch = project("quit", 30);
    let mut d = Driver::open(scratch.path());

    d.key(KeyCode::Char('n'))
        .typed("a task interrupted by leaving")
        .key(KeyCode::Enter);
    d.key(KeyCode::Char('q'));

    assert!(!d.app.quit, "the browser left while a run was going");
    assert!(d.app.leaving);
    d.shows("stopping the run");

    d.until("the browser to leave", |app| app.quit);

    // It waited long enough for the run to write itself down.
    let runs = std::fs::read_dir(scratch.path().join(".ostraka/runs"))
        .expect("a runs directory")
        .count();
    assert_eq!(runs, 1, "the run was abandoned without a record");
    ostraka_adapter::interrupt::clear();
}

#[test]
fn filtering_narrows_the_list_and_escape_gives_it_back() {
    let _guard = exclusive();
    let scratch = project("filter", 0);
    let mut d = Driver::open(scratch.path());

    for task in ["write a file", "rename a field"] {
        d.key(KeyCode::Char('n')).typed(task).key(KeyCode::Enter);
        d.until("the run to finish", |app| {
            app.session.as_ref().is_some_and(|s| !s.live())
        });
        d.key(KeyCode::Esc);
    }
    d.shows("2 runs");

    d.key(KeyCode::Char('/')).typed("rename");
    d.shows("1 of 2 runs");
    d.hides("write a file");

    d.key(KeyCode::Esc);
    d.shows("2 runs");
    d.shows("write a file");
}

#[test]
fn the_panes_cycle_and_the_diff_is_the_change_that_was_made() {
    let _guard = exclusive();
    let scratch = project("panes", 0);
    let mut d = Driver::open(scratch.path());

    d.key(KeyCode::Char('n'))
        .typed("write a file")
        .key(KeyCode::Enter);
    d.until("the run to finish", |app| {
        app.session.as_ref().is_some_and(|s| !s.live())
    });
    d.key(KeyCode::Esc);

    // checks, then events, then the diff, which is read from the commit the
    // run made rather than from anything the agent said about itself.
    d.shows("check");
    d.key(KeyCode::Tab);
    d.shows("reading the repository");
    d.key(KeyCode::Tab);
    d.shows("written by the agent");
    d.key(KeyCode::Tab);
    d.shows("check");
}

#[test]
fn the_palette_reaches_a_command_by_name_and_the_leader_by_letter() {
    let scratch = Scratch::new("commands");
    std::fs::write(scratch.path().join("Cargo.toml"), "[package]\n").expect("write");
    let mut d = Driver::open(scratch.path());

    // By name, typed.
    d.ctrl('k').typed("keys").key(KeyCode::Enter);
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.shows("promote the selected run");
    d.shows("ctrl-x");
    d.key(KeyCode::Esc);

    // And by chord, which reaches the same command.
    d.ctrl('x').key(KeyCode::Char('h'));
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.key(KeyCode::Esc);
    assert_eq!(d.app.dialog, None);
}

#[test]
fn a_narrow_terminal_moves_between_the_columns_with_enter_and_escape() {
    let _guard = exclusive();
    let scratch = project("narrow", 0);
    let mut d = Driver::open(scratch.path());
    d.key(KeyCode::Char('n'))
        .typed("write a file")
        .key(KeyCode::Enter);
    d.until("the run to finish", |app| {
        app.session.as_ref().is_some_and(|s| !s.live())
    });
    d.key(KeyCode::Esc);

    d.width = 48;
    assert_eq!(d.app.focus, Focus::List);
    d.shows("write a file");
    // The list only: two columns on forty-eight means neither can be read.
    d.hides("author");

    d.key(KeyCode::Enter);
    assert_eq!(d.app.focus, Focus::Detail);
    d.shows("author");

    d.key(KeyCode::Esc);
    assert_eq!(d.app.focus, Focus::List);
    assert!(!d.app.quit, "escape left the browser instead of the pane");
}

#[test]
fn a_second_run_cannot_be_started_over_a_live_one_by_any_route() {
    let _guard = exclusive();
    let scratch = project("one-at-a-time", 30);
    let mut d = Driver::open(scratch.path());

    d.key(KeyCode::Char('n'))
        .typed("the first task")
        .key(KeyCode::Enter);
    let first = d.app.session.as_ref().expect("a session").prompt.clone();

    d.key(KeyCode::Char('n'));
    assert_eq!(d.app.typing, None, "the box opened over a live run");
    d.shows("already going");

    d.ctrl('x').key(KeyCode::Char('n'));
    assert_eq!(d.app.typing, None, "the leader opened it");

    d.ctrl('k').typed("new run");
    d.shows("no command matches that");
    d.key(KeyCode::Esc);

    assert_eq!(
        d.app.session.as_ref().expect("a session").prompt,
        first,
        "the live run was replaced"
    );
    d.until("the run to reach the author", |app| {
        app.session.as_ref().and_then(|s| s.phase) == Some(Phase::Authoring)
    });

    d.key(KeyCode::Char('s'));
    d.until("the run to stop", |app| {
        app.session.as_ref().is_some_and(|s| !s.live())
    });
    ostraka_adapter::interrupt::clear();
}
