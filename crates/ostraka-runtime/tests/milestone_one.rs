//! End-to-end: one task, two different adapters, a gate that really runs.
//!
//! These tests build a real git repository in a temp directory and drive the
//! orchestrator over it. The "vendors" are shell scripts, which is the point —
//! the runtime knows nothing about any particular CLI, so a script is as valid
//! an adapter as a commercial tool.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
use ostraka_core::gate::Verdict;
use ostraka_core::identity::ActorId;
use ostraka_core::record::Outcome;
use ostraka_core::task::TaskSpec;
use ostraka_runtime::gate::Refusal;
use ostraka_runtime::{orchestrator, route};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    repo: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.repo).ok();
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
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit, plus two executable "agents".
fn fixture(name: &str) -> Fixture {
    let repo = std::env::temp_dir().join(format!("ostraka-e2e-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&repo).ok();
    std::fs::create_dir_all(&repo).expect("temp dir");

    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").expect("write");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    Fixture { repo }
}

/// A reviewer body that answers with the marker it was given.
///
/// The marker is derived per review and appears only in the prompt, so a
/// reviewer has to read it out of what it was handed — which is the mechanism
/// under test, not a detail of the fixture.
fn verdict(answer: &str) -> String {
    format!(
        "marker=$(printf '%s' \"$1\" | grep -o 'VERDICT-[0-9a-f]*:' | head -1)\n\
         echo \"$marker {answer}\""
    )
}

/// Writes an executable script and returns a profile that runs it.
fn agent(repo: &Path, id: &str, body: &str) -> Profile {
    let path = repo.join(format!("{id}.sh"));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    Profile::parse(&format!(
        r#"
        id = "{id}"
        command = "{}"
        args = ["{{{{prompt}}}}"]
        "#,
        path.display()
    ))
    .expect("valid profile")
}

fn config(check_cmd: &str) -> Config {
    Config::parse(&format!(
        r#"
        [gate]
        checks = [{{ name = "test", cmd = "{check_cmd}", required = true }}]

        [gate.review]
        must_differ_from_author = true
        "#
    ))
    .expect("valid config")
}

/// Where a fixture's run happens: the repository is the fixture, and the
/// worktrees and records sit beside it as they do in a workspace.
fn places(f: &Fixture) -> (PathBuf, PathBuf) {
    (f.repo.join("worktrees"), f.repo.join(".ostraka"))
}

fn task(prompt: &str, author: &str) -> TaskSpec {
    TaskSpec {
        id: "t1".into(),
        prompt: prompt.into(),
        adapter: "writer".into(),
        author: ActorId::new(author),
        base_ref: "HEAD".into(),
        model: None,
    }
}

#[test]
fn what_is_linked_into_a_worktree_is_not_part_of_the_change() {
    // Found by a real vendor run: the notes directory is linked into every
    // worktree, `git status` reported the symlink as something the agent had
    // added, and it was committed with the work and shown to a reviewer as
    // part of the change. A link is scaffolding, not a change.
    let f = fixture("linked");
    std::fs::create_dir_all(f.repo.join("notes")).expect("notes");
    std::fs::write(f.repo.join("notes/earlier.md"), "what was worked out").expect("write");

    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let notes = f.repo.join("notes");
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: Some(&notes),
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("runs");

    assert!(report.approved(), "{:?}", report.refusal);
    // The diff the reviewer was handed, and the commit that was made.
    assert!(report.diff.contains("added.txt"), "{}", report.diff);
    assert!(
        !report.diff.contains("notes"),
        "the link was shown to a reviewer as part of the change:\n{}",
        report.diff
    );

    let out = Command::new("git")
        .args([
            "show",
            "--name-only",
            "--format=",
            &format!("ostraka/{}", report.record.run_id),
        ])
        .current_dir(&f.repo)
        .output()
        .expect("git runs");
    let committed = String::from_utf8_lossy(&out.stdout);
    assert!(committed.contains("added.txt"), "{committed}");
    assert!(
        !committed.contains("notes"),
        "the link was committed:\n{committed}"
    );
}

#[test]
fn an_author_that_changed_nothing_is_reported_as_such_and_not_sent_to_a_reviewer() {
    // A legitimate answer to a task — the work was already done — and not one a
    // reviewer can rule on. Handing it an empty diff produces a confused answer
    // that then gets reported as the reason the run was refused.
    let f = fixture("no-change");
    let writer = agent(&f.repo, "writer", "echo 'that test already exists'");
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a test that is already there", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("runs");

    assert!(!report.approved());
    assert!(report.record.approval.is_none(), "a reviewer was called");
    assert!(matches!(
        report.refusal,
        Some(ostraka_runtime::gate::Refusal::NoChange)
    ));
}

#[test]
fn a_run_the_operator_stopped_says_so_rather_than_blaming_the_agent() {
    // Ctrl-C is not a verdict on the change. Reporting it as "the author could
    // not run" would blame the work for a decision the operator made.
    let f = fixture("interrupted");
    let writer = agent(&f.repo, "writer", "echo working; sleep 120");
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    ostraka_adapter::interrupt::clear();
    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    // Scoped so the signalling thread cannot outlive this test and set the
    // flag underneath another one.
    let report = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(400));
            ostraka_adapter::interrupt::request();
        });
        orchestrator::run_task(
            &places,
            &config("true"),
            &routing,
            &task("do a thing", "archon"),
            &ActorId::new("ephor"),
            None,
        )
        .expect("runs")
    });
    ostraka_adapter::interrupt::clear();

    assert!(!report.approved());
    assert!(report.record.approval.is_none(), "a reviewer was called");
    assert!(
        matches!(report.refusal, Some(Refusal::Interrupted)),
        "wrong refusal: {:?}",
        report.refusal
    );
}

