//! The gate, and the one type the orchestrator cannot forge.
//!
//! The structural rule is that whoever wrote a change cannot be the one who
//! approves it. Stating that in documentation makes it a convention; stating it
//! here makes it a fact about the program.
//!
//! [`MergeToken`] has private fields and no public constructor. The only way to
//! obtain one is [`evaluate`], which requires both an [`AllChecksPassed`] —
//! itself obtainable only by actually running the checks — and an approval whose
//! reviewer differs from the change's author. An orchestrator holding every
//! other type in this crate still cannot produce one.
//!
//! This is also why the gate is not a separate crate. Across a crate boundary it
//! would have to be an injectable trait, and an injectable gate is a bypassable
//! one — which is the failure the whole design exists to prevent.

use ostraka_core::gate::{Approval, Check, CheckRecord, GateSpec};
use ostraka_core::identity::ActorId;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Proof that every required check ran and passed.
///
/// Constructible only by [`run_checks`], in this module. It carries no data a
/// caller could fabricate from elsewhere.
#[derive(Debug)]
pub struct AllChecksPassed {
    records: Vec<CheckRecord>,
}

impl AllChecksPassed {
    pub fn records(&self) -> &[CheckRecord] {
        &self.records
    }
}

/// Permission to merge a change. Cannot be constructed outside this module.
#[derive(Debug)]
pub struct MergeToken {
    author: ActorId,
    reviewer: ActorId,
    checks: Vec<CheckRecord>,
}

impl MergeToken {
    pub fn author(&self) -> &ActorId {
        &self.author
    }

    pub fn reviewer(&self) -> &ActorId {
        &self.reviewer
    }

    pub fn checks(&self) -> &[CheckRecord] {
        &self.checks
    }
}

/// Why a gate refused.
///
/// `ChecksFailed` carries the check records rather than only the names. A run
/// that fails is exactly when the evidence matters most, so the refusal itself
/// holds it and a caller cannot record the outcome without it.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// One or more required checks failed. Named so the reason is legible.
    ChecksFailed {
        failed: Vec<String>,
        records: Vec<CheckRecord>,
    },
    /// The reviewer rejected the change.
    Rejected { reason: String },
    /// The authoring agent could not run, and left nothing behind.
    ///
    /// Distinct from a rejection: nothing was judged. A vendor that hit a rate
    /// limit or an expired credential and a vendor that considered the task and
    /// declined it are the same empty diff, and only one of them is about the
    /// change. Its own words are carried so the difference is legible.
    AuthorFailed {
        code: String,
        diagnostics: Option<String>,
    },
    /// The change touched paths the project's policy does not allow.
    PolicyViolation { reason: String },
    /// The worktree could not be made ready to work in.
    ///
    /// Nothing was judged and nothing was written: the environment was not
    /// there. Reporting it as a failed check would say the change was rejected,
    /// which is the confusion this variant exists to end.
    SetupFailed { step: String, reason: String },
    /// The operator stopped the run.
    ///
    /// Not a judgement on the change at all — nobody finished looking at it.
    Interrupted,
    /// The authoring agent outlived the ceiling and was killed.
    ///
    /// Not a rejection and not a crash: nothing was judged, and the change was
    /// never finished. Reporting it as either would blame the work for the
    /// clock.
    TimedOut { after_secs: u64 },
    /// The authoring agent ran cleanly and changed nothing.
    ///
    /// A legitimate answer to a task — the work was already done, or the agent
    /// judged that nothing was needed — and not one a reviewer can rule on: it
    /// would be handed an empty diff and asked what it thinks of it.
    NoChange,
    /// The reviewer and the author are the same identity.
    SelfApproval { actor: ActorId },
}

/// Runs the project's checks in a worktree.
///
/// Returns the proof type only when every required check exited zero. Optional
/// checks are recorded but do not block, so that a project can observe a check
/// before it enforces one.
pub fn run_checks(
    spec: &GateSpec,
    worktree: &Path,
) -> std::result::Result<AllChecksPassed, Refusal> {
    let mut records = Vec::with_capacity(spec.checks.len());
    let mut failed = Vec::new();

    let ceiling = spec.timeout_secs.map(Duration::from_secs);
    for check in &spec.checks {
        let record = run_one(check, worktree, ceiling);
        if check.required && !record.passed() {
            failed.push(check.name.clone());
        }
        records.push(record);
    }

    if failed.is_empty() {
        Ok(AllChecksPassed { records })
    } else {
        Err(Refusal::ChecksFailed { failed, records })
    }
}

fn run_one(check: &Check, worktree: &Path, ceiling: Option<Duration>) -> CheckRecord {
    let mut record = run_command(&check.cmd, worktree, ceiling);
    record.name = check.name.clone();
    record
}

