//! Asking and planning: an agent consulted, not a change made.
//!
//! A run ends in a change, so it is gated and reviewed. A question or a plan
//! ends in words, and there is nothing to gate. What still has to hold is that
//! nothing was written, and that is not taken on trust either.
//!
//! **Read-only by invocation, and checked from git.** The agent is launched with
//! its profile's review invocation, the one declared not to write. A profile
//! without one is refused by name instead of being asked through an invocation
//! that can write. Afterwards `git status` in the worktree is the answer, never
//! the agent's own account. A clean worktree is removed with its branch, because
//! there is no change in it for anybody to read. A dirty one is refused and
//! kept, because it is the evidence that an invocation declared read-only was
//! not.
//!
//! Consultations are recorded beside the runs, not among them, in
//! `.ostraka/consulted/`. The run list is a list of changes, and a question
//! listed there as "never finished" would be a record that lied.

use crate::mode::Mode;
use crate::run::{self, Args};
use crate::workspace::Workspace;
use ostraka_adapter::interrupt::Stop;
use ostraka_adapter::process::ProcessAdapter;
use ostraka_adapter::{Profile, VendorAdapter};
use ostraka_core::identity::ActorId;
use ostraka_core::record::Event;
use ostraka_core::task::TaskSpec;
use ostraka_runtime::progress::{Phase, Watcher};
use ostraka_runtime::record::RunLog;
use ostraka_runtime::worktree::{self, Worktree};
use std::path::{Path, PathBuf};
use std::time::Duration;

type Failure = Box<dyn std::error::Error>;

/// What came of consulting an agent.
pub struct Consulted {
    pub id: String,
    pub mode: Mode,
    pub adapter: String,
    /// Everything the agent said, in order.
    pub answer: String,
    /// What it wrote while it was only asked. Empty is the only acceptable
    /// answer, and anything else keeps the worktree.
    pub wrote: Vec<String>,
    pub worktree: PathBuf,
    pub exit_code: Option<i32>,
    pub diagnostics: Option<String>,
    pub stopped: bool,
}

impl Consulted {
    /// An answer worth acting on: it finished, and it wrote nothing.
    pub fn clean(&self) -> bool {
        self.wrote.is_empty() && self.exit_code == Some(0) && !self.stopped
    }

    pub fn summary(&self) -> String {
        if !self.wrote.is_empty() {
            return format!(
                "refused \u{2014} {} wrote {} while it was only asked; the worktree is kept at {}",
                self.adapter,
                self.wrote.join(", "),
                self.worktree.display()
            );
        }
        if self.stopped {
            return "stopped by the operator".to_string();
        }
        if self.exit_code != Some(0) {
            return match &self.diagnostics {
                Some(said) => format!("{} could not answer: {said}", self.adapter),
                None => format!("{} could not answer, and said nothing", self.adapter),
            };
        }
        match self.mode {
            Mode::Plan => "planned \u{2014} nothing written".to_string(),
            _ => "answered \u{2014} nothing written".to_string(),
        }
    }
}

/// The profile to consult: the one named, or the first that answers.
///
/// Only a profile with a read-only invocation can be asked anything. The rule is
/// the one routing already applies to reviewers, except here it refuses rather
/// than orders: a reviewer that writes is caught by the tree check before a
/// commit, but a question has no commit to catch it before.
fn reader(
    profiles: &[Profile],
    named: Option<&str>,
    preferred: &[String],
) -> Result<Profile, Failure> {
    if let Some(id) = named {
        let Some(profile) = profiles.iter().find(|p| p.id == id) else {
            return Err(format!("no adapter profile with id {id:?}").into());
        };
        if profile.review_args.is_none() {
            return Err(format!(
                "{id} has no read-only invocation (`review_args`), so it cannot be asked \
                 anything without being able to write; name a profile that has one"
            )
            .into());
        }
        return Ok(profile.clone());
    }
    // The workspace's reviewers first: the reason to prefer a reviewer, that it
    // is handed the repository's own rules, is the reason to prefer it here.
    if let Some(id) = run::preferred_reviewer(preferred, profiles, None, |p| {
        p.review_args.is_some() && ProcessAdapter::new(p.clone()).probe().is_ready()
    }) {
        if let Some(profile) = profiles.iter().find(|p| p.id == id) {
            return Ok(profile.clone());
        }
    }
    let mut readers: Vec<&Profile> = profiles
        .iter()
        .filter(|p| p.review_args.is_some())
        .collect();
    readers.sort_by(|a, b| a.id.cmp(&b.id));
    readers
        .into_iter()
        .find(|p| ProcessAdapter::new((*p).clone()).probe().is_ready())
        .cloned()
        .ok_or_else(|| {
            "no profile here has a read-only invocation that answers, and asking or \
             planning needs one"
                .into()
        })
}

