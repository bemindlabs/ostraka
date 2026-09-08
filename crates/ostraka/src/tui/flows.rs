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

use super::view::{App, Dialog, Focus, Screen};
use super::{handle, take_stock};
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
         \x20 printf 'written by the agent\\n' >> wrote.txt\n\
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

    /// Types a task, runs it, and waits for it to be over.
    fn task(&mut self, text: &str) -> &mut Self {
        let done = self.app.thread.turns.len() + 1;
        self.typed(text).key(KeyCode::Enter);
        self.until("the run to finish", move |app| {
            app.thread.turns.len() == done
        })
    }

    /// The last turn in the thread.
    fn last(&self) -> &super::thread::Turn {
        self.app.thread.turns.last().expect("a finished turn")
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
    // would be written, take the offer, and end up somewhere a task works.
    let scratch = Scratch::new("setup");
    std::fs::write(scratch.path().join("Cargo.toml"), "[package]\n").expect("write");
    let mut d = Driver::open(scratch.path());

    d.shows("not an Ostraka project yet");
    d.shows("a Rust project");
    d.shows("i set this directory up");

    // Not offered here, and saying why beats a key that quietly does nothing.
    d.typed("a task this directory cannot take")
        .key(KeyCode::Enter);
    d.shows("cannot run anything yet");
    assert!(d.app.thread.live.is_none());

    d.ctrl('x').key(KeyCode::Char('i'));
    d.hides("not an Ostraka project yet");
    d.shows("Nothing has been asked here yet");
    assert!(scratch.path().join("adapters/codex.toml").is_file());
}