/// Runs one shell command in a worktree, bounded, capturing both streams.
///
/// Shared with worktree preparation, which needs exactly this and must not be
/// reported as a gate check.
pub fn run_command(cmd: &str, worktree: &Path, ceiling: Option<Duration>) -> CheckRecord {
    let started = Instant::now();
    let check = Check {
        name: String::new(),
        cmd: cmd.to_string(),
        required: true,
    };

    let spawned = Command::new("sh")
        .arg("-c")
        .arg(&check.cmd)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let (exit_code, stdout, stderr) = match spawned {
        Ok(child) => wait_for(child, ceiling),
        // A check that could not be started has not passed. Recording the
        // failure is the point; swallowing it would be the bug.
        Err(e) => (None, String::new(), e.to_string()),
    };

    CheckRecord {
        name: check.name.clone(),
        cmd: check.cmd.clone(),
        exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Runs a check to completion, or kills it for outlasting the ceiling.
///
/// Both streams are read on their own threads: a check that fills a pipe buffer
/// while this thread waits on the other one deadlocks, and a build that prints
/// a lot is the ordinary case rather than the exotic one.
fn wait_for(
    mut child: std::process::Child,
    ceiling: Option<Duration>,
) -> (Option<i32>, String, String) {
    fn reader(stream: Option<impl Read + Send + 'static>) -> std::sync::mpsc::Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_string(&mut text);
            }
            let _ = tx.send(text);
        });
        rx
    }

    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());

    let killed = match ceiling {
        None => false,
        Some(ceiling) => {
            let deadline = Instant::now() + ceiling;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break false,
                    Err(_) => break false,
                    Ok(None) => {}
                }
                if Instant::now() >= deadline || ostraka_adapter::interrupt::requested() {
                    let _ = child.kill();
                    break true;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };

    let status = child.wait().ok();
    // After a kill, take what already arrived rather than waiting for the
    // streams to close: killing a shell does not kill what it spawned, and a
    // surviving grandchild holds these pipes open. Waiting for EOF here would
    // wait on exactly the process the ceiling just stopped waiting for.
    let collect = |rx: std::sync::mpsc::Receiver<String>| -> String {
        if killed {
            rx.recv_timeout(Duration::from_millis(200))
                .unwrap_or_default()
        } else {
            rx.recv().unwrap_or_default()
        }
    };
    let stdout = collect(out);
    let mut stderr = collect(err);
    if killed {
        // Said in the record rather than left as a bare signal death, which
        // reads like the check crashed on its own.
        stderr.push_str(&format!(
            "\nostraka: killed after {}s — the gate's timeout_secs\n",
            ceiling.map(|c| c.as_secs()).unwrap_or_default()
        ));
    }
    // A killed process reports no code, which is already how "did not pass" is
    // spelled everywhere else here.
    (status.and_then(|s| s.code()), stdout, stderr)
}

/// The gate itself: the only path to a [`MergeToken`].
pub fn evaluate(
    checks: AllChecksPassed,
    author: &ActorId,
    approval: &Approval,
    must_differ_from_author: bool,
) -> std::result::Result<MergeToken, Refusal> {
    if must_differ_from_author && &approval.reviewer == author {
        return Err(Refusal::SelfApproval {
            actor: author.clone(),
        });
    }

    match &approval.verdict {
        ostraka_core::gate::Verdict::Reject { reason } => Err(Refusal::Rejected {
            reason: reason.clone(),
        }),
        ostraka_core::gate::Verdict::Approve => Ok(MergeToken {
            author: author.clone(),
            reviewer: approval.reviewer.clone(),
            checks: checks.records,
        }),
    }
}

