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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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
}

impl ProcessAdapter {
    /// An adapter that writes changes.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            role: Role::Author,
            isolation_root: None,
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
        }
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

        // Stderr is drained on its own thread. Reading it after stdout would
        // deadlock the moment a vendor wrote more than a pipe buffer of
        // progress to stderr before closing stdout, which the talkative ones do.
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

        Ok(Box::new(ProcessSession {
            child,
            profile: self.profile.clone(),
            pending: VecDeque::new(),
            drained: false,
            stderr_tail,
            stderr_reader,
        }))
    }
}

pub struct ProcessSession {
    child: Child,
    profile: Profile,
    pending: VecDeque<Event>,
    drained: bool,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    stderr_reader: Option<JoinHandle<()>>,
}

impl ProcessSession {
    fn drain_stdout(&mut self) {
        if self.drained {
            return;
        }
        self.drained = true;
        let Some(stdout) = self.child.stdout.take() else {
            return;
        };
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if let Some(event) = normalize_line(self.profile.event_format, &line) {
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
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        // Kept only for a failure. A successful run's stderr is progress
        // reporting, and recording it would bury the runs that went wrong.
        let diagnostics = if exit_code == Some(0) {
            None
        } else {
            let tail = self.stderr_tail.lock().expect("stderr tail lock");
            let text = tail
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(text)
        };
        AdapterOutcome {
            exit_code,
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