#[test]
fn a_task_typed_into_the_box_runs_and_is_recorded() {
    let _guard = exclusive();
    let scratch = project("run", 0);
    let mut d = Driver::open(scratch.path());
    d.shows("Nothing has been asked here yet");

    d.task("write a file");

    let screen = d.screen();
    // What was asked, every phase it went through, the agent's own words, the
    // check that ran, the verdict, and how it ended.
    for expected in [
        "write a file",
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

    let run_id = d
        .last()
        .finished
        .as_ref()
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
    assert_eq!(d.app.runs.len(), 1);
}

#[test]
fn the_second_task_starts_where_the_first_one_finished() {
    // The whole reason a thread exists. Off `HEAD`, the second task cannot see
    // what the first one wrote, and "now add a test for that" is impossible.
    let _guard = exclusive();
    let scratch = project("thread", 0);
    let mut d = Driver::open(scratch.path());

    assert_eq!(d.app.thread.base_ref, "HEAD");
    d.task("write a file");
    let first = d
        .last()
        .finished
        .as_ref()
        .map(|f| f.run_id.clone())
        .expect("a finished run");
    assert!(d.last().approved(), "the first run was not approved");
    assert_eq!(d.app.thread.base_ref, format!("ostraka/{first}"));
    // And it says so, where you are rather than buried in a menu.
    d.shows("on ");

    d.task("write it again");
    assert_eq!(d.app.thread.turns.len(), 2);

    // The proof is in git: the second run's branch has the first run's commit
    // behind it, which is what "starting where the last one finished" means.
    let second = d
        .last()
        .finished
        .as_ref()
        .map(|f| f.run_id.clone())
        .expect("a second run");
    let out = Command::new("git")
        .args(["log", "--format=%H", &format!("ostraka/{second}")])
        .current_dir(scratch.path())
        .output()
        .expect("git log");
    let commits = String::from_utf8_lossy(&out.stdout).lines().count();
    assert_eq!(commits, 3, "the chain did not build on the first run");
}

#[test]
fn a_refused_run_is_not_the_ground_the_next_one_stands_on() {
    // Building on a change the gate would not take is a way of taking it.
    let _guard = exclusive();
    let scratch = project("refused", 0);
    // A gate nothing can pass.
    std::fs::write(
        scratch.path().join("ostraka.toml"),
        "[gate]\nchecks = [{ name = \"check\", cmd = \"false\", required = true }]\n\n\
         [gate.review]\nmust_differ_from_author = true\n",
    )
    .expect("config");
    let mut d = Driver::open(scratch.path());

    d.task("write a file");
    assert!(!d.last().approved());
    assert_eq!(
        d.app.thread.base_ref, "HEAD",
        "the chain advanced through a refusal"
    );
    d.shows("FAIL");
}

/// Rewrites the writer in a fixture into one whose context fills halfway
/// through, which is what a token limit actually looks like from here: part of
/// a change on disk, a non-zero exit, and the reason on stderr.
fn out_of_context(dir: &Path) {
    std::fs::write(
        dir.join("writer.sh"),
        "#!/bin/sh\n\
         case \"$1\" in --probe) echo ok; exit 0;; esac\n\
         echo 'reading the repository'\n\
         printf 'half a line\\n' >> wrote.txt\n\
         echo 'Error: prompt is too long: 210000 tokens > 200000 maximum' >&2\n\
         exit 1\n",
    )
    .expect("script");
}

#[test]
fn a_task_whose_agent_runs_out_of_tokens_is_refused_and_says_why() {
    // The dangerous shape: the vendor had written part of the change when its
    // context filled, so the worktree is not empty and the exit code is not
    // zero. Half a change is not a change anybody should be asked to review,
    // and the chain must not stand on one.
    let _guard = exclusive();
    let scratch = project("out-of-tokens", 0);
    out_of_context(scratch.path());
    let mut d = Driver::open(scratch.path());
    // Pinned, so which of the two scripts authors is not left to routing.
    d.app.thread.adapter = Some("writer".into());
    d.app.thread.review_adapter = Some("reader".into());

    d.task("write something long");

    assert!(!d.last().approved(), "a half-written change was approved");
    assert_eq!(
        d.app.thread.base_ref, "HEAD",
        "the chain stood on a change that ran out"
    );

    // The vendor's own account of why, both where it said it and on the rule
    // that closes the turn.
    let screen = d.screen();
    assert!(screen.contains("prompt is too long"), "{screen}");
    assert!(
        screen.contains("could not run"),
        "the outcome did not say why:\n{screen}"
    );

    // The reviewer was never asked. There was nothing finished to review, and
    // asking would have spent a second vendor to be told so.
    assert!(
        !screen.contains("review"),
        "a reviewer was called on half a change:\n{screen}"
    );

    // And what it managed to write is kept, because that is the evidence.
    let left = std::fs::read_dir(scratch.path().join("worktrees"))
        .expect("a worktrees directory")
        .filter_map(|e| e.ok())
        .find(|e| e.path().join("wrote.txt").is_file());
    assert!(left.is_some(), "the evidence was thrown away");
}

#[test]
fn a_run_can_be_stopped_from_the_browser_and_is_not_called_a_verdict() {
    let _guard = exclusive();
    let scratch = project("stop", 30);
    let mut d = Driver::open(scratch.path());

    d.typed("a task nobody wants finished").key(KeyCode::Enter);
    // In the same breath as starting it, which is the ordering that used to
    // lose the request to the run clearing the flag behind it.
    d.ctrl('x').key(KeyCode::Char('s'));
    assert!(d.app.thread.live.as_ref().expect("a session").stopping);
    d.shows("stopping");

    d.until("the run to stop", |app| app.thread.turns.len() == 1);
    d.shows("stopped by the operator");
    ostraka_adapter::interrupt::clear();
}

#[test]
fn quitting_during_a_run_waits_for_it_rather_than_walking_away() {
    // A process that exited here would leave a vendor writing into a worktree.
    let _guard = exclusive();
    let scratch = project("quit", 30);
    let mut d = Driver::open(scratch.path());

    d.typed("a task interrupted by leaving").key(KeyCode::Enter);
    d.ctrl('c');

    assert!(!d.app.quit, "the browser left while a run was going");
    assert!(d.app.leaving);
    d.shows("stopping the run");

    d.until("the browser to leave", |app| app.quit);

    let runs = std::fs::read_dir(scratch.path().join(".ostraka/runs"))
        .expect("a runs directory")
        .count();
    assert_eq!(runs, 1, "the run was abandoned without a record");
    ostraka_adapter::interrupt::clear();
}

#[test]
fn a_run_is_looked_up_in_a_dialog_and_read_on_a_screen_of_its_own() {
    let _guard = exclusive();
    let scratch = project("lookup", 0);
    let mut d = Driver::open(scratch.path());
    d.task("write a file");
    d.task("write it again");

    d.ctrl('x').key(KeyCode::Char('l'));
    assert_eq!(d.app.dialog, Some(Dialog::Runs));
    d.shows("write a file");
    d.shows("enter opens it");

    // Typing in the dialog narrows it.
    d.typed("again");
    d.shows("1 of 2 runs");
    d.hides("write a file");

    d.key(KeyCode::Enter);
    assert_eq!(d.app.dialog, None);
    assert_eq!(d.app.screen, Screen::Record);
    assert_eq!(d.app.focus, Focus::Keys);

    // checks, then events, then the diff, which is read from the commit the
    // run made rather than from anything the agent said about itself.
    d.shows("check");
    d.key(KeyCode::Tab);
    d.shows("reading the repository");
    d.key(KeyCode::Tab);
    d.shows("written by the agent");

    // And escape comes back to the work, with the box.
    d.key(KeyCode::Esc);
    assert_eq!(d.app.screen, Screen::Work);
    assert_eq!(d.app.focus, Focus::Prompt);
    assert!(!d.app.quit);
}

#[test]
fn the_agents_dialog_names_who_writes_and_who_reviews() {
    let _guard = exclusive();
    let scratch = project("agents", 0);
    let mut d = Driver::open(scratch.path());

    d.ctrl('x').key(KeyCode::Char('a'));
    assert_eq!(d.app.dialog, Some(Dialog::Agents));
    d.shows("automatic");
    d.shows("writer");
    d.shows("reader");

    d.key(KeyCode::Char('a'));
    d.shows("writes");
    assert!(d.app.thread.adapter.is_some());
    d.key(KeyCode::Down).key(KeyCode::Char('r'));
    assert!(d.app.thread.review_adapter.is_some());
    assert_ne!(d.app.thread.adapter, d.app.thread.review_adapter);
    d.key(KeyCode::Esc);

    // And a run started afterwards is run by the pair that was named.
    let author = d.app.thread.adapter.clone().expect("an author");
    d.task("write a file");
    assert_eq!(
        d.app.current().map(|r| r.adapter.clone()),
        Some(author),
        "the named author did not write it"
    );
}

#[test]
fn a_fresh_thread_goes_back_to_head() {
    let _guard = exclusive();
    let scratch = project("fresh", 0);
    let mut d = Driver::open(scratch.path());
    d.task("write a file");
    assert!(d.app.thread.continuing());

    d.ctrl('x').key(KeyCode::Char('f'));
    assert_eq!(d.app.thread.base_ref, "HEAD");
    assert!(d.app.thread.turns.is_empty());
    d.shows("Nothing has been asked here yet");
}

#[test]
fn the_palette_reaches_a_command_by_name_and_the_leader_by_letter() {
    let scratch = Scratch::new("commands");
    std::fs::write(scratch.path().join("Cargo.toml"), "[package]\n").expect("write");
    let mut d = Driver::open(scratch.path());

    d.ctrl('k').typed("keys").key(KeyCode::Enter);
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.shows("promote a record");
    d.shows("ctrl-x");
    d.key(KeyCode::Esc);

    d.ctrl('x').key(KeyCode::Char('h'));
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.key(KeyCode::Esc);
    assert_eq!(d.app.dialog, None);
}