/// The gate again, from a finished run's own evidence.
///
/// A run mints its token in memory and the token dies with the process, so
/// promoting an approved change later has to ask the same question a second
/// time. It is asked here, in the module that owns the answer: nothing outside
/// gains a way to build a [`MergeToken`], and a run that was refused cannot be
/// promoted by a caller that decides it disagrees.
///
/// The evidence is the run record, which is not inside the worktree. An agent
/// works in a worktree; the records live above it, so a fleet cannot write its
/// own approval. A person with a text editor can — and that same person can
/// commit anything they like directly. This gate is between the fleet and the
/// branch, not between the operator and their own repository.
///
/// Required checks are read from the project's current spec, not from the
/// record: a check added since the run was made has never passed, and a record
/// that predates it must not promote as though it had.
pub fn reaffirm(
    spec: &GateSpec,
    record: &ostraka_core::record::RunRecord,
    must_differ_from_author: bool,
) -> std::result::Result<MergeToken, Refusal> {
    let mut records = Vec::with_capacity(spec.checks.len());
    let mut failed = Vec::new();

    for check in &spec.checks {
        match record.checks.iter().find(|c| c.name == check.name) {
            Some(evidence) => {
                if check.required && !evidence.passed() {
                    failed.push(check.name.clone());
                }
                records.push(evidence.clone());
            }
            None if check.required => failed.push(check.name.clone()),
            None => {}
        }
    }

    if !failed.is_empty() {
        return Err(Refusal::ChecksFailed { failed, records });
    }

    let Some(approval) = &record.approval else {
        return Err(Refusal::Rejected {
            reason: "the run record carries no approval, so nothing reviewed this change"
                .to_string(),
        });
    };

    evaluate(
        AllChecksPassed { records },
        &record.author,
        approval,
        must_differ_from_author,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ostraka_core::gate::{ReviewPolicy, Verdict};

    fn spec(cmds: &[(&str, &str, bool)]) -> GateSpec {
        GateSpec {
            timeout_secs: None,
            checks: cmds
                .iter()
                .map(|(name, cmd, required)| Check {
                    name: (*name).to_string(),
                    cmd: (*cmd).to_string(),
                    required: *required,
                })
                .collect(),
            review: ReviewPolicy::default(),
        }
    }

    #[test]
    fn a_check_that_hangs_is_killed_and_says_so() {
        // A wedged test suite otherwise wedges the gate, and the gate is what
        // every run waits on.
        let mut spec = spec(&[("hang", "sleep 120", true)]);
        spec.timeout_secs = Some(1);
        let started = Instant::now();
        let refusal = run_checks(&spec, Path::new(".")).expect_err("must refuse");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the ceiling was not enforced"
        );
        match refusal {
            Refusal::ChecksFailed { failed, records } => {
                assert_eq!(failed, ["hang"]);
                assert!(
                    records[0].stderr.contains("killed after"),
                    "a killed check read as a crash: {:?}",
                    records[0].stderr
                );
            }
            other => panic!("wrong refusal: {other:?}"),
        }
    }

    #[test]
    fn a_check_that_finishes_inside_the_ceiling_is_untouched() {
        let mut spec = spec(&[("quick", "echo fine", true)]);
        spec.timeout_secs = Some(30);
        let passed = run_checks(&spec, Path::new(".")).expect("passes");
        assert!(passed.records()[0].passed());
        assert!(!passed.records()[0].stderr.contains("killed"));
    }

    #[test]
    fn checks_actually_run_and_their_output_is_captured() {
        let passed = run_checks(&spec(&[("echo", "echo hello", true)]), Path::new("."))
            .expect("check passes");
        let record = &passed.records()[0];
        assert!(record.passed());
        assert!(record.stdout.contains("hello"));
    }

    #[test]
    fn a_failing_required_check_refuses_the_gate() {
        let refusal = run_checks(&spec(&[("fail", "exit 1", true)]), Path::new("."))
            .expect_err("must refuse");
        match refusal {
            Refusal::ChecksFailed { failed, records } => {
                assert_eq!(failed, ["fail"]);
                // The evidence travels with the refusal.
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].exit_code, Some(1));
            }
            other => panic!("wrong refusal: {other:?}"),
        }
    }

    #[test]
    fn an_optional_check_is_recorded_but_does_not_block() {
        let passed = run_checks(
            &spec(&[("advisory", "exit 3", false), ("real", "true", true)]),
            Path::new("."),
        )
        .expect("optional failure does not block");
        assert_eq!(passed.records().len(), 2);
        assert!(!passed.records()[0].passed());
    }

    #[test]
    fn a_command_that_cannot_run_is_a_failure_not_a_pass() {
        let refusal = run_checks(
            &spec(&[("missing", "definitely-not-a-real-binary-xyz", true)]),
            Path::new("."),
        )
        .expect_err("must refuse");
        assert!(matches!(refusal, Refusal::ChecksFailed { .. }));
    }

    fn passing_checks() -> AllChecksPassed {
        run_checks(&spec(&[("ok", "true", true)]), Path::new(".")).expect("passes")
    }

    #[test]
    fn the_author_cannot_approve_their_own_change() {
        let archon = ActorId::new("archon");
        let refusal = evaluate(
            passing_checks(),
            &archon,
            &Approval {
                reviewer: archon.clone(),
                verdict: Verdict::Approve,
            },
            true,
        )
        .expect_err("self-approval must be refused");
        assert_eq!(refusal, Refusal::SelfApproval { actor: archon });
    }

    #[test]
    fn an_independent_approval_mints_a_token() {
        let token = evaluate(
            passing_checks(),
            &ActorId::new("archon"),
            &Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Approve,
            },
            true,
        )
        .expect("independent approval");
        assert_eq!(token.author().as_str(), "archon");
        assert_eq!(token.reviewer().as_str(), "ephor");
        assert_eq!(token.checks().len(), 1);
    }

    #[test]
    fn a_rejection_mints_nothing() {
        let refusal = evaluate(
            passing_checks(),
            &ActorId::new("archon"),
            &Approval {
                reviewer: ActorId::new("ephor"),
                verdict: Verdict::Reject {
                    reason: "no tests".into(),
                },
            },
            true,
        )
        .expect_err("rejection");
        assert_eq!(
            refusal,
            Refusal::Rejected {
                reason: "no tests".to_string()
            }
        );
    }
}
