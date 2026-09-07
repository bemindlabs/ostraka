//! The generic process adapter.
//!
//! There is one adapter implementation, driven by a profile. Every supported
//! vendor is a data file, which is what keeps vendor names out of the runtime
//! and lets someone add an unsupported CLI without waiting for a release.

use crate::capability::{Availability, Capabilities};
use crate::event::normalize_line;
use crate::profile::{Profile, Role};
use crate::{AdapterOutcome, Error, Result, Session, VendorAdapter};
use ostraka_core::record::Event;
use ostraka_core::task::TaskSpec;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long to keep collecting output after a vendor has been killed.
///
/// Short: it is only there to catch what was already in flight.
const GRACE: Duration = Duration::from_millis(200);

/// How often a waiting adapter looks up to see whether it should stop.
///
/// Short enough that Ctrl-C feels immediate, long enough not to spin.
const POLL: Duration = Duration::from_millis(100);

/// How much of a failed process's stderr is kept.
///
/// Enough to carry the reason — an expired credential, a rate limit, a missing
/// sandbox capability — without letting a chatty vendor's banner fill the run
/// record. The tail rather than the head: the error is at the end.
const STDERR_TAIL_LINES: usize = 20;

pub struct ProcessAdapter {
    profile: Profile,
    role: Role,
    /// Where relocated vendor home directories are created. Absent means the
    /// caller never offered one, which is fine until a profile asks for
    /// isolation — at which point launching un-isolated would quietly break
    /// the promise the profile makes, so it is refused instead.
    isolation_root: Option<PathBuf>,
    /// Wall-clock ceiling for one invocation. `None` means wait forever, which
    /// is what this did before the ceiling existed.
    timeout: Option<Duration>,
}

