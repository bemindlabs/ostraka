//! End-to-end: one task, two different adapters, a gate that really runs.
//!
//! These tests build a real git repository in a temp directory and drive the
//! orchestrator over it. The "vendors" are shell scripts, which is the point —
//! the runtime knows nothing about any particular CLI, so a script is as valid
//! an adapter as a commercial tool.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
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
    )
    .unwrap();

    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("add a test that is already there", "archon"),
        &ActorId::new("ephor"),
        &f.repo.join(".ostraka"),
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
    )
    .unwrap();

    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("change something", "archon"),
        &ActorId::new("ephor"),
        &f.repo.join(".ostraka"),
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
    )
    .unwrap();

    let records = f.repo.join(".ostraka");
    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("add a file", "archon"),
        &ActorId::new("ephor"),
        &records,
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
    )
    .unwrap();

    let report = orchestrator::run_task(
        &f.repo,
        &config("exit 1"),
        &routing,
        &task("break it", "archon"),
        &ActorId::new("ephor"),
        &f.repo.join(".ostraka"),
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
    )
    .unwrap();

    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("do too much", "archon"),
        &ActorId::new("ephor"),
        &f.repo.join(".ostraka"),
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
    )
    .unwrap();

    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("say nothing", "archon"),
        &ActorId::new("ephor"),
        &f.repo.join(".ostraka"),
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
    )
    .unwrap();

    // Same identity on both sides: the reviewing adapter differs, the actor does not.
    let report = orchestrator::run_task(
        &f.repo,
        &config("true"),
        &routing,
        &task("approve myself", "archon"),
        &ActorId::new("archon"),
        &f.repo.join(".ostraka"),
    )
    .expect("run completes");

    assert!(!report.approved());
    match report.refusal {
        Some(Refusal::SelfApproval { ref actor }) => assert_eq!(actor.as_str(), "archon"),
        other => panic!("expected a self-approval refusal, got {other:?}"),
    }
}