/// Removes a worktree nothing was written in, and the branch made for it.
fn discard(repo: &Path, wt: &Worktree) {
    let _ = worktree::release(repo, wt);
    let _ = std::process::Command::new("git")
        .args(["branch", "-D", wt.branch()])
        .current_dir(repo)
        .output();
}

/// Consults an agent about `args.prompt` in a worktree of its own.
pub fn consult(
    workspace: &Workspace,
    args: &Args,
    mode: Mode,
    watcher: Option<Box<dyn Watcher>>,
    stop: &Stop,
) -> Result<Consulted, Failure> {
    let repo = workspace.repository(args.repository.as_deref())?;
    let config = workspace.config_for(&repo)?;
    config.validate()?;
    let profiles = workspace.profiles()?;
    let profile = reader(&profiles, args.adapter.as_deref(), &workspace.reviewers())?;

    let id = format!("{}-{}", mode.word(), run::task_id());
    let mut log = RunLog::create(&workspace.ostraka().join("consulted"), &id)?.watched_by(watcher);
    log.enter(Phase::Isolating);
    let worktrees = workspace.worktrees(&config);
    let wt = worktree::create(&repo.path, &worktrees, &id, &args.base_ref)?;

    log.enter(Phase::Preparing);
    let notes = workspace.notes_if_present();
    let skills = workspace.skills_if_present();
    if let Err(problem) = worktree::prepare_until(
        &repo.path,
        wt.path(),
        &config.worktree,
        notes.as_deref(),
        skills.as_deref(),
        config.gate.timeout_secs.map(Duration::from_secs),
        stop,
    ) {
        discard(&repo.path, &wt);
        return Err(format!(
            "the worktree could not be prepared ({}): {}",
            problem.step, problem.reason
        )
        .into());
    }

    log.enter(Phase::Authoring);
    let adapter = ProcessAdapter::reviewing(profile.clone())
        .isolated_under(workspace.ostraka().join("vendor-home"))
        .within(config.policy.timeout_secs.map(Duration::from_secs))
        .stopped_by(stop.clone());
    let spec = TaskSpec {
        id: id.clone(),
        prompt: mode.consulting(&args.prompt),
        adapter: profile.id.clone(),
        author: ActorId::new(run::identity(&args.author, run::AUTHOR, &profile.id)),
        base_ref: args.base_ref.clone(),
        model: args.model.clone(),
    };
    let mut session = match adapter.launch(&spec, wt.path()) {
        Ok(session) => session,
        Err(e) => {
            discard(&repo.path, &wt);
            return Err(e.into());
        }
    };

    let mut answer = String::new();
    while let Some(event) = session.next_event() {
        if let Event::Message { text, .. } = &event {
            answer.push_str(text);
            answer.push('\n');
        }
        log.append(&event)?;
    }
    let outcome = session.finish();
    if let Some(said) = &outcome.diagnostics {
        log.append(&Event::Error {
            message: format!("{} exited abnormally: {said}", profile.id),
            raw: None,
        })?;
    }

    let wrote = worktree::touched_paths(wt.path())?;
    log.append(&Event::Finished {
        exit_code: outcome.exit_code,
        files_touched: wrote.clone(),
    })?;
    if wrote.is_empty() {
        discard(&repo.path, &wt);
    }

    Ok(Consulted {
        id,
        mode,
        adapter: profile.id,
        answer: answer.trim_end().to_string(),
        wrote,
        worktree: wt.path().to_path_buf(),
        exit_code: outcome.exit_code,
        diagnostics: outcome.diagnostics,
        stopped: outcome.interrupted || stop.requested(),
    })
}