impl ProcessAdapter {
    /// An adapter that writes changes.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            role: Role::Author,
            isolation_root: None,
            timeout: None,
        }
    }

    /// An adapter that reviews someone else's change.
    ///
    /// Separate constructor rather than a flag on `launch`, so that the review
    /// invocation is fixed when the routing is decided: nothing downstream can
    /// hand a reviewer the author's write-enabled command line.
    pub fn reviewing(profile: Profile) -> Self {
        Self {
            profile,
            role: Role::Review,
            isolation_root: None,
            timeout: None,
        }
    }

    /// How long this vendor gets before it is killed.
    ///
    /// An agent that hangs — a stalled connection, a prompt waiting on stdin
    /// nobody will answer — otherwise hangs the run forever, and a fleet runner
    /// that can be stopped by one wedged subprocess has not automated anything.
    pub fn within(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Where this adapter may create a relocated home for its vendor.
    pub fn isolated_under(mut self, root: impl Into<PathBuf>) -> Self {
        self.isolation_root = Some(root.into());
        self
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The environment this vendor runs with.
    ///
    /// Built rather than inherited: an operating-system baseline, the names the
    /// profile asks for, the values it sets, and whatever isolation adds. See
    /// [`crate::environment`] for why the launcher's environment does not come
    /// along.
    ///
    /// Isolation wins by construction — the profile is refused at parse time if
    /// its `env` sets the same variable — because the isolation guarantee is
    /// structural and a stray `env` entry should not be able to undo it.
    fn environment(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let isolation = match (&self.profile.isolation, &self.isolation_root) {
            (None, _) => std::collections::BTreeMap::new(),
            (Some(isolation), Some(root)) => isolation.provision(root, &self.profile.id)?,
            (Some(_), None) => {
                return Err(Error::Isolation {
                    id: self.profile.id.clone(),
                    message: "the profile asks for a relocated home directory and the caller \
                              offered nowhere to put it; running anyway would read the \
                              operator's setup"
                        .to_string(),
                });
            }
        };
        Ok(crate::environment::build(
            &self.profile.inherit_env,
            &self.profile.env,
            isolation,
        ))
    }
}

impl VendorAdapter for ProcessAdapter {
    fn id(&self) -> &str {
        &self.profile.id
    }

    fn probe(&self) -> Availability {
        // Deliberately inherits the environment, unlike `launch`. A probe asks
        // a question rather than running a task, and building an environment
        // here would mean provisioning isolation — creating directories — as a
        // side effect of merely listing what is installed.
        //
        // `--version` is the one flag it is safe to assume by default: it must
        // not modify anything, and a CLI that refuses it is one we cannot reason
        // about. A profile may name a stronger question — a CLI can be on PATH
        // and answer `--version` while being unauthenticated and unusable.
        match Command::new(&self.profile.command)
            .args(&self.profile.probe_args)
            .stdin(Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => Availability::Ready {
                version: String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            },
            Ok(out) => Availability::Unusable {
                reason: format!(
                    "{} {} exited with {}",
                    self.profile.command,
                    self.profile.probe_args.join(" "),
                    out.status
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Availability::NotFound {
                command: self.profile.command.clone(),
            },
            Err(e) => Availability::Unusable {
                reason: e.to_string(),
            },
        }
    }

    fn capabilities(&self) -> Capabilities {
        self.profile.capabilities
    }

    fn launch(&self, spec: &TaskSpec, worktree: &Path) -> Result<Box<dyn Session>> {
        let args = self.profile.render_args(
            self.role,
            &spec.prompt,
            spec.model.as_deref(),
            &worktree.to_string_lossy(),
        );

        let mut child = Command::new(&self.profile.command)
            .args(&args)
            // Cleared first. `envs` alone adds to what this process inherited,
            // which is the whole launcher environment.
            .env_clear()
            .envs(self.environment()?)
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Launch {
                command: self.profile.command.clone(),
                source,
            })?;

        // Both streams are read on their own threads. Stderr must be, or a
        // vendor writing more than a pipe buffer of progress before closing
        // stdout deadlocks the run — which the talkative ones do. Stdout must
        // be too, for a different reason: a deadline cannot be applied to a
        // blocking read on this thread, and a vendor that hangs without writing
        // would wedge the run somewhere no timeout could reach it.
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let stderr_reader = child.stderr.take().map(|stderr| {
            let sink = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    let mut tail = sink.lock().expect("stderr tail lock");
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            })
        });

        let (stdout_tx, stdout_rx) = mpsc::channel();
        let stdout_reader = child.stdout.take().map(|stdout| {
            std::thread::spawn(move || {
                // Line by line rather than one blob at EOF: a vendor that has
                // to be killed never reaches EOF, and what it said before it
                // stalled is the only clue anyone gets.
                for line in BufReader::new(stdout)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    if stdout_tx.send(line).is_err() {
                        return;
                    }
                }
            })
        });

        Ok(Box::new(ProcessSession {
            child,
            deadline: self.timeout.map(|t| Instant::now() + t),
            stdout_rx: Some(stdout_rx),
            stdout_reader,
            timed_out: false,
            interrupted: false,
            profile: self.profile.clone(),
            role: self.role,
            pending: VecDeque::new(),
            stdout_text: String::new(),
            drained: false,
            stderr_tail,
            stderr_reader,
        }))
    }
}

pub struct ProcessSession {
    child: Child,
    profile: Profile,
    role: Role,
    /// When this invocation must be killed, if it has not finished.
    deadline: Option<Instant>,
    stdout_rx: Option<Receiver<String>>,
    stdout_reader: Option<JoinHandle<()>>,
    /// True once the deadline passed and the child was killed for it.
    timed_out: bool,
    /// True once the operator asked to stop and the child was killed for it.
    interrupted: bool,
    pending: VecDeque<Event>,
    /// Kept verbatim alongside the normalized events, because a vendor that
    /// reports its own accounting on stdout reports it in its own shape.
    stdout_text: String,
    drained: bool,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    stderr_reader: Option<JoinHandle<()>>,
}

