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
use crate::chord::{Action as Chord, label};
use crate::workspace::Workspace;
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

/// A workspace the browser can actually run something in.
///
/// ```text
/// <workspace>/.ostraka/{ostraka.toml, adapters/}
/// <workspace>/repositories/work   a git repository with one commit
/// <workspace>/notes/
/// ```
///
/// `pause` is what the author sleeps for before it writes: zero for the flows
/// that want a verdict, and long enough to press a key for the flows that want
/// to interrupt one.
fn project(name: &str, pause: u32) -> Scratch {
    let scratch = Scratch::new(name);
    let dir = scratch.path();
    std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("ostraka");
    std::fs::create_dir_all(dir.join("notes")).expect("notes");
    let repo = dir.join("repositories/work");
    std::fs::create_dir_all(&repo).expect("repository");

    // One script, two profiles. It answers a review by reading the marker out
    // of the prompt it was handed — which is the mechanism, not a shortcut —
    // answers read-only when it is invoked read-only, and otherwise writes a
    // file.
    //
    // The read-only invocation is not decoration. A profile without one cannot
    // be consulted at all — a question would have to go through the invocation
    // that can write, and is refused instead — so a fixture without it cannot
    // be asked anything, which is most of what this browser now does.
    let body = format!(
        "#!/bin/sh\n\
         case \"$1\" in --probe) echo ok; exit 0;; esac\n\
         review=no\n\
         case \"$1\" in --review) review=yes; shift;; esac\n\
         marker=$(printf '%s' \"$1\" | grep -o 'VERDICT-[0-9a-f]*:' | head -1)\n\
         if [ -n \"$marker\" ]; then\n\
         \x20 echo \"the change does what the task asked\"\n\
         \x20 echo \"$marker APPROVE\"\n\
         elif [ \"$review\" = yes ]; then\n\
         \x20 echo 'this repository holds a seed file and nothing else'\n\
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
            dir.join(format!(".ostraka/adapters/{id}.toml")),
            format!(
                "id = \"{id}\"\ncommand = \"{}\"\nargs = [\"{{{{prompt}}}}\"]\n\
                 review_args = [\"--review\", \"{{{{prompt}}}}\"]\nprobe_args = [\"--probe\"]\n",
                script.display()
            ),
        )
        .expect("profile");
    }

    std::fs::write(
        dir.join(".ostraka/ostraka.toml"),
        "[gate]\n\
         checks = [{ name = \"check\", cmd = \"true\", required = true }]\n\n\
         [gate.review]\n\
         must_differ_from_author = true\n",
    )
    .expect("config");
    std::fs::write(repo.join(".gitignore"), "/.ostraka/\n").expect("gitignore");
    std::fs::write(repo.join("seed.txt"), "seed\n").expect("seed");

    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "flow@example.invalid"]);
    git(&repo, &["config", "user.name", "flow"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    scratch
}

/// The one repository a fixture workspace holds.
fn repo_of(scratch: &Scratch) -> PathBuf {
    scratch.path().join("repositories/work")
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
    fn open(root: &Path) -> Self {
        let workspace = Workspace::at(root);
        let records_root = workspace.records();
        let app = super::open(&workspace, &records_root).expect("the browser opens");
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

    /// A browser chord, held with control, which reaches every chord by default.
    fn chord(&mut self, c: char) -> &mut Self {
        handle(
            &mut self.app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
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
    /// Puts this thread in the mode that runs.
    ///
    /// A thread opens in ask. A journey about starting, stopping or leaving a
    /// run says so here, the way an operator would with shift-tab, rather than
    /// relying on whichever mode happens to be the default.
    fn auto(&mut self) -> &mut Self {
        self.app.thread_mut().mode = crate::mode::Mode::Auto;
        self
    }

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
    ///
    /// Said rather than assumed: a thread opens in ask, where enter answers a
    /// question and changes nothing. These journeys are about runs, so they
    /// put the thread in the mode that runs, the way an operator would.
    fn task(&mut self, text: &str) -> &mut Self {
        self.app.thread_mut().mode = crate::mode::Mode::Auto;
        let done = self.app.thread().turns.len() + 1;
        self.typed(text).key(KeyCode::Enter);
        self.until("the run to finish", move |app| {
            app.thread().turns.len() == done
        })
    }

    /// The last turn in the thread.
    fn last(&self) -> &super::thread::Turn {
        self.app.thread().turns.last().expect("a finished turn")
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
    let error = super::run(&Workspace::at(scratch.path())).expect_err("a pipe is not a terminal");
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
    std::fs::create_dir_all(scratch.path().join("repositories/work")).expect("repository");
    std::fs::write(
        scratch.path().join("repositories/work/Cargo.toml"),
        "[package]\n",
    )
    .expect("write");
    let mut d = Driver::open(scratch.path());

    d.shows("not an Ostraka project yet");
    d.shows("a Rust project");
    d.shows("i set this directory up");

    // Not offered here, and saying why beats a key that quietly does nothing.
    d.typed("a task this directory cannot take")
        .key(KeyCode::Enter);
    d.shows("cannot run anything yet");
    assert!(d.app.thread().live.is_none());

    d.chord('x').key(KeyCode::Char('i'));
    d.hides("not an Ostraka project yet");
    d.shows("Ask anything about this repository");
    assert!(
        scratch
            .path()
            .join(".ostraka/adapters/codex.toml")
            .is_file()
    );
}

#[test]
fn onboarding_an_empty_workspace_says_what_is_missing_before_it_is_missed() {
    // A workspace is set up before anything is cloned into it, so the first
    // thing wrong is that there is nothing to work on — and the second, once
    // there is, is whether git knows about it. Both used to be found out from
    // inside a run, in somebody else's words, after a vendor had been paid.
    let scratch = Scratch::new("no-git");
    let mut d = Driver::open(scratch.path());

    d.shows("not an Ostraka project yet");
    // An empty workspace's problem is not git yet — it is that there is
    // nothing to work on, and that is the sentence it gets.
    d.shows("Nothing has been cloned into");
    d.shows("x walks through it");
    // And no recognisable toolchain either, so the gate it would write is a
    // placeholder. Better said before the offer is taken.
    d.shows("fails on purpose");

    // Taking the offer does not make the warning untrue, so it stays.
    d.chord('x').key(KeyCode::Char('i'));
    assert!(scratch.path().join(".ostraka/ostraka.toml").is_file());
    assert!(
        d.app.blocked.is_some(),
        "the warning went away without the cause"
    );
}

#[test]
fn a_task_in_a_repository_git_does_not_know_offers_the_steps_out_of_it() {
    // What this replaces: the run started, spent a vendor, and came back with
    // "did not finish — git worktree add failed: fatal: not a git repository",
    // which is true, is git's account from two layers down, and leaves the
    // operator to work out both that the answer is `git init` and that `git
    // init` alone is not enough either.
    let scratch = Scratch::new("guided");
    // A workspace with something cloned in that git has never heard of, which
    // is what an operator who copied a directory rather than cloning one has.
    std::fs::create_dir_all(scratch.path().join("repositories/work")).expect("repository");
    let mut d = Driver::open(scratch.path());
    d.chord('x').key(KeyCode::Char('i'));

    d.typed("write a file").key(KeyCode::Enter);
    assert!(
        d.app.thread().live.is_none(),
        "a run was started with nowhere to work"
    );
    assert_eq!(d.app.dialog, Some(Dialog::Fix));
    assert_eq!(
        d.app.pane().prompt,
        "write a file",
        "the task was thrown away"
    );
    d.shows("not a git repository");
    d.shows("git init");

    // Step by step, and only when asked.
    d.key(KeyCode::Char('y'));
    assert!(
        scratch.path().join("repositories/work/.git").is_dir(),
        "step one did nothing"
    );
    d.shows("commits everything");

    // The commit needs an author, and a machine running these may have none
    // configured; that is the repository's business rather than this test's.
    for (key, value) in [
        ("user.email", "flow@example.invalid"),
        ("user.name", "flow"),
    ] {
        assert!(
            Command::new("git")
                .args(["config", key, value])
                .current_dir(scratch.path().join("repositories/work"))
                .status()
                .expect("git runs")
                .success()
        );
    }

    d.key(KeyCode::Char('y'));
    assert!(
        d.app.remedy.as_ref().is_some_and(|r| r.done()),
        "the steps did not finish"
    );
    d.key(KeyCode::Esc);

    d.chord('x').key(KeyCode::Char('x'));
    d.shows("nothing is in the way");
    assert!(d.app.blocked.is_none());
}

#[test]
fn a_workspace_with_nothing_in_it_can_start_a_repository_and_work_in_it() {
    // The whole first five minutes when there is nothing to clone: set the
    // place up, start something, and be walked through the one thing `git
    // init` does not do — the commit a worktree branches from.
    let scratch = Scratch::new("start-here");
    let mut d = Driver::open(scratch.path());
    d.chord('x').key(KeyCode::Char('i'));
    d.shows("Nothing has been cloned into");

    d.chord('x').key(KeyCode::Char('w'));
    assert_eq!(d.app.dialog, Some(Dialog::Repos));
    d.shows("start one here");

    d.key(KeyCode::Char('n')).typed("fresh").key(KeyCode::Enter);
    assert!(scratch.path().join("repositories/fresh/.git").is_dir());
    assert_eq!(
        d.app.repository().map(|r| r.name.clone()),
        Some("fresh".into())
    );

    // Started, not finished: a worktree needs a commit to branch from, and the
    // guided fix is what asks before making one.
    assert!(
        d.app.blocked.is_some(),
        "a repository with no commits read as ready"
    );
    d.chord('x').key(KeyCode::Char('x'));
    d.shows("no commits");
    d.shows("Commit what is here");
}

#[test]
fn a_task_typed_into_the_box_runs_and_is_recorded() {
    let _guard = exclusive();
    let scratch = project("run", 0);
    let mut d = Driver::open(scratch.path());
    d.shows("Ask anything about this repository");
    // Nothing to summarise yet, which is the honest version of that sentence.
    d.hides("Last asked here");

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

    assert_eq!(d.app.thread().base_ref, "HEAD");
    d.task("write a file");
    let first = d
        .last()
        .finished
        .as_ref()
        .map(|f| f.run_id.clone())
        .expect("a finished run");
    assert!(d.last().approved(), "the first run was not approved");
    assert_eq!(d.app.thread().base_ref, format!("ostraka/{first}"));
    // And it says so, where you are rather than buried in a menu.
    d.shows("on ");

    d.task("write it again");
    assert_eq!(d.app.thread().turns.len(), 2);

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
        .current_dir(repo_of(&scratch))
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
        scratch.path().join(".ostraka/ostraka.toml"),
        "[gate]\nchecks = [{ name = \"check\", cmd = \"false\", required = true }]\n\n\
         [gate.review]\nmust_differ_from_author = true\n",
    )
    .expect("config");
    let mut d = Driver::open(scratch.path());

    d.task("write a file");
    assert!(!d.last().approved());
    assert_eq!(
        d.app.thread().base_ref,
        "HEAD",
        "the chain advanced through a refusal"
    );
    d.shows("FAIL");
}

#[test]
fn the_command_line_continues_a_run_the_way_the_browser_does() {
    // Two paths, one pipeline — the claim `run::execute` makes in its own doc
    // comment. The browser threads by advancing `base_ref` on approval; `--from`
    // is that rule with a run id in front of it, so the piece of work that took
    // a browser can be done by a script or by CI.
    //
    // Before it, continuing meant passing `--base-ref ostraka/<id>`: a naming
    // convention that appears nowhere but the source.
    let _guard = exclusive();
    let scratch = project("cli-thread", 0);
    let workspace = Workspace::at(scratch.path());

    let first = crate::run::execute(
        &workspace,
        &crate::run::Args::for_task("write a file".into()),
        None,
        &ostraka_adapter::interrupt::Stop::new(),
    )
    .expect("the first run completes");
    assert!(first.approved(), "the first run was not approved");

    let mut args = crate::run::Args::for_task("write it again".into());
    args.from = Some(first.record.run_id.clone());
    let second = crate::run::execute(
        &workspace,
        &args,
        None,
        &ostraka_adapter::interrupt::Stop::new(),
    )
    .expect("the second run completes");
    assert!(second.approved(), "the second run was not approved");

    // The proof is in git, the same proof the browser's thread test takes: the
    // second run's branch has the first run's commit behind it.
    let out = Command::new("git")
        .args([
            "log",
            "--format=%H",
            &format!("ostraka/{}", second.record.run_id),
        ])
        .current_dir(repo_of(&scratch))
        .output()
        .expect("git log");
    let commits = String::from_utf8_lossy(&out.stdout).lines().count();
    assert_eq!(commits, 3, "the chain did not build on the first run");
}

#[test]
fn the_command_line_will_not_continue_a_refused_run() {
    // The browser's rule, held to on the other path. Building on a change the
    // gate would not take is a way of taking it, and the command line must not
    // be the way around a rule the browser enforces.
    //
    // The failure this prevents is quiet rather than loud: a refused run *has*
    // a branch — made before the agent started — whose head is the commit it
    // branched from, so `--base-ref ostraka/<refused>` succeeds and starts
    // somewhere else entirely.
    let _guard = exclusive();
    let scratch = project("cli-refused", 0);
    std::fs::write(
        scratch.path().join(".ostraka/ostraka.toml"),
        "[gate]\nchecks = [{ name = \"check\", cmd = \"false\", required = true }]\n\n\
         [gate.review]\nmust_differ_from_author = true\n",
    )
    .expect("config");
    let workspace = Workspace::at(scratch.path());

    let refused = crate::run::execute(
        &workspace,
        &crate::run::Args::for_task("write a file".into()),
        None,
        &ostraka_adapter::interrupt::Stop::new(),
    )
    .expect("the run completes");
    assert!(!refused.approved(), "the gate let it through");

    let mut args = crate::run::Args::for_task("carry on".into());
    args.from = Some(refused.record.run_id.clone());
    // `RunReport` is not `Debug`, so the refusal is taken by hand rather than
    // through `expect_err`.
    let refusal = match crate::run::execute(
        &workspace,
        &args,
        None,
        &ostraka_adapter::interrupt::Stop::new(),
    ) {
        Ok(_) => panic!("a refused run was continued"),
        Err(e) => e,
    };
    let said = refusal.to_string();
    assert!(
        said.contains("not a base to build on"),
        "the reason was not given: {said}"
    );
}

#[test]
fn continuing_a_run_nobody_recorded_says_so() {
    let _guard = exclusive();
    let scratch = project("cli-nosuch", 0);
    let workspace = Workspace::at(scratch.path());
    let mut args = crate::run::Args::for_task("carry on".into());
    args.from = Some("t-never-happened".into());
    let refusal = match crate::run::execute(
        &workspace,
        &args,
        None,
        &ostraka_adapter::interrupt::Stop::new(),
    ) {
        Ok(_) => panic!("a run nobody recorded was continued"),
        Err(e) => e,
    };
    assert!(
        refusal.to_string().contains("no run"),
        "{}",
        refusal.to_string()
    );
}

#[test]
fn choosing_an_agent_the_workspace_has_not_written_writes_it_first() {
    // Without the profile the name would be set and the run could not resolve
    // it: routing reads `adapters/` and nothing else, so being offered
    // something that fails when it is taken is worse than not being offered it.
    //
    // The list is seeded rather than discovered. Discovery asks the machine
    // what is installed, and a test that asks the machine is a test that does
    // nothing on a machine with nothing installed — which is every CI runner
    // this has. The earlier version of this test returned early there, passing
    // without exercising a line of what it was written for.
    let _guard = exclusive();
    let scratch = project("agents-adopt", 0);
    let dir = scratch.path();
    let mut d = Driver::open(dir);

    d.chord('x').key(KeyCode::Char('a'));
    assert_eq!(d.app.dialog, Some(Dialog::Agents));

    // A profile this build ships, which this workspace does not have. `codex`
    // is one of `init::TEMPLATES`, so writing it needs nothing installed.
    d.app.agents.push(super::view::Agent {
        id: "codex".into(),
        ready: true,
        note: "codex-cli 0.0.0".into(),
        configured: false,
    });
    d.app.pick = d.app.agents.len() - 1;
    assert!(
        !dir.join(".ostraka/adapters/codex.toml").exists(),
        "the fixture already had the profile this is about writing"
    );
    d.shows("not configured");

    d.key(KeyCode::Char('a'));
    assert_eq!(d.app.thread().adapter.as_deref(), Some("codex"));
    assert!(
        dir.join(".ostraka/adapters/codex.toml").is_file(),
        "the profile was named but never written"
    );

    // The bytes are the ones `init` ships, not an invention.
    let written = std::fs::read_to_string(dir.join(".ostraka/adapters/codex.toml")).expect("read");
    let shipped = crate::init::TEMPLATES
        .iter()
        .find(|(name, _)| *name == "codex.toml")
        .map(|(_, text)| *text)
        .expect("shipped");
    assert_eq!(written, shipped);

    // And it is one of the workspace's own now, not an offer.
    let now = d
        .app
        .agents
        .iter()
        .find(|a| a.id == "codex")
        .expect("still listed");
    assert!(
        now.configured,
        "the written profile is still offered as missing"
    );
}

#[test]
fn only_what_is_installed_is_ever_offered() {
    // The list is an offer. Anything in it that is not installed is a choice
    // that fails when it is taken, so the filter is the property worth pinning
    // — and it holds whatever this machine happens to have.
    let _guard = exclusive();
    let scratch = project("agents-offer", 0);
    let mut d = Driver::open(scratch.path());
    d.chord('x').key(KeyCode::Char('a'));

    assert!(
        d.app.agents.iter().any(|a| a.configured),
        "the workspace's own profiles are missing from its own list"
    );
    assert!(
        d.app
            .agents
            .iter()
            .filter(|a| !a.configured)
            .all(|a| a.ready),
        "something not installed was offered"
    );
}

#[test]
fn the_browser_shows_what_a_run_left_before_it_removes_any_of_it() {
    // A list before an action, the way the command line is a dry run before
    // `--apply`. Removing a directory is not a keystroke to offer without
    // saying what it will take.
    let _guard = exclusive();
    let scratch = project("prune", 0);
    let mut d = Driver::open(scratch.path());

    // A refused run keeps its worktree on purpose — it is the evidence — and
    // that is exactly what fills a disk later.
    std::fs::write(
        scratch.path().join(".ostraka/ostraka.toml"),
        "[gate]\nchecks = [{ name = \"check\", cmd = \"false\", required = true }]\n\n\
         [gate.review]\nmust_differ_from_author = true\n",
    )
    .expect("config");
    d.task("write a file");
    assert!(!d.last().approved(), "the gate let it through");

    d.chord('x').key(KeyCode::Char('u'));
    assert_eq!(d.app.dialog, Some(Dialog::Prune));
    assert_eq!(
        d.app.leftovers.len(),
        1,
        "the refused run's worktree is not listed"
    );
    let left = d.app.leftovers[0].path.clone();
    assert!(left.is_dir(), "the fixture is wrong: nothing is there");
    // The thing somebody is actually afraid of, said before the key is pressed.
    d.shows("Branches and run records are untouched");

    // Esc leaves it alone.
    d.key(KeyCode::Esc);
    assert!(left.is_dir(), "esc removed something");

    d.chord('x').key(KeyCode::Char('u'));
    d.key(KeyCode::Char('y'));
    assert!(!left.exists(), "y did not remove the worktree");
    assert_eq!(d.app.dialog, None);

    // The branch the run made is still there, which is the whole posture: a
    // checkout can be made again and a commit cannot.
    let run_id = d
        .last()
        .finished
        .as_ref()
        .map(|f| f.run_id.clone())
        .expect("a finished run");
    let out = Command::new("git")
        .args(["rev-parse", "--verify", &format!("ostraka/{run_id}")])
        .current_dir(repo_of(&scratch))
        .output()
        .expect("git");
    assert!(out.status.success(), "the branch went with the worktree");
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
    d.app.thread_mut().adapter = Some("writer".into());
    d.app.thread_mut().review_adapter = Some("reader".into());

    d.task("write something long");

    assert!(!d.last().approved(), "a half-written change was approved");
    assert_eq!(
        d.app.thread().base_ref,
        "HEAD",
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
    let left = std::fs::read_dir(scratch.path().join(".ostraka/worktrees"))
        .expect("a worktrees directory")
        .filter_map(|e| e.ok())
        .find(|e| e.path().join("wrote.txt").is_file());
    assert!(left.is_some(), "the evidence was thrown away");
}

#[test]
fn an_author_out_of_credit_is_offered_a_profile_that_answers_and_the_task_runs_again() {
    let _guard = exclusive();
    let scratch = project("fallback", 0);
    let dir = scratch.path();
    // A vendor whose account is empty: nothing written, a non-zero exit, and
    // its reason on stderr.
    std::fs::write(
        dir.join("writer.sh"),
        "#!/bin/sh\n\
         case \"$1\" in --probe) echo ok; exit 0;; esac\n\
         echo 'Error: insufficient credit on this account' >&2\n\
         exit 1\n",
    )
    .expect("script");
    // A third profile that writes like the fixture's own.
    std::fs::copy(dir.join("reader.sh"), dir.join("spare.sh")).expect("spare");
    std::fs::write(
        dir.join(".ostraka/adapters/spare.toml"),
        format!(
            "id = \"spare\"\ncommand = \"{}\"\nargs = [\"{{{{prompt}}}}\"]\nprobe_args = [\"--probe\"]\n",
            dir.join("spare.sh").display()
        ),
    )
    .expect("profile");

    let mut d = Driver::open(dir);
    d.app.thread_mut().adapter = Some("writer".into());
    d.app.thread_mut().review_adapter = Some("reader".into());
    d.task("write a file");
    assert!(!d.last().approved());

    d.until("the other profiles to answer", |app| {
        app.dialog == Some(Dialog::Fallback) && app.agents_loading.is_none()
    });
    d.shows("writer could not run");
    d.shows("insufficient credit");
    // Neither the profile that failed nor the reviewer it would have had.
    let offered: Vec<String> = d.app.agents.iter().map(|a| a.id.clone()).collect();
    assert!(!offered.contains(&"writer".to_string()), "{offered:?}");
    assert!(!offered.contains(&"reader".to_string()), "{offered:?}");
    let spare = offered
        .iter()
        .position(|id| id == "spare")
        .expect("the spare profile was offered");
    for _ in 0..spare {
        d.key(KeyCode::Down);
    }

    let done = d.app.thread().turns.len() + 1;
    d.key(KeyCode::Enter);
    assert_eq!(d.app.thread().adapter.as_deref(), Some("spare"));
    d.until("the task to run again", move |app| {
        app.thread().turns.len() == done
    });
    assert!(d.last().approved(), "the task did not run again with spare");
    assert_eq!(d.last().prompt, "write a file");
}

#[test]
fn a_run_can_be_stopped_from_the_browser_and_is_not_called_a_verdict() {
    let _guard = exclusive();
    let scratch = project("stop", 30);
    let mut d = Driver::open(scratch.path());

    d.auto()
        .typed("a task nobody wants finished")
        .key(KeyCode::Enter);
    // In the same breath as starting it, which is the ordering that used to
    // lose the request to the run clearing the flag behind it.
    d.chord('x').key(KeyCode::Char('s'));
    assert!(d.app.thread().live.as_ref().expect("a session").stopping);
    d.shows("stopping");

    d.until("the run to stop", |app| app.thread().turns.len() == 1);
    d.shows("stopped by the operator");
    ostraka_adapter::interrupt::clear();
}

#[test]
fn two_panes_run_at_once_and_stopping_one_leaves_the_other_going() {
    // The journey the per-run stop exists for: real runs, in two panes, at the
    // same time, and `s` asking only the pane in front.
    let _guard = exclusive();
    let scratch = project("two-panes", 30);
    let mut d = Driver::open(scratch.path());

    d.auto().typed("the first task").key(KeyCode::Enter);
    assert!(d.app.thread().running(), "the first run did not start");

    // A new pane is a new thread, and a new thread opens in ask.
    d.chord('t');
    d.auto().typed("the second task").key(KeyCode::Enter);
    assert!(
        d.app.thread().running(),
        "a second pane could not start a run beside the first: {:?}",
        d.app.status
    );
    assert_eq!(
        d.app.panes.iter().filter(|p| p.thread.running()).count(),
        2,
        "two runs were not going at once"
    );

    // Stop the pane in front, and only it.
    d.chord('x').key(KeyCode::Char('s'));
    d.until("the second run to stop", |app| {
        app.thread().turns.len() == 1
    });
    d.shows("stopped by the operator");
    assert!(
        d.app.panes[0].thread.running(),
        "stopping one pane stopped the other"
    );
    assert!(
        !d.app.panes[0]
            .thread
            .live
            .as_ref()
            .expect("a session")
            .stopping,
        "the other pane was asked to stop too"
    );

    // And the first stops on its own when it is asked.
    d.chord(']');
    d.chord('x').key(KeyCode::Char('s'));
    d.until("the first run to stop", |app| {
        app.panes[0].thread.turns.len() == 1
    });
}

#[test]
fn quitting_during_a_run_waits_for_it_rather_than_walking_away() {
    // A process that exited here would leave a vendor writing into a worktree.
    let _guard = exclusive();
    let scratch = project("quit", 30);
    let mut d = Driver::open(scratch.path());

    d.auto()
        .typed("a task interrupted by leaving")
        .key(KeyCode::Enter);
    d.ctrl('c');
    // Asked first. It says what leaving costs before it costs it.
    assert_eq!(d.app.dialog, Some(Dialog::Leaving));
    d.shows("A run is going");
    d.key(KeyCode::Char('y'));

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

    d.chord('x').key(KeyCode::Char('l'));
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

    d.chord('x').key(KeyCode::Char('a'));
    assert_eq!(d.app.dialog, Some(Dialog::Agents));
    d.shows("automatic");
    d.shows("writer");
    d.shows("reader");

    d.key(KeyCode::Char('a'));
    d.shows("writes");
    assert!(d.app.thread().adapter.is_some());
    d.key(KeyCode::Down).key(KeyCode::Char('r'));
    assert!(d.app.thread().review_adapter.is_some());
    assert_ne!(d.app.thread().adapter, d.app.thread().review_adapter);
    d.key(KeyCode::Esc);

    // And a run started afterwards is run by the pair that was named.
    let author = d.app.thread().adapter.clone().expect("an author");
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
    assert!(d.app.thread().continuing());

    d.chord('x').key(KeyCode::Char('f'));
    assert_eq!(d.app.thread().base_ref, "HEAD");
    assert!(d.app.thread().turns.is_empty());
    // The thread is empty; the directory is not, and the screen says which.
    d.shows("Ask anything about this repository");
    d.shows("Last asked here");
    d.shows("write a file");
}

#[test]
fn the_palette_reaches_a_command_by_name_and_the_leader_by_letter() {
    let scratch = Scratch::new("commands");
    std::fs::create_dir_all(scratch.path().join("repositories/work")).expect("repository");
    std::fs::write(
        scratch.path().join("repositories/work/Cargo.toml"),
        "[package]\n",
    )
    .expect("write");
    let mut d = Driver::open(scratch.path());

    d.chord('k').typed("keys").key(KeyCode::Enter);
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.shows("what you type is the task");
    d.shows(label(Chord::Leader));

    // It lists every command, which is more than fits, so it scrolls rather
    // than stopping at whatever the terminal happened to have room for.
    d.shows("more lines below");
    d.key(KeyCode::End);
    assert_eq!(d.app.dialog, Some(Dialog::Keys), "scrolling closed it");
    // The last section, so it really reached the end.
    d.shows("getting around");
    // And the way out is on the screen wherever it has been scrolled to.
    d.shows("esc closes this");
    d.key(KeyCode::Esc);

    d.chord('x').key(KeyCode::Char('h'));
    assert_eq!(d.app.dialog, Some(Dialog::Keys));
    d.key(KeyCode::Esc);
    assert_eq!(d.app.dialog, None);
}

/// Dragging over a transcript selects it, and the selection can be copied.
///
/// The mouse is captured, so the terminal's own selection is not available —
/// and would be the wrong selection anyway once panes sit side by side. This
/// drives the real mouse handler and asserts against the drawn buffer, which
/// is where a selection either shows or does not.
mod selecting {
    use super::*;
    use crate::tui::select;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::style::Modifier;

    impl Driver {
        fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> &mut Self {
            // Drawn first: a drag is read against where the transcript was
            // last put on the screen, which is a thing only drawing knows.
            self.screen();
            super::super::mouse(
                &mut self.app,
                MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
            );
            self
        }

        /// A press, a drag and a release across one row of a transcript.
        fn drag(&mut self, row: u16, from: u16, to: u16) -> &mut Self {
            self.mouse(MouseEventKind::Down(MouseButton::Left), from, row)
                .mouse(MouseEventKind::Drag(MouseButton::Left), to, row)
                .mouse(MouseEventKind::Up(MouseButton::Left), to, row)
        }

        /// Where the transcript of the pane in front was drawn.
        fn body(&mut self) -> ratatui::layout::Rect {
            self.screen();
            let at = self.app.at;
            self.app
                .transcripts
                .iter()
                .find(|(index, _, _)| *index == at)
                .map(|(_, area, _)| *area)
                .expect("the transcript was drawn")
        }

        /// Every cell the last draw reversed, read off the buffer.
        fn reversed(&mut self) -> String {
            let mut terminal =
                Terminal::new(TestBackend::new(self.width, self.height)).expect("test terminal");
            let app = &mut self.app;
            terminal
                .draw(|frame| super::super::view::draw(frame, app))
                .expect("draws");
            let buffer = terminal.backend().buffer().clone();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        out.push_str(cell.symbol());
                    }
                }
            }
            out
        }
    }

    #[test]
    fn dragging_over_a_transcript_selects_what_it_covered_and_copying_takes_it() {
        let _guard = exclusive();
        let scratch = project("select", 0);
        let mut driver = Driver::open(scratch.path());
        driver.task("write the file");

        // The first row of the transcript is the task, written back as it was
        // asked. Somewhere to drag over that is the same every run.
        let body = driver.body();
        let top = body.y;
        driver.drag(top, body.x, body.x + 5);

        let selection = driver.app.selection.expect("a selection was made");
        assert!(!selection.dragging, "the button was released");
        assert_eq!(selection.pane, driver.app.at);

        // What is on the screen, not what the state says about it.
        let reversed = driver.reversed();
        assert!(
            !reversed.is_empty(),
            "nothing was drawn as selected:\n{}",
            driver.screen()
        );
        let lines = super::super::view::transcript_of(&driver.app, driver.app.at);
        assert_eq!(select::text(&lines, &selection), reversed.trim_end());

        // And it is offered: a command nobody can find is a feature nobody has.
        driver.chord('k');
        driver.shows("copy what is selected");
        driver.key(KeyCode::Esc);

        // Copying says what it did. It cannot say the terminal took it —
        // OSC 52 has no reply — so it says what was asked.
        driver.ctrl('x').key(KeyCode::Char('y'));
        let said = driver.app.status.clone().unwrap_or_default();
        assert!(said.contains("clipboard"), "{said}");
    }

    #[test]
    fn a_click_that_selects_nothing_focuses_the_pane_and_leaves_no_highlight() {
        let _guard = exclusive();
        let scratch = project("select-click", 0);
        let mut driver = Driver::open(scratch.path());
        driver.task("write the file");

        let body = driver.body();
        driver
            .mouse(MouseEventKind::Down(MouseButton::Left), body.x + 2, body.y)
            .mouse(MouseEventKind::Up(MouseButton::Left), body.x + 2, body.y);
        assert!(driver.app.selection.is_none(), "a click selected something");
        assert!(driver.reversed().is_empty());
        assert_eq!(driver.app.focus, Focus::Prompt);
    }

    #[test]
    fn escape_drops_the_selection_rather_than_leaving_the_browser() {
        let _guard = exclusive();
        let scratch = project("select-escape", 0);
        let mut driver = Driver::open(scratch.path());
        driver.task("write the file");

        let body = driver.body();
        driver.drag(body.y, body.x, body.x + 5);
        assert!(driver.app.selection.is_some());

        driver.key(KeyCode::Esc);
        assert!(driver.app.selection.is_none(), "escape kept the selection");
        assert!(!driver.app.quit, "escape left the browser");
        assert!(driver.reversed().is_empty());
    }

    /// Raised in review: dragging past the top edge scrolls, and the head was
    /// read against the scroll captured before it — so the selection stayed a
    /// line behind the edge and never reached the line that had just been
    /// revealed, which is the only line that drag was for.
    #[test]
    fn dragging_past_the_top_edge_reaches_the_line_it_revealed() {
        let _guard = exclusive();
        let scratch = project("select-edge", 0);
        let mut driver = Driver::open(scratch.path());
        driver.task("write the file");
        driver.task("write it again");
        driver.task("and once more");

        let body = driver.body();
        // Somewhere in the middle, then dragged off the top of the transcript.
        driver.mouse(
            MouseEventKind::Down(MouseButton::Left),
            body.x,
            body.y + body.height / 2,
        );
        let scroll_before = driver.app.scroll;
        assert!(
            scroll_before > 0,
            "the transcript did not overflow the pane"
        );

        driver.mouse(
            MouseEventKind::Drag(MouseButton::Left),
            body.x,
            body.y.saturating_sub(1),
        );
        let selection = driver.app.selection.expect("a selection");
        assert_eq!(
            driver.app.scroll,
            scroll_before - 1,
            "dragging past the edge did not scroll"
        );
        let (from, _) = selection.range();
        assert_eq!(
            from.line,
            usize::from(driver.app.scroll),
            "the head stopped short of the line that was revealed"
        );
    }

    /// The selection lives in the transcript's own coordinates, so scrolling
    /// moves it with the text. Held in screen cells it would stay on the rows
    /// it was drawn over and come to cover something else entirely.
    #[test]
    fn scrolling_under_a_selection_moves_it_with_the_text() {
        let _guard = exclusive();
        let scratch = project("select-scroll", 0);
        let mut driver = Driver::open(scratch.path());
        driver.task("write the file");
        driver.task("write it again");

        let body = driver.body();
        driver.drag(body.y, body.x, body.x + 8);
        let before = driver.app.selection.expect("a selection");
        let text = select::text(
            &super::super::view::transcript_of(&driver.app, driver.app.at),
            &before,
        );

        driver.app.scroll_pane(driver.app.at, -2);
        let after = driver.app.selection.expect("still a selection");
        assert_eq!(before, after, "scrolling moved the selection itself");
        assert_eq!(
            select::text(
                &super::super::view::transcript_of(&driver.app, driver.app.at),
                &after
            ),
            text,
            "the selection came to cover different text"
        );
    }
}

/// Conversation first: a thread opens in ask, and running is an act somebody
/// takes on an answer they already have.
///
/// The habit this replaces cost real money. Opening in auto made the first
/// message a gated run, so a workspace whose gate `init` could not infer — the
/// check that fails on purpose — answered every question with a refusal, after
/// paying a vendor to get there.
mod conversation {
    use super::*;
    use crate::mode::Mode;
    use crate::tui::command::Command;

    #[test]
    fn a_thread_opens_in_ask_and_enter_changes_nothing() {
        let _guard = exclusive();
        let scratch = project("conversation", 0);
        let mut d = Driver::open(scratch.path());
        assert_eq!(
            d.app.thread().mode,
            Mode::Ask,
            "a thread opened in {:?}",
            d.app.thread().mode
        );
        d.shows("Ask anything about this repository");

        let done = d.app.thread().turns.len() + 1;
        d.typed("what does this repository do").key(KeyCode::Enter);
        d.until("the question to be answered", move |app| {
            app.thread().turns.len() == done
        });

        // A consultation has no outcome, because there was no change to judge.
        let turn = d.last();
        assert!(
            turn.finished.as_ref().is_some_and(|f| f.outcome.is_none()),
            "asking produced a verdict"
        );
        // And nothing was committed, so the next run still starts from HEAD.
        assert_eq!(d.app.thread().base_ref, "HEAD");
    }

    #[test]
    fn what_was_asked_can_be_run_without_being_typed_again() {
        let _guard = exclusive();
        let scratch = project("escalate", 0);
        let mut d = Driver::open(scratch.path());

        let asked = "write the file this task is about";
        let done = d.app.thread().turns.len() + 1;
        d.typed(asked).key(KeyCode::Enter);
        d.until("the question to be answered", move |app| {
            app.thread().turns.len() == done
        });

        assert_eq!(d.app.thread().ready.as_deref(), Some(asked));
        assert!(
            Command::offered(d.app.situation()).contains(&Command::Go),
            "going was not offered after a question"
        );
        d.shows("or e to have it done");

        // The same words, as a run. Not retyped, and not reworded.
        let done = d.app.thread().turns.len() + 1;
        d.chord('x').key(KeyCode::Char('e'));
        d.until("the run to finish", move |app| {
            app.thread().turns.len() == done
        });
        assert_eq!(d.last().prompt, asked);
        assert!(
            d.last()
                .finished
                .as_ref()
                .is_some_and(|f| f.outcome.is_some()),
            "going did not produce a run"
        );
        // Taken, so it is not offered again.
        assert!(d.app.thread().ready.is_none());
        assert!(!Command::offered(d.app.situation()).contains(&Command::Go));
    }

    /// Raised in review: the held task is what was typed, and a typed task can
    /// name agents with `@`. Passed through whole, those names reach the agent
    /// as part of the task and the profiles they chose are lost — so the
    /// escalation runs something other than what was asked.
    #[test]
    fn going_reads_the_agents_a_question_named_rather_than_passing_them_on() {
        let _guard = exclusive();
        let scratch = project("escalate-mentions", 0);
        let mut d = Driver::open(scratch.path());

        let done = d.app.thread().turns.len() + 1;
        d.typed("@writer why is this file here").key(KeyCode::Enter);
        d.until("the question to be answered", move |app| {
            app.thread().turns.len() == done
        });
        assert_eq!(
            d.app.thread().ready.as_deref(),
            Some("@writer why is this file here")
        );

        let done = d.app.thread().turns.len() + 1;
        d.chord('x').key(KeyCode::Char('e'));
        d.until("the run to finish", move |app| {
            app.thread().turns.len() == done
        });

        // The transcript keeps what was typed, names and all.
        assert_eq!(d.last().prompt, "@writer why is this file here");
        // The run itself was given the task without them, and the profile the
        // name chose did the writing.
        let record = d
            .last()
            .finished
            .as_ref()
            .map(|f| f.run_id.clone())
            .expect("a finished run");
        let text = std::fs::read_to_string(
            scratch
                .path()
                .join(format!(".ostraka/runs/{record}/record.json")),
        )
        .expect("the record");
        assert!(
            text.contains("\"why is this file here\""),
            "the name was passed through as part of the task: {text}"
        );
        assert!(text.contains("\"writer\""), "{text}");
    }

    /// `init` writes a gate that fails on purpose where it could not tell how a
    /// project is verified. Finding that out from a refusal means the run was
    /// authored and a vendor was paid first.
    #[test]
    fn a_placeholder_gate_stops_a_run_before_it_costs_anything() {
        let _guard = exclusive();
        let scratch = project("placeholder-gate", 0);
        std::fs::write(
            scratch.path().join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"declare-your-checks\", \
             cmd = \"echo no >&2; exit 1\", required = true }]\n",
        )
        .expect("config");

        let mut d = Driver::open(scratch.path());

        // Asking still works. A question is not gated, so a workspace with no
        // gate yet is still one somebody can use.
        let done = d.app.thread().turns.len() + 1;
        d.typed("what is here").key(KeyCode::Enter);
        d.until("the question to be answered", move |app| {
            app.thread().turns.len() == done
        });

        // Running does not, and says so without starting anything.
        let before = d.app.thread().turns.len();
        d.auto().typed("change something").key(KeyCode::Enter);
        assert!(
            !d.app.thread().running(),
            "a run started against a gate that cannot pass"
        );
        assert_eq!(d.app.thread().turns.len(), before);
        let said = d.app.status.clone().unwrap_or_default();
        assert!(said.contains("declare-your-checks"), "{said}");
        assert!(said.contains("ask instead"), "{said}");
        // The task is still in the box, not thrown away.
        assert_eq!(d.app.pane().prompt, "change something");
    }
}

/// The agents beside the work: shown and hidden without taking the keys, and
/// fed by what the runtime writes while a run is going.
mod agents_beside {
    use super::*;

    #[test]
    fn toggling_the_agents_leaves_the_task_being_typed_where_it_was() {
        let _guard = exclusive();
        let scratch = project("agents-toggle", 0);
        let mut d = Driver::open(scratch.path());
        d.screen();
        assert!(d.app.side_shown, "a wide terminal did not show the agents");

        d.typed("half a ta");
        d.chord('x').key(KeyCode::Char('v'));
        assert!(!d.app.side, "the leader did not hide them");
        assert_eq!(d.app.focus, Focus::Prompt, "hiding them took the keys");
        d.typed("sk");
        assert_eq!(d.app.pane().prompt, "half a task");
        d.hides("writes next");

        d.chord('x').key(KeyCode::Char('v'));
        assert!(d.app.side);
        assert_eq!(d.app.focus, Focus::Prompt);
    }

    /// End to end: a run started in this browser is seen the way a `drain`
    /// worker in another shell would be — through the `live.json` the runtime
    /// writes beside it — and the profile writing it says so.
    #[test]
    fn a_run_in_progress_shows_who_is_writing_it() {
        let _guard = exclusive();
        let scratch = project("agents-live", 3);
        let mut d = Driver::open(scratch.path());
        d.screen();
        d.auto().typed("write the file").key(KeyCode::Enter);
        d.until("the pane to see the writer", |app| {
            app.live
                .iter()
                .any(|(_, live)| live.phase == ostraka_runtime::progress::Phase::Authoring)
        });
        let writer = d.app.live[0].1.author.clone();
        let out = d.screen();
        let row = out
            .lines()
            .skip_while(|line| !line.contains(writer.as_str()))
            .take(5)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            row.contains("writing"),
            "the writer is not shown writing:\n{out}"
        );

        // And once the run is over, nobody is still shown working on it.
        d.until("the run to finish", |app| !app.thread().running());
        d.until("the pane to catch up", |app| app.live.is_empty());
        d.hides("writing");
    }
}