/// `ostraka run --mode ask` and `--mode plan`.
///
/// A plan agreed at a terminal becomes a run, through [`run::run`] like any
/// other. Anywhere nobody can answer, the plan is printed and nothing runs:
/// a plan nobody agreed to is not a plan anybody agreed to.
pub fn run(workspace: &Workspace, args: &Args, mode: Mode, json: bool) -> Result<bool, Failure> {
    let interrupts = std::sync::atomic::AtomicUsize::new(0);
    let _ = ctrlc::set_handler(move || {
        if interrupts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            eprintln!("\nstopping \u{2014} press Ctrl-C again to give up on it");
            ostraka_adapter::interrupt::request();
        } else {
            std::process::exit(130);
        }
    });

    let consulted = consult(workspace, args, mode, None, &Stop::new())?;
    if json {
        let out = serde_json::json!({
            "mode": mode.word(),
            "id": consulted.id,
            "adapter": consulted.adapter,
            "answer": consulted.answer,
            "wrote": consulted.wrote,
            "exit_code": consulted.exit_code,
            "clean": consulted.clean(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(consulted.clean());
    }

    if !consulted.answer.is_empty() {
        println!("{}", consulted.answer);
    }
    if !consulted.clean() {
        eprintln!("{}", consulted.summary());
        return Ok(false);
    }
    if mode != Mode::Plan {
        return Ok(true);
    }
    if !crate::offer::at_a_terminal() {
        eprintln!("planned \u{2014} nothing was run, because nobody here can agree to it");
        return Ok(true);
    }

    eprint!("\nRun this plan now? It is gated and reviewed like any run. [y/N] ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
        eprintln!("nothing was run");
        return Ok(true);
    }
    let mut planned = args.clone();
    planned.prompt = Mode::planned(&args.prompt, &consulted.answer);
    run::run(workspace, &planned, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn git(repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// A workspace with two profiles run by one script.
    ///
    /// `writer` has no read-only invocation. `reader` has one, and under it the
    /// script answers a question, or writes anyway where the task says to, which
    /// is the misbehaviour the worktree check exists for. Either profile gives a
    /// verdict when handed a marker. An author writes `good` only once it has
    /// been told why the last attempt was not kept, and the gate passes only on
    /// `good`, so a loop has something to learn.
    fn workspace(name: &str) -> (Scratch, Workspace) {
        let dir = std::env::temp_dir().join(format!("ostraka-mode-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("ostraka");
        let repo = dir.join("repositories/work");
        std::fs::create_dir_all(&repo).expect("repository");

        let script = dir.join("agent.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             case \"$1\" in --probe) echo ok; exit 0;; esac\n\
             reading=no\n\
             if [ \"$1\" = \"--read-only\" ]; then reading=yes; shift; fi\n\
             marker=$(printf '%s' \"$1\" | grep -o 'VERDICT-[0-9a-f]*:' | head -1)\n\
             if [ -n \"$marker\" ]; then echo \"$marker APPROVE\"; exit 0; fi\n\
             if [ \"$reading\" = yes ]; then\n\
             \x20 if printf '%s' \"$1\" | grep -q 'write anyway'; then echo sneaky > sneaky.txt; fi\n\
             \x20 echo 'the answer is 42'; exit 0\n\
             fi\n\
             if printf '%s' \"$1\" | grep -q 'was not kept'; then echo good > wrote.txt; else echo bad > wrote.txt; fi\n\
             echo done\n",
        )
        .expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let command = script.display();
        std::fs::write(
            dir.join(".ostraka/adapters/writer.toml"),
            format!(
                "id = \"writer\"\ncommand = \"{command}\"\nargs = [\"{{{{prompt}}}}\"]\nprobe_args = [\"--probe\"]\n"
            ),
        )
        .expect("writer");
        std::fs::write(
            dir.join(".ostraka/adapters/reader.toml"),
            format!(
                "id = \"reader\"\ncommand = \"{command}\"\nargs = [\"{{{{prompt}}}}\"]\n\
                 review_args = [\"--read-only\", \"{{{{prompt}}}}\"]\nprobe_args = [\"--probe\"]\n"
            ),
        )
        .expect("reader");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\n\
             checks = [{ name = \"good\", cmd = \"grep -q good wrote.txt\", required = true }]\n\n\
             [gate.review]\n\
             must_differ_from_author = true\n",
        )
        .expect("config");
        std::fs::write(repo.join("seed.txt"), "seed\n").expect("seed");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "mode@example.invalid"]);
        git(&repo, &["config", "user.name", "mode"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);

        let workspace = Workspace::at(&dir);
        (Scratch(dir), workspace)
    }

    fn asked(prompt: &str, adapter: &str) -> Args {
        let mut args = Args::for_task(prompt.to_string());
        args.adapter = Some(adapter.to_string());
        args
    }

    #[test]
    fn a_question_is_answered_and_leaves_nothing_behind() {
        let (scratch, workspace) = workspace("ask");
        let consulted = consult(
            &workspace,
            &asked("what is the answer?", "reader"),
            Mode::Ask,
            None,
            &Stop::new(),
        )
        .expect("consulted");

        assert!(consulted.clean(), "{}", consulted.summary());
        assert!(consulted.answer.contains("42"), "{}", consulted.answer);
        assert!(!consulted.worktree.exists(), "a clean worktree was kept");
        let repo = scratch.0.join("repositories/work");
        assert!(
            git(&repo, &["branch", "--list", "ostraka/*"])
                .trim()
                .is_empty(),
            "a branch was left for a question"
        );
        assert!(
            workspace
                .ostraka()
                .join("consulted/runs")
                .join(&consulted.id)
                .is_dir(),
            "the consultation was not recorded"
        );
    }

    #[test]
    fn an_agent_that_writes_while_asked_is_refused_and_its_worktree_kept() {
        let (_scratch, workspace) = workspace("sneaky");
        let consulted = consult(
            &workspace,
            &asked("write anyway", "reader"),
            Mode::Plan,
            None,
            &Stop::new(),
        )
        .expect("consulted");

        assert!(!consulted.clean());
        assert_eq!(consulted.wrote, ["sneaky.txt"]);
        assert!(
            consulted.worktree.join("sneaky.txt").exists(),
            "the evidence went"
        );
        assert!(
            consulted.summary().starts_with("refused"),
            "{}",
            consulted.summary()
        );
    }

    #[test]
    fn a_profile_that_can_only_write_is_not_asked_anything() {
        let (_scratch, workspace) = workspace("writer");
        let refused = consult(
            &workspace,
            &asked("what is the answer?", "writer"),
            Mode::Ask,
            None,
            &Stop::new(),
        )
        .err()
        .expect("refused");
        assert!(refused.to_string().contains("read-only"), "{refused}");
    }

    #[test]
    fn a_loop_tries_again_with_the_reason_and_stops_once_approved() {
        let (_scratch, workspace) = workspace("loop");
        let mut args = asked("write the file", "writer");
        args.review_adapter = Some("reader".to_string());
        args.attempts = 3;

        let mut again = Vec::new();
        let report = run::execute_looping(
            &workspace,
            &args,
            || None,
            &Stop::new(),
            |n, _| again.push(n),
        )
        .expect("ran");
        assert!(report.approved(), "{:?}", report.refusal);
        assert_eq!(again, [2], "it should take exactly one more attempt");
    }

    #[test]
    fn auto_takes_one_attempt_and_the_refusal_stands() {
        let (_scratch, workspace) = workspace("auto");
        let mut args = asked("write the file", "writer");
        args.review_adapter = Some("reader".to_string());

        let report = run::execute_looping(
            &workspace,
            &args,
            || None,
            &Stop::new(),
            |_, _| panic!("auto tried again"),
        )
        .expect("ran");
        assert!(!report.approved());
    }
}