impl ProcessSession {
    /// Collects everything the vendor wrote, or kills it for taking too long.
    ///
    /// The reader runs on its own thread, so waiting for it can carry a
    /// deadline. On expiry the child is killed and the reader is drained
    /// afterwards, which keeps whatever it managed to say — a vendor usually
    /// explains itself before it hangs, and that explanation is the only clue
    /// anyone gets.
    fn drain_stdout(&mut self) {
        if self.drained {
            return;
        }
        self.drained = true;
        let Some(rx) = self.stdout_rx.take() else {
            return;
        };

        let mut lines: Vec<String> = Vec::new();
        loop {
            // Polled rather than blocked, so a Ctrl-C is noticed even for a
            // vendor producing no output at all — which is the one most worth
            // being able to stop.
            let wait = match self.deadline {
                None => POLL,
                Some(deadline) => POLL.min(deadline.saturating_duration_since(Instant::now())),
            };
            match rx.recv_timeout(wait) {
                Ok(line) => lines.push(line),
                // The vendor closed its output: it is done.
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let expired = self
                        .deadline
                        .is_some_and(|deadline| Instant::now() >= deadline);
                    if !expired && !crate::interrupt::requested() {
                        continue;
                    }
                    self.timed_out = expired;
                    self.interrupted = !expired;
                    let _ = self.child.kill();
                    // Take what already arrived and stop. Killing a shell does
                    // not kill what it spawned, and a surviving grandchild
                    // holds this pipe open — so waiting for EOF here would wait
                    // on precisely the process we just gave up waiting for.
                    while let Ok(line) = rx.recv_timeout(GRACE) {
                        lines.push(line);
                    }
                    break;
                }
            }
        }
        let text = lines.join("\n");

        self.stdout_text = text;
        for line in self.stdout_text.lines() {
            if let Some(event) = normalize_line(self.profile.event_format, line) {
                self.pending.push_back(event);
            }
        }

        // A whole-document format only makes sense once the document is whole.
        if self.profile.event_format == crate::profile::EventFormat::Json {
            let pointer = self.profile.event_text.clone().unwrap_or_default();
            if let Some(event) = crate::event::document_event(&self.stdout_text, &pointer) {
                self.pending.push_back(event);
            }
        }
    }
}

impl Session for ProcessSession {
    fn next_event(&mut self) -> Option<Event> {
        self.drain_stdout();
        self.pending.pop_front()
    }