#[test]
fn an_author_that_could_not_run_is_refused_in_its_own_words_without_calling_a_reviewer() {
    // A vendor out of quota and a vendor that considered the task and did
    // nothing leave the same empty worktree. Only one of them is about the
    // change, and the run has to say which.
    let f = fixture("author-failed");
    let writer = agent(&f.repo, "writer", "echo 'usage limit reached' >&2; exit 1");
    // Reaching this reviewer at all would be the failure: there is nothing to
    // review, and running it would spend a second vendor to be told so.
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("change something", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("runs");

    assert!(!report.approved());
    assert!(report.record.approval.is_none(), "a reviewer was called");
    match report.refusal {
        Some(ostraka_runtime::gate::Refusal::AuthorFailed { code, diagnostics }) => {
            assert_eq!(code, "1");
            assert!(
                diagnostics.is_some_and(|d| d.contains("usage limit reached")),
                "the vendor's reason was discarded"
            );
        }
        other => panic!("wrong refusal: {other:?}"),
    }
}

#[test]
fn an_approved_run_produces_a_token_a_commit_and_a_replayable_record() {
    let f = fixture("approve");
    let writer = agent(
        &f.repo,
        "writer",
        "echo 'made the change' && echo new > added.txt",
    );
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");

    assert!(
        report.approved(),
        "expected approval, got {:?}",
        report.refusal
    );

    let token = report.token.as_ref().expect("token minted");
    assert_eq!(token.author().as_str(), "archon");
    assert_eq!(token.reviewer().as_str(), "ephor");

    // The gate really ran the check and captured its result.
    assert_eq!(report.record.checks.len(), 1);
    assert!(report.record.checks[0].passed());

    // The agent's work reached the diff.
    assert!(
        report.diff.contains("added.txt"),
        "diff missing the new file: {}",
        report.diff
    );

    // And the run is replayable from disk.
    let (record, events) = orchestrator::replay(&records, &report.record.run_id).expect("replays");
    assert_eq!(record.outcome, Some(Outcome::Approved));
    assert!(!events.is_empty(), "no events were logged");
}

#[test]
fn a_failing_check_refuses_before_any_reviewer_is_consulted() {
    let f = fixture("checkfail");
    let writer = agent(&f.repo, "writer", "echo broken > added.txt");
    // A reviewer that would approve anything. It must never be reached.
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("exit 1"),
        &routing,
        &task("break it", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");

    assert!(!report.approved());
    assert!(
        matches!(report.refusal, Some(Refusal::ChecksFailed { .. })),
        "expected a check failure, got {:?}",
        report.refusal
    );
    assert!(
        report.record.approval.is_none(),
        "reviewer should not have been consulted once a required check failed"
    );

    // The record of a failed run must still carry what failed and why. This is
    // the case where someone actually needs to read it.
    assert_eq!(
        report.record.checks.len(),
        1,
        "check evidence was discarded"
    );
    assert!(!report.record.checks[0].passed());
    assert_eq!(report.record.checks[0].exit_code, Some(1));
}

#[test]
fn a_rejecting_reviewer_blocks_a_change_whose_checks_all_passed() {
    let f = fixture("reject");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(&f.repo, "reviewer", &verdict("REJECT: out of scope"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("do too much", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");

    assert!(!report.approved());
    assert!(report.record.checks[0].passed(), "checks did pass");
    match report.refusal {
        Some(Refusal::Rejected { ref reason }) => assert_eq!(reason, "out of scope"),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn a_silent_reviewer_is_a_rejection_not_a_pass() {
    let f = fixture("silent");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(&f.repo, "reviewer", "exit 0");
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("say nothing", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");

    assert!(!report.approved(), "silence must never read as approval");
}

#[test]
fn the_author_cannot_review_their_own_change_end_to_end() {
    let f = fixture("selfapprove");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    // Same identity on both sides: the reviewing adapter differs, the actor does not.
    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("approve myself", "archon"),
        &ActorId::new("archon"),
        None,
    )
    .expect("run completes");

    assert!(!report.approved());
    match report.refusal {
        Some(Refusal::SelfApproval { ref actor }) => assert_eq!(actor.as_str(), "archon"),
        other => panic!("expected a self-approval refusal, got {other:?}"),
    }
}

// --- running out of tokens ------------------------------------------------
//
// A vendor that exhausts its context window does not stop politely. It exits
// non-zero, sometimes after having written part of what it was asked for, and
// what it says about why is on stderr. These pin down what happens then.

/// A vendor that also reports what it spent, the way a shipped profile does.
fn counting_agent(repo: &Path, id: &str, body: &str) -> Profile {
    let path = repo.join(format!("{id}.sh"));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    Profile::parse(&format!(
        r#"
        id = "{id}"
        command = "{}"
        args = ["{{{{prompt}}}}"]

        [usage]
        stream = "stderr"
        shape = "text"
        total = "tokens used"
        "#,
        path.display()
    ))
    .expect("valid profile")
}

/// Runs a pair against one task and hands back the report.
fn run_pair(
    f: &Fixture,
    writer: Profile,
    reviewer: Profile,
    prompt: &str,
) -> orchestrator::RunReport {
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();
    let (worktrees, records) = places(f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: None,
        skills: None,
    };
    orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task(prompt, "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("runs")
}

#[test]
fn an_author_that_ran_out_of_context_halfway_is_refused_rather_than_reviewed() {
    // The dangerous shape of running out of tokens, and the one this used to
    // get wrong: the vendor had already written part of the change when its
    // context filled, so the worktree is not empty and the exit code is not
    // zero. That half-written change was gated, reviewed and approved.
    //
    // A context window running out is a stop like any other. Only who stopped
    // it differs, and half a change is not a change anybody should be asked to
    // review — which is what a killed author has always been told.
    let f = fixture("out-of-context");
    let writer = agent(
        &f.repo,
        "writer",
        "echo 'half a change' > added.txt\n\
         echo 'Error: prompt is too long: 210000 tokens > 200000 maximum' >&2\n\
         exit 1",
    );
    // Approving. Reaching it at all is the failure this test is about.
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));

    let report = run_pair(&f, writer, reviewer, "write a long thing");

    assert!(!report.approved(), "a half-written change was approved");
    assert!(
        report.record.approval.is_none(),
        "a reviewer was asked about half a change"
    );
    match report.refusal {
        Some(Refusal::AuthorFailed { code, diagnostics }) => {
            assert_eq!(code, "1");
            assert!(
                diagnostics.is_some_and(|d| d.contains("prompt is too long")),
                "the vendor's own account of why was discarded"
            );
        }
        other => panic!("wrong refusal: {other:?}"),
    }

    // And what it did write is still there. A refused run keeps its worktree
    // because that is the evidence, and this is exactly the run where somebody
    // wants to see how far it got before it ran out.
    let left = std::fs::read_dir(f.repo.join("worktrees"))
        .expect("a worktrees directory")
        .filter_map(|e| e.ok())
        .find(|e| e.path().join("added.txt").is_file());
    assert!(left.is_some(), "the evidence was thrown away");
}

#[test]
fn a_reviewer_that_ran_out_of_context_is_a_rejection_in_its_own_words() {
    // Fail safe, and say why. A reviewer that ran out of context and one that
    // read the change and disliked it are the same exit code from here, and
    // only one of them is about the change.
    let f = fixture("reviewer-out");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(
        &f.repo,
        "reviewer",
        "echo 'Error: prompt is too long: 210000 tokens > 200000 maximum' >&2; exit 1",
    );

    let report = run_pair(&f, writer, reviewer, "change something");

    assert!(
        !report.approved(),
        "a reviewer that never read it approved it"
    );
    match report.record.approval.map(|a| a.verdict) {
        Some(Verdict::Reject { reason }) => {
            assert!(reason.contains("could not run"), "{reason}");
            assert!(
                reason.contains("prompt is too long"),
                "the reviewer's reason was discarded: {reason}"
            );
        }
        other => panic!("wrong verdict: {other:?}"),
    }
}

#[test]
fn a_reviewer_cut_off_before_its_verdict_is_a_rejection() {
    // The quiet shape: the vendor exits zero, having said something sensible
    // and stopped mid-sentence with the verdict line still unwritten. Nothing
    // here is an error; there is simply no answer, and no answer is not yes.
    let f = fixture("reviewer-cut");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(
        &f.repo,
        "reviewer",
        "echo 'The change adds a file and the test covers it, so I think it is'\n\
         exit 0",
    );

    let report = run_pair(&f, writer, reviewer, "change something");

    assert!(!report.approved(), "a truncated answer read as approval");
    assert!(
        matches!(
            report.record.approval.map(|a| a.verdict),
            Some(Verdict::Reject { .. })
        ),
        "a reviewer that never got to its verdict must be a rejection"
    );
}

#[test]
fn a_reviewer_that_answered_twice_is_a_rejection() {
    // What a retry after a context error looks like from here: the vendor
    // answered, hit the limit, started again, and both answers came out. Two
    // verdicts is not one verdict, and picking either would be picking.
    let f = fixture("reviewer-twice");
    let writer = agent(&f.repo, "writer", "echo new > added.txt");
    let reviewer = agent(
        &f.repo,
        "reviewer",
        "marker=$(printf '%s' \"$1\" | grep -o 'VERDICT-[0-9a-f]*:' | head -1)\n\
         echo \"$marker REJECT: ran out of room\"\n\
         echo \"$marker APPROVE\"",
    );

    let report = run_pair(&f, writer, reviewer, "change something");

    assert!(!report.approved());
    match report.record.approval.map(|a| a.verdict) {
        Some(Verdict::Reject { reason }) => {
            assert!(reason.contains("more than one"), "{reason}");
        }
        other => panic!("wrong verdict: {other:?}"),
    }
}

#[test]
fn what_a_run_spent_is_recorded_even_when_it_ran_out() {
    // The run that ran out of tokens is the one somebody most wants the number
    // for. Usage is read from what the vendor said before it stopped, and it
    // said it on the way out.
    let f = fixture("spent");
    let writer = counting_agent(
        &f.repo,
        "writer",
        "echo 'tokens used 210000' >&2\n\
         echo 'Error: prompt is too long' >&2\n\
         exit 1",
    );
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));

    let report = run_pair(&f, writer, reviewer, "write a long thing");

    assert!(!report.approved());
    let spent = report
        .record
        .usage
        .iter()
        .find(|u| u.adapter == "writer")
        .expect("the author's usage was not recorded");
    assert_eq!(spent.total, Some(210_000));
}

#[test]
fn an_author_is_told_about_the_notes_and_the_record_is_not() {
    // The notes directory is linked into every worktree, shared across runs,
    // outside the diff, and survives a refusal. None of that is visible from a
    // directory listing, so it is said — to the author, and to nobody else.
    //
    // The failure this pins down is the one the change could have introduced:
    // `task.prompt` is read again by the run record and by the commit message,
    // so composing the preamble in place would have put it in both, where it is
    // neither what was asked for nor part of the audit trail.
    let f = fixture("notes-prompt");
    let notes = f.repo.join("workspace-notes");
    std::fs::create_dir_all(&notes).expect("notes dir");
    std::fs::write(notes.join("earlier.md"), "what an earlier run worked out").expect("write");

    let writer = agent(
        &f.repo,
        "writer",
        // Keeps the instruction it was given, reads what an earlier run left,
        // and makes a change for the gate to judge. Both artefacts go into
        // `notes/` rather than into the worktree: an approved run releases its
        // worktree, so anything written there is gone before it can be
        // asserted on — which is what "proves it can read" was resting on.
        "printf '%s' \"$1\" > notes/seen-prompt.txt\n\
         cat notes/earlier.md > notes/read-back.txt\n\
         echo new > added.txt",
    );
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: Some(&notes),
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");
    assert!(report.approved(), "expected approval: {:?}", report.refusal);

    // 1. The author was told, and the operator's sentence came first.
    let seen = std::fs::read_to_string(notes.join("seen-prompt.txt")).expect("the agent's prompt");
    assert!(
        seen.starts_with("add a file"),
        "the task was not first: {seen}"
    );
    assert!(seen.contains("not part of this repository"), "{seen}");
    assert!(seen.contains("shared with every other run"), "{seen}");

    // 2. Writing to the link reached the workspace, not the worktree, and it is
    //    still there after the worktree was released.
    assert!(
        notes.join("seen-prompt.txt").is_file(),
        "what the agent wrote to notes/ did not reach the workspace"
    );

    // 3. And reading worked, which is the half the preamble asks for first.
    //    Asserted rather than implied: the read-back is written back through
    //    the link, so it outlives the worktree it was produced in.
    let read_back = std::fs::read_to_string(notes.join("read-back.txt"))
        .expect("the agent could not read what an earlier run left");
    assert_eq!(read_back, "what an earlier run worked out");

    // 4. The record kept what was asked, not what was sent.
    assert_eq!(report.record.prompt, "add a file");

    // 5. And so did the commit message, which outlives the record directory.
    let out = Command::new("git")
        .args([
            "log",
            "-1",
            "--format=%B",
            &format!("ostraka/{}", report.record.run_id),
        ])
        .current_dir(&f.repo)
        .output()
        .expect("git log runs");
    let message = String::from_utf8_lossy(&out.stdout);
    assert!(
        message.starts_with("add a file"),
        "the commit did not carry the task: {message}"
    );
    assert!(
        !message.contains("about this worktree"),
        "the preamble reached the commit message: {message}"
    );
}

#[test]
fn a_repository_that_keeps_its_own_notes_is_not_told_they_are_someone_elses() {
    // `prepare` leaves what the checkout brought: a repository tracking its own
    // `notes/` keeps it rather than having the workspace's linked over it. The
    // author prompt has to follow that, and asking the configuration does not —
    // it would say the directory is "not part of this repository" about a
    // directory the repository owns, and invite writing into the very diff the
    // change is judged on.
    let f = fixture("own-notes");
    // The repository's own notes, committed, so the worktree brings them.
    std::fs::create_dir_all(f.repo.join("notes")).expect("repo notes");
    std::fs::write(f.repo.join("notes/design.md"), "the repository's own\n").expect("write");
    git(&f.repo, &["add", "-A"]);
    git(&f.repo, &["commit", "-q", "-m", "notes of its own"]);

    // And a workspace notes directory, declared, as it would be for any run.
    let workspace_notes = f.repo.join("workspace-notes");
    std::fs::create_dir_all(&workspace_notes).expect("workspace notes");

    // Kept outside the worktree, which is released on success, and outside the
    // checkout, so it is not part of the change either.
    let seen_at = f.repo.join("seen-prompt.txt");
    let writer = agent(
        &f.repo,
        "writer",
        &format!(
            "printf '%s' \"$1\" > {}\necho new > added.txt",
            seen_at.display()
        ),
    );
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: Some(&workspace_notes),
        skills: None,
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");
    assert!(report.approved(), "expected approval: {:?}", report.refusal);

    let seen = std::fs::read_to_string(&seen_at).expect("the prompt the agent was given");
    assert_eq!(
        seen, "add a file",
        "the author was told about notes that were never linked:\n{seen}"
    );
}

#[test]
fn a_workspace_skill_reaches_the_agent_and_stays_out_of_the_change() {
    // The mirror of notes and the reason skills belong to the workspace rather
    // than to a vendor: every CLI here keeps its own idea of skills in a home
    // directory that isolation relocates, so a run depending on those would
    // answer differently on a different machine. One the workspace owns is one
    // every vendor gets and every machine reproduces.
    let f = fixture("skills");
    let skills = f.repo.join("workspace-skills");
    std::fs::create_dir_all(&skills).expect("skills dir");
    std::fs::write(
        skills.join("house-style.md"),
        "every file ends with a newline\n",
    )
    .expect("write");
    let notes = f.repo.join("workspace-notes");
    std::fs::create_dir_all(&notes).expect("notes dir");

    let writer = agent(
        &f.repo,
        "writer",
        // Proves it can read the skill, and keeps the instruction it was given.
        // Both land in notes/, which outlives the worktree an approved run
        // releases.
        "printf '%s' \"$1\" > notes/seen-prompt.txt\n\
         cat skills/house-style.md > notes/read-skill.txt\n\
         echo new > added.txt",
    );
    let reviewer = agent(&f.repo, "reviewer", &verdict("APPROVE"));
    let routing = route::select(
        &[writer, reviewer],
        Some("writer"),
        Some("reviewer"),
        &f.repo.join(".ostraka/vendor-home"),
        None,
    )
    .unwrap();

    let (worktrees, records) = places(&f);
    let places = orchestrator::Places {
        repo: &f.repo,
        worktrees: &worktrees,
        records: &records,
        name: "work",
        notes: Some(&notes),
        skills: Some(&skills),
    };
    let report = orchestrator::run_task(
        &places,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        None,
    )
    .expect("run completes");
    assert!(report.approved(), "expected approval: {:?}", report.refusal);

    // 1. The skill was readable from inside the worktree.
    let read = std::fs::read_to_string(notes.join("read-skill.txt"))
        .expect("the agent could not read the workspace's skill");
    assert_eq!(read, "every file ends with a newline\n");

    // 2. The author was told what it is, and told the rule before the history.
    let seen = std::fs::read_to_string(notes.join("seen-prompt.txt")).expect("the prompt");
    assert!(
        seen.contains("`skills/` is not part of this repository"),
        "{seen}"
    );
    let skills_at = seen.find("`skills/`").expect("skills named");
    let notes_at = seen.find("`notes/`").expect("notes named");
    assert!(skills_at < notes_at, "notes came first:\n{seen}");

    // 3. And none of it reached the change a reviewer judged.
    assert!(
        !report.diff.contains("house-style"),
        "the workspace's skills landed in the diff: {}",
        report.diff
    );
}
