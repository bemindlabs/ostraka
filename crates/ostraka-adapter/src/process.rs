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
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub struct ProcessAdapter {
    profile: Profile,
    role: Role,
}

impl ProcessAdapter {
    /// An adapter that writes changes.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            role: Role::Author,
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
        }
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}

impl VendorAdapter for ProcessAdapter {
    fn id(&self) -> &str {
        &self.profile.id
    }

    fn probe(&self) -> Availability {
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

        let child = Command::new(&self.profile.command)
            .args(&args)
            .envs(&self.profile.env)
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Launch {
                command: self.profile.command.clone(),
                source,
            })?;

        Ok(Box::new(ProcessSession {
            child,
            profile: self.profile.clone(),
            pending: VecDeque::new(),
            drained: false,
        }))
    }
}

pub struct ProcessSession {
    child: Child,
    profile: Profile,
    pending: VecDeque<Event>,
    drained: bool,
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
        AdapterOutcome {
            exit_code,
            // Files touched are read from the worktree's git status by the
            // runtime, not from the vendor's own account of what it did.
            files_touched: Vec::new(),
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