    fn finish(mut self: Box<Self>) -> AdapterOutcome {
        self.drain_stdout();
        let exit_code = self.child.wait().ok().and_then(|s| s.code());
        // Joined only when the vendor ended on its own. After a kill a
        // grandchild may still hold the pipes, and joining would reintroduce
        // exactly the wait the ceiling exists to bound. The threads end when
        // the last writer closes; nothing leaks but patience.
        if !self.timed_out && !self.interrupted {
            if let Some(reader) = self.stderr_reader.take() {
                let _ = reader.join();
            }
            if let Some(reader) = self.stdout_reader.take() {
                let _ = reader.join();
            }
        }
        let stderr_text = {
            let tail = self.stderr_tail.lock().expect("stderr tail lock");
            tail.iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Read before the tail is discarded: a vendor reports what it spent on
        // a successful run too, and that is the run whose cost anyone asks about.
        let usage = self.profile.usage.as_ref().and_then(|spec| {
            let role = match self.role {
                Role::Author => "author",
                Role::Review => "reviewer",
            };
            spec.read(&self.profile.id, role, &self.stdout_text, &stderr_text)
        });

        // Diagnostics are kept only for a failure. A successful run's stderr is
        // progress reporting, and recording it would bury the runs that went
        // wrong.
        let diagnostics = if exit_code == Some(0) {
            None
        } else {
            (!stderr_text.trim().is_empty()).then_some(stderr_text)
        };
        // A terminal sends Ctrl-C to the whole foreground process group, so the
        // child usually dies of the same signal before the poll above notices
        // and the drain ends at EOF instead. The run still ended because the
        // operator said so, and reporting it as the agent failing would blame
        // the work for their decision.
        let interrupted = self.interrupted || crate::interrupt::requested();

        AdapterOutcome {
            exit_code,
            timed_out: self.timed_out,
            interrupted,
            usage,
            // Files touched are read from the worktree's git status by the
            // runtime, not from the vendor's own account of what it did.
            files_touched: Vec::new(),
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::identity::ActorId;

    /// Serializes the tests that touch the interrupt flag.
    ///
    /// The flag is global because a signal is global — that is right for a
    /// process and hostile to a test harness that runs threads in parallel, so
    /// every test that reads or writes it takes this first. Without it, the
    /// interrupt tests make the timeout tests take the interrupt branch and
    /// report that the ceiling was not enforced.
    static SIGNALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        let guard = SIGNALS.lock().unwrap_or_else(|e| e.into_inner());
        crate::interrupt::clear();
        guard
    }

    fn profile(command: &str) -> Profile {
        Profile::parse(&format!(
            r#"
            id = "t"
            command = "{command}"
            args = ["{{{{prompt}}}}"]
            "#
        ))
        .expect("valid profile")
    }

    #[test]
    fn a_missing_command_probes_as_not_found() {
        let adapter = ProcessAdapter::new(profile("definitely-not-a-real-binary-xyz"));
        match adapter.probe() {
            Availability::NotFound { command } => {
                assert_eq!(command, "definitely-not-a-real-binary-xyz");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    fn shell_profile(script: &str) -> Profile {
        Profile::parse(&format!(
            r#"
            id = "sh"
            command = "sh"
            args = ["-c", "{script}", "--", "{{{{prompt}}}}"]
            "#
        ))
        .expect("valid profile")
    }

    fn spec() -> TaskSpec {
        TaskSpec {
            id: "t1".into(),
            prompt: "go".into(),
            adapter: "sh".into(),
            author: ActorId::new("archon"),
            base_ref: "HEAD".into(),
            model: None,
        }
    }

    #[test]
    fn a_failure_keeps_the_reason_the_vendor_gave_on_stderr() {
        // The shape that motivated this: nothing on stdout, the reason on
        // stderr, a non-zero exit. Without capture it is an unreadable "1".
        let adapter = ProcessAdapter::new(shell_profile("echo 'usage limit reached' >&2; exit 1"));
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        let outcome = session.finish();
        assert_eq!(outcome.exit_code, Some(1));
        assert!(
            outcome
                .diagnostics
                .as_deref()
                .is_some_and(|d| d.contains("usage limit reached")),
            "diagnostics were {:?}",
            outcome.diagnostics
        );
    }

    #[test]
    fn a_vendor_that_hangs_is_killed_at_the_ceiling() {
        let _signals = exclusive();
        // The reason this exists: an agent waiting on a stalled connection, or
        // on stdin nobody will answer, otherwise wedges the run forever.
        let adapter = ProcessAdapter::new(shell_profile("echo starting; sleep 120"))
            .within(Some(Duration::from_millis(400)));
        let started = Instant::now();
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        let outcome = session.finish();
        let elapsed = started.elapsed();

        assert!(outcome.timed_out, "the ceiling was not enforced");
        assert!(
            elapsed < Duration::from_secs(20),
            "waited {elapsed:?} for a 400ms ceiling"
        );
    }

    #[test]
    fn what_a_killed_vendor_managed_to_say_is_kept() {
        let _signals = exclusive();
        // A vendor usually explains itself before it hangs, and that sentence
        // is the only clue anyone gets about why.
        let adapter = ProcessAdapter::new(shell_profile("echo 'about to stall'; sleep 120"))
            .within(Some(Duration::from_millis(400)));
        let mut session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        let first = session.next_event().expect("kept the output");
        match first {
            Event::Message { text, .. } => assert_eq!(text, "about to stall"),
            other => panic!("wrong variant: {other:?}"),
        }
        assert!(session.finish().timed_out);
    }

    #[test]
    fn a_child_killed_by_the_same_ctrl_c_still_reads_as_an_interrupt() {
        let _signals = exclusive();
        // The shape a real terminal produces: the signal reaches the whole
        // foreground group, so the child dies on its own and the drain ends at
        // EOF. Without this the run reported "the author could not run", which
        // blames the agent for the operator's decision.
        let adapter = ProcessAdapter::new(shell_profile("echo working; exit 130"));
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        crate::interrupt::request();
        let outcome = session.finish();
        crate::interrupt::clear();
        assert!(
            outcome.interrupted,
            "an interrupt in flight was not recorded"
        );
    }

    #[test]
    fn an_interrupt_stops_a_vendor_that_has_no_ceiling_at_all() {
        let _signals = exclusive();
        // The case that used to wedge forever: no deadline, no output, and an
        // operator who wants their terminal back.
        let adapter = ProcessAdapter::new(shell_profile("echo working; sleep 120"));
        let started = Instant::now();
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        // Scoped, so the thread is joined before the guard is released. A
        // detached one fires after this test has finished and sets the flag
        // underneath whichever test runs next — which is how the timeout tests
        // started reporting that the ceiling was not enforced.
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                crate::interrupt::request();
            });
            session.finish()
        });
        crate::interrupt::clear();

        assert!(outcome.interrupted, "the interrupt was not noticed");
        assert!(!outcome.timed_out, "an interrupt is not a timeout");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "waited {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_vendor_that_finishes_in_time_is_not_marked_as_stopped() {
        let _signals = exclusive();
        let adapter =
            ProcessAdapter::new(shell_profile("echo quick")).within(Some(Duration::from_secs(30)));
        let outcome = adapter
            .launch(&spec(), Path::new("."))
            .expect("launches")
            .finish();
        assert!(!outcome.timed_out);
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[test]
    fn no_ceiling_means_what_it_did_before() {
        let _signals = exclusive();
        // The default has to stay "wait", or adding the field would change how
        // every existing project behaves.
        let adapter = ProcessAdapter::new(shell_profile("echo done"));
        let outcome = adapter
            .launch(&spec(), Path::new("."))
            .expect("launches")
            .finish();
        assert!(!outcome.timed_out);
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[test]
    fn a_successful_run_records_no_diagnostics() {
        // Progress reporting on stderr is normal and is not evidence of
        // anything; keeping it would bury the runs that actually went wrong.
        let adapter = ProcessAdapter::new(shell_profile("echo 'fetching...' >&2; echo done"));
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        let outcome = session.finish();
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.diagnostics, None);
    }

    #[test]
    fn a_vendor_that_floods_stderr_does_not_deadlock_and_keeps_the_tail() {
        // More stderr than a pipe buffer holds, written before stdout closes.
        // Draining stderr after stdout would hang here forever.
        let adapter = ProcessAdapter::new(shell_profile(
            "i=0; while [ $i -lt 4000 ]; do echo 'padding padding padding padding' >&2; \
             i=$((i+1)); done; echo 'the real error' >&2; exit 2",
        ));
        let session = adapter.launch(&spec(), Path::new(".")).expect("launches");
        let outcome = session.finish();
        assert_eq!(outcome.exit_code, Some(2));
        let diagnostics = outcome.diagnostics.expect("kept a tail");
        assert!(diagnostics.contains("the real error"), "tail lost the end");
        assert!(
            diagnostics.lines().count() <= STDERR_TAIL_LINES,
            "kept {} lines",
            diagnostics.lines().count()
        );
    }

    fn isolating_profile() -> Profile {
        Profile::parse(
            r#"
            id = "iso"
            command = "sh"
            args = ["-c", "echo \"$VENDOR_HOME\"", "--", "{{prompt}}"]

            [isolation]
            home_env = "VENDOR_HOME"
            home_source = ".vendor"
            "#,
        )
        .expect("valid profile")
    }

    #[test]
    fn an_isolating_profile_will_not_launch_with_nowhere_to_isolate_into() {
        // Running anyway would read the operator's setup while the profile says
        // it does not. A refusal is the only honest outcome.
        let adapter = ProcessAdapter::new(isolating_profile());
        let Err(err) = adapter.launch(&spec(), Path::new(".")) else {
            panic!("must refuse to launch un-isolated");
        };
        assert!(err.to_string().contains("relocated home"), "{err}");
    }

    #[test]
    fn isolation_puts_the_relocated_home_in_the_vendors_environment() {
        let root = std::env::temp_dir().join(format!("ostraka-process-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let adapter = ProcessAdapter::new(isolating_profile()).isolated_under(&root);
        let mut session = adapter.launch(&spec(), Path::new(".")).expect("launches");

        let expected = root.join("iso");
        match session.next_event().expect("one event") {
            Event::Message { text, .. } => assert_eq!(text, expected.to_string_lossy()),
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(session.finish().exit_code, Some(0));
        assert!(expected.is_dir(), "the relocated home was never created");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_yields_its_output_as_events_and_an_exit_code() {
        let adapter = ProcessAdapter::new(profile("echo"));
        let spec = TaskSpec {
            id: "t1".into(),
            prompt: "hello from the worktree".into(),
            adapter: "t".into(),
            author: ActorId::new("archon"),
            base_ref: "HEAD".into(),
            model: None,
        };
        let mut session = adapter
            .launch(&spec, Path::new("."))
            .expect("echo launches");

        let first = session.next_event().expect("one event");
        match first {
            Event::Message { text, .. } => assert_eq!(text, "hello from the worktree"),
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(session.finish().exit_code, Some(0));
    }
}
