//! `ostraka bench` — the same tasks through every candidate, judged by the gate.
//!
//! The question this answers is which agent to route which work to, and the
//! reason it can be answered here rather than by a leaderboard is that a run
//! already ends in an objective verdict: the project's own checks, executed in
//! a worktree, on the change that was actually written. A benchmark is that
//! verdict repeated across a matrix and tabulated. Nothing new judges anything.
//!
//! Three decisions are worth stating, because each of them is a way the numbers
//! could have been made to mean less than they appear to.
//!
//! **The reviewer is held constant.** Varying both ends measures pairs, not
//! candidates: a cell where a weak author met a strong reviewer and a cell
//! where the reverse happened are not two readings of the same instrument. The
//! runtime refuses a reviewer that is the author, so a candidate that *is* the
//! constant reviewer needs a stand-in — the next profile in `reviewers`. Those
//! cells are marked in the report, because a stand-in is a different instrument
//! and a table that hid that would be inviting exactly the comparison it cannot
//! support.
//!
//! **The gate is the score and the review is not.** A check that exits non-zero
//! is a fact about the change; a reviewer's verdict is an opinion about it, and
//! an opinion from a model that has its own preferences. Both are reported and
//! only the first is ranked on.
//!
//! **A cell that did not finish is not a cell that failed.** A rate limit, an
//! expired credential, a ceiling and a stop are outcomes about the run rather
//! than about the candidate, and averaging them into a pass rate would rank a
//! vendor by how reliably its billing worked. They are counted separately and
//! excluded from the rate.

use crate::run;
use crate::workspace::Workspace;
use ostraka_runtime::gate::Refusal;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

type Failure = Box<dyn std::error::Error>;

/// Where a workspace declares what to benchmark.
///
/// Beside the configuration rather than inside it: a gate is what a repository
/// agrees on and a benchmark is what an operator is curious about this week.
pub const FILE: &str = "bench.toml";

/// One piece of work, asked of every candidate unchanged.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Task {
    /// Short name for the column. Must be unique.
    pub id: String,
    /// What the agent is asked to do.
    pub prompt: String,
}

/// One thing being measured: a profile, and optionally the models to try it on.
///
/// A model is an identifier and identifiers arrive through configuration, which
/// is why they are named here and nowhere in the runtime. An entry with no
/// models is one cell on whatever that profile's own default is — which is
/// operator state and moves between machines, so the report says `default`
/// rather than pretending to know what it was.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Candidate {
    pub adapter: String,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Suite {
    /// Reviewer profiles, in preference order. The first is used for every
    /// candidate it is not; the rest stand in where it is.
    pub reviewers: Vec<String>,
    #[serde(default, rename = "task")]
    pub tasks: Vec<Task>,
    #[serde(default, rename = "candidate")]
    pub candidates: Vec<Candidate>,
}

impl Suite {
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let suite: Suite = toml::from_str(text)?;
        suite.validate()?;
        Ok(suite)
    }

    /// Refuses a suite that cannot produce a comparison, before anything is
    /// spent finding that out one vendor call at a time.
    pub fn validate(&self) -> Result<(), Failure> {
        if self.tasks.is_empty() {
            return Err("a benchmark with no tasks measures nothing".into());
        }
        if self.candidates.is_empty() {
            return Err("a benchmark with no candidates measures nothing".into());
        }
        if self.reviewers.is_empty() {
            return Err(
                "name at least one reviewer profile: every run is gated and reviewed, and \
                 a benchmark that skipped the review would not be measuring what Ostraka does"
                    .into(),
            );
        }
        let mut seen: Vec<&str> = Vec::new();
        for task in &self.tasks {
            if seen.contains(&task.id.as_str()) {
                return Err(format!("two tasks share the id {:?}", task.id).into());
            }
            seen.push(&task.id);
        }
        // A candidate whose only possible reviewer is itself cannot be run at
        // all, and finding that out after the first cell has been paid for is
        // the wrong time.
        for candidate in &self.candidates {
            if self.reviewers.iter().all(|r| *r == candidate.adapter) {
                return Err(format!(
                    "candidate {:?} is the only reviewer named, and the gate refuses a \
                     reviewer that is the author — name a second reviewer",
                    candidate.adapter
                )
                .into());
            }
        }
        Ok(())
    }

    /// Every cell, in the order they will be run.
    ///
    /// Task-major rather than candidate-major: a benchmark stopped halfway is
    /// far more useful having finished one task across every candidate than
    /// having finished every task on one.
    pub fn cells(&self) -> Vec<Cell> {
        let mut out = Vec::new();
        for task in &self.tasks {
            for candidate in &self.candidates {
                let models: Vec<Option<String>> = if candidate.models.is_empty() {
                    vec![None]
                } else {
                    candidate.models.iter().cloned().map(Some).collect()
                };
                for model in models {
                    let reviewer = self
                        .reviewers
                        .iter()
                        .find(|r| **r != candidate.adapter)
                        .expect("validate refused a candidate with no possible reviewer")
                        .clone();
                    out.push(Cell {
                        task: task.id.clone(),
                        prompt: task.prompt.clone(),
                        adapter: candidate.adapter.clone(),
                        model,
                        stood_in: reviewer != self.reviewers[0],
                        reviewer,
                    });
                }
            }
        }
        out
    }
}

/// One run that a benchmark will make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub task: String,
    pub prompt: String,
    pub adapter: String,
    pub model: Option<String>,
    pub reviewer: String,
    /// The preferred reviewer was this candidate, so a stand-in judged it.
    pub stood_in: bool,
}

impl Cell {
    /// How this candidate is named in a report. One string, so that the same
    /// name identifies it in a row, in a ranking and in JSON.
    pub fn candidate(&self) -> String {
        match &self.model {
            Some(model) => format!("{}:{model}", self.adapter),
            None => format!("{}:default", self.adapter),
        }
    }
}

/// What a cell turned into.
///
/// The three groups are deliberate. `Approved` and `Rejected` are verdicts on
/// the change; `Incomplete` is everything that never reached one, and it does
/// not enter the pass rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Verdict {
    /// The gate passed and an independent reviewer approved.
    Approved,
    /// A required check failed. The change was judged and found wanting, which
    /// is the outcome a benchmark is really asking about.
    ChecksFailed { failed: Vec<String> },
    /// The checks passed and the reviewer said no.
    Rejected { reason: String },
    /// The candidate ran cleanly and wrote nothing.
    NoChange,
    /// Nothing was judged: the agent, the environment or the clock stopped it.
    Incomplete { said: String },
}

impl Verdict {
    /// Whether this cell is one the pass rate is computed over.
    pub fn judged(&self) -> bool {
        !matches!(self, Verdict::Incomplete { .. })
    }

    pub fn passed(&self) -> bool {
        matches!(self, Verdict::Approved)
    }

    /// Whether the project's own checks passed, regardless of what the reviewer
    /// then said. The objective half.
    pub fn gate_passed(&self) -> bool {
        matches!(self, Verdict::Approved | Verdict::Rejected { .. })
    }

    fn describe(&self) -> String {
        match self {
            Verdict::Approved => "approved".to_string(),
            Verdict::ChecksFailed { failed } => format!("checks failed: {}", failed.join(", ")),
            Verdict::Rejected { .. } => "gate passed, review rejected".to_string(),
            Verdict::NoChange => "wrote nothing".to_string(),
            Verdict::Incomplete { said } => format!("did not finish: {said}"),
        }
    }
}

/// One finished cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Result_ {
    pub task: String,
    pub candidate: String,
    pub adapter: String,
    pub model: Option<String>,
    pub reviewer: String,
    pub stood_in: bool,
    pub verdict: Verdict,
    /// The run this came from, so `ostraka replay <id>` reaches the evidence.
    pub run_id: Option<String>,
    pub seconds: u64,
    /// What the authoring invocation cost, as that vendor reported it. Absent
    /// for a vendor that reports nothing — which is a different claim from
    /// zero, and is why this is an option rather than a default.
    pub author_tokens: Option<u64>,
}

/// Turns a finished run into a verdict.
///
/// Reads the refusal rather than the outcome, because `Outcome::Rejected`
/// covers both "a check failed" and "the reviewer said no" and those are the
/// two answers a benchmark most needs to keep apart.
///
/// Matched exhaustively and deliberately so: a refusal variant added later
/// stops this compiling rather than falling through to something. Nothing here
/// may default to a pass — rule 5.
fn verdict_of(report: &ostraka_runtime::orchestrator::RunReport) -> Verdict {
    match &report.refusal {
        None => Verdict::Approved,
        Some(Refusal::ChecksFailed { failed, .. }) => Verdict::ChecksFailed {
            failed: failed.clone(),
        },
        Some(Refusal::Rejected { reason }) => Verdict::Rejected {
            reason: reason.clone(),
        },
        Some(Refusal::NoChange) => Verdict::NoChange,
        Some(Refusal::AuthorFailed { code, .. }) => Verdict::Incomplete {
            said: format!("the agent exited {code}"),
        },
        Some(Refusal::SetupFailed { step, reason }) => Verdict::Incomplete {
            said: format!("the worktree was not ready: {step} — {reason}"),
        },
        Some(Refusal::TimedOut { after_secs }) => Verdict::Incomplete {
            said: format!("the ceiling of {after_secs}s ran out"),
        },
        Some(Refusal::Interrupted) => Verdict::Incomplete {
            said: "stopped".to_string(),
        },
        Some(Refusal::PolicyViolation { reason }) => Verdict::Incomplete {
            said: format!("policy: {reason}"),
        },
        Some(Refusal::SelfApproval { actor }) => Verdict::Incomplete {
            said: format!("{} was asked to approve its own change", actor.as_str()),
        },
    }
}

/// How each candidate did across every task it was asked.
#[derive(Debug, Clone, Serialize)]
pub struct Standing {
    pub candidate: String,
    pub judged: usize,
    pub approved: usize,
    pub gate_passed: usize,
    pub incomplete: usize,
    pub seconds: u64,
    pub tokens: Option<u64>,
    /// A stand-in reviewer judged at least one of these cells.
    pub stood_in: bool,
}

impl Standing {
    /// Out of the cells that reached a verdict. `None` where none did — a
    /// candidate every one of whose runs fell over has no rate, and printing
    /// 0% would say its work was bad rather than that it never arrived.
    pub fn rate(&self) -> Option<f64> {
        (self.judged > 0).then(|| self.approved as f64 / self.judged as f64)
    }
}

/// The standings, best first.
///
/// Ranked on approvals over cells that were judged, then on the objective half
/// alone, then on time. A candidate with no judged cells sorts last however
/// fast it was: there is nothing to rank it on.
pub fn standings(results: &[Result_]) -> Vec<Standing> {
    let mut out: Vec<Standing> = Vec::new();
    for r in results {
        let row = match out.iter_mut().find(|s| s.candidate == r.candidate) {
            Some(row) => row,
            None => {
                out.push(Standing {
                    candidate: r.candidate.clone(),
                    judged: 0,
                    approved: 0,
                    gate_passed: 0,
                    incomplete: 0,
                    seconds: 0,
                    tokens: None,
                    stood_in: false,
                });
                out.last_mut().expect("just pushed")
            }
        };
        row.seconds += r.seconds;
        row.stood_in |= r.stood_in;
        if let Some(t) = r.author_tokens {
            row.tokens = Some(row.tokens.unwrap_or(0) + t);
        }
        if r.verdict.judged() {
            row.judged += 1;
        } else {
            row.incomplete += 1;
        }
        if r.verdict.passed() {
            row.approved += 1;
        }
        if r.verdict.gate_passed() {
            row.gate_passed += 1;
        }
    }
    out.sort_by(|a, b| {
        b.rate()
            .unwrap_or(-1.0)
            .partial_cmp(&a.rate().unwrap_or(-1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.gate_passed.cmp(&a.gate_passed))
            .then(a.seconds.cmp(&b.seconds))
            .then(a.candidate.cmp(&b.candidate))
    });
    out
}

/// Reads the suite a workspace declares.
pub fn load(workspace: &Workspace) -> Result<Suite, Failure> {
    let path = workspace.ostraka().join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| -> Failure {
        format!(
            "no benchmark declared at {}: {e}\n\n{}",
            path.display(),
            EXAMPLE
        )
        .into()
    })?;
    Suite::parse(&text)
}

/// What to write when there is nothing to read.
///
/// Printed rather than created: a command that reports what is missing should
/// not answer by writing a file nobody asked for, and the shape is four lines.
pub const EXAMPLE: &str = "\
A benchmark is a file. The shape:

    reviewers = [\"profile-a\", \"profile-b\"]

    [[task]]
    id = \"a-short-name\"
    prompt = \"what every candidate is asked to do\"

    [[candidate]]
    adapter = \"profile-a\"
    models = [\"an-id-this-machine-can-run\"]";

/// Runs the whole matrix.
///
/// Sequential, and that is the current answer rather than a permanent one: a
/// run holds a single interrupt flag, so two at once would both stop when
/// either was asked to. What that costs is wall-clock; what it buys is a
/// benchmark that can be stopped.
pub fn measure(
    workspace: &Workspace,
    suite: &Suite,
    cells: &[Cell],
    mut say: impl FnMut(&Cell, usize, usize),
) -> Vec<Result_> {
    let _ = suite;
    let mut results = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        say(cell, i + 1, cells.len());
        // Cleared before the run rather than inside it: a stop asked for in the
        // same breath as a run would otherwise be wiped a moment later.
        ostraka_adapter::interrupt::clear();
        let started = Instant::now();
        let args = run::Args {
            prompt: cell.prompt.clone(),
            repository: None,
            // The identity is the profile id, deliberately. Everywhere else an
            // actor is a person or a role and the adapter is recorded beside
            // it; here the profile *is* the thing on trial, so a record saying
            // "written by grok, reviewed by claude-code" is the sentence a
            // benchmark wants, and `ostraka runs` lists the matrix rather than
            // fifty rows all attributed to "author".
            //
            // It does not weaken the self-approval guard. That compares the two
            // ActorIds, and these two are the two profile ids, which `cells()`
            // has already established differ — so the guard is being handed a
            // stricter pair than usual, not a looser one.
            author: cell.adapter.clone(),
            reviewer: cell.reviewer.clone(),
            adapter: Some(cell.adapter.clone()),
            review_adapter: Some(cell.reviewer.clone()),
            base_ref: run::BASE_REF.to_string(),
            from: None,
            model: cell.model.clone(),
            // One. A benchmark measures what a profile does with a task, and a
            // loop would measure what it does with three.
            attempts: 1,
        };
        let (verdict, run_id, tokens) = match run::execute(
            workspace,
            &args,
            None,
            &ostraka_adapter::interrupt::Stop::new(),
        ) {
            Ok(report) => {
                let tokens = report
                    .record
                    .usage
                    .iter()
                    .filter(|u| u.role == "author")
                    .map(|u| u.counted())
                    .reduce(|a, b| a + b);
                (
                    verdict_of(&report),
                    Some(report.record.run_id.clone()),
                    tokens,
                )
            }
            // A run that could not start at all is still a cell, and saying so
            // is worth more than a benchmark that stops because one profile is
            // logged out.
            Err(e) => (
                Verdict::Incomplete {
                    said: e.to_string().lines().next().unwrap_or("").to_string(),
                },
                None,
                None,
            ),
        };
        results.push(Result_ {
            task: cell.task.clone(),
            candidate: cell.candidate(),
            adapter: cell.adapter.clone(),
            model: cell.model.clone(),
            reviewer: cell.reviewer.clone(),
            stood_in: cell.stood_in,
            verdict,
            run_id,
            seconds: started.elapsed().as_secs(),
            author_tokens: tokens,
        });
        if ostraka_adapter::interrupt::requested() {
            break;
        }
    }
    results
}

/// Writes the report beside the run records it points at.
fn keep(workspace: &Workspace, results: &[Result_]) -> Result<std::path::PathBuf, Failure> {
    let dir = workspace.ostraka().join("bench");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", stamp()));
    std::fs::write(&path, serde_json::to_string_pretty(&report(results))?)?;
    Ok(path)
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("b{secs}-{}", std::process::id())
}

pub fn report(results: &[Result_]) -> serde_json::Value {
    serde_json::json!({
        "results": results,
        "standings": standings(results),
    })
}

/// The table, as a string, so it can be asserted rather than looked at.
pub fn table(results: &[Result_]) -> String {
    let mut out = String::new();
    let mut tasks: Vec<&str> = Vec::new();
    for r in results {
        if !tasks.contains(&r.task.as_str()) {
            tasks.push(&r.task);
        }
    }
    for task in tasks {
        out.push_str(&format!("{task}\n"));
        let rows: Vec<&Result_> = results.iter().filter(|r| r.task == task).collect();
        let pad = rows.iter().map(|r| r.candidate.len()).max().unwrap_or(0);
        for r in rows {
            let mark = if r.verdict.passed() { "ok  " } else { "    " };
            let note = if r.stood_in {
                "  (stand-in reviewer)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {mark}{:<pad$}  {:>4}s  {}{note}\n",
                r.candidate,
                r.seconds,
                r.verdict.describe(),
            ));
        }
        out.push('\n');
    }

    out.push_str("standings\n");
    for s in standings(results) {
        let rate = match s.rate() {
            Some(rate) => format!("{:>3.0}%", rate * 100.0),
            None => "   —".to_string(),
        };
        let tokens = match s.tokens {
            Some(t) => format!("{:>8}", thousands(t)),
            None => "       —".to_string(),
        };
        let note = if s.stood_in {
            "  a stand-in reviewer judged at least one cell"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {rate}  {:<24}  {} approved / {} judged  gate {}  {} incomplete  {:>5}s {tokens}{note}\n",
            s.candidate, s.approved, s.judged, s.gate_passed, s.incomplete, s.seconds
        ));
    }
    out
}

/// "1 task", "2 tasks". A count printed before somebody decides whether to
/// spend it should not also be the thing they notice about the sentence.
fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn thousands(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Every profile the suite names that this workspace has not configured.
///
/// A suite is written by hand and a profile id is a string, so a typo is the
/// likeliest thing wrong with a benchmark — and the cost of finding out from
/// routing is one failed cell per occurrence, each of which has already been
/// paid for. Checked once, before anything runs, over candidates and reviewers
/// alike: a misspelled reviewer takes down every cell rather than one.
pub fn unknown(suite: &Suite, configured: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let named = suite
        .candidates
        .iter()
        .map(|c| c.adapter.clone())
        .chain(suite.reviewers.iter().cloned());
    for id in named {
        if !configured.contains(&id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// What checking a suite's models against its profiles found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModelCheck {
    /// Models a candidate names that its profile does not offer, each with
    /// what the profile does offer, so the refusal can say what to write.
    pub unoffered: Vec<(String, String, Vec<String>)>,
    /// Profiles whose models could not be listed, with why. Nothing named on
    /// them was checked, and that is said rather than guessed at.
    pub unchecked: Vec<(String, String)>,
}

/// Every model a candidate names, against what its profile lists.
///
/// The dry run printed the matrix whatever the models said, so a mistyped or
/// retired model id surfaced as an authoring failure in a paid cell — one per
/// task it was crossed with. A profile's `[models]` table is where what it can
/// run is written down, and the model picker already reads it; this reads the
/// same catalogs.
///
/// A candidate naming no model runs its profile's default and has nothing to
/// check. A profile with no `[models]` table, or whose listing failed or came
/// back empty, cannot say what it offers, so the models named on it are
/// reported as unchecked with the reason — not passed as fine, and not refused
/// on a guess.
pub fn check_models(suite: &Suite, catalogs: &[crate::models::Catalog]) -> ModelCheck {
    let mut found = ModelCheck::default();
    for candidate in &suite.candidates {
        if candidate.models.is_empty() {
            continue;
        }
        let catalog = catalogs.iter().find(|c| c.profile == candidate.adapter);
        let why = match catalog {
            // `catalog` answers nothing both for a profile with no `[models]`
            // table and for one whose table does not parse, so the sentence
            // covers both rather than naming one of them wrongly.
            None => Some(format!(
                "`{}` has no readable [models] table, so what it can run cannot be listed",
                candidate.adapter
            )),
            Some(c) if c.models.is_empty() => Some(
                c.note
                    .clone()
                    .unwrap_or_else(|| format!("`{}` listed no models", candidate.adapter)),
            ),
            Some(c) => c.note.clone(),
        };
        if let Some(why) = why {
            if !found
                .unchecked
                .iter()
                .any(|(id, _)| *id == candidate.adapter)
            {
                found.unchecked.push((candidate.adapter.clone(), why));
            }
            continue;
        }
        let offered = catalog.map(|c| c.models.clone()).unwrap_or_default();
        for model in &candidate.models {
            if !offered.contains(model) {
                found
                    .unoffered
                    .push((candidate.adapter.clone(), model.clone(), offered.clone()));
            }
        }
    }
    found
}

/// `ostraka bench`.
pub fn run(workspace: &Workspace, dry_run: bool, json: bool) -> Result<bool, Failure> {
    let suite = load(workspace)?;

    // Not `unwrap_or_default()`. A workspace with no `adapters/` directory, or
    // one holding a profile that does not parse, would otherwise be reported as
    // "configured here: none" — which reads as "you have written no profiles"
    // and is a different thing from "one of them is broken". Seen while
    // testing this: a workspace whose profiles were somewhere else entirely
    // produced a confident empty list and no hint that anything had failed.
    let configured: Vec<String> = workspace
        .profiles()
        .map_err(|e| -> Failure {
            format!("the benchmark could not read this workspace's adapter profiles: {e}").into()
        })?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let unknown = unknown(&suite, &configured);
    if !unknown.is_empty() {
        return Err(format!(
            "the benchmark names {} this workspace has not configured: {}\n\nconfigured here: {}",
            if unknown.len() == 1 {
                "a profile"
            } else {
                "profiles"
            },
            unknown.join(", "),
            if configured.is_empty() {
                "none".to_string()
            } else {
                configured.join(", ")
            },
        )
        .into());
    }

    let cells = suite.cells();

    if dry_run {
        // Before the matrix is printed, because the matrix is what somebody
        // decides to pay for. A model its profile does not offer is refused by
        // name; a profile that cannot list its models is said to be unchecked.
        // Listing runs each profile's own listing command, which is why it is
        // done here and not on every load of the suite.
        let checked = check_models(&suite, &crate::models::catalogs(&workspace.adapters()));
        if !checked.unoffered.is_empty() {
            let lines: Vec<String> = checked
                .unoffered
                .iter()
                .map(|(profile, model, offered)| {
                    format!(
                        "  {profile}: `{model}` \u{2014} it offers {}",
                        if offered.is_empty() {
                            "nothing".to_string()
                        } else {
                            offered.join(", ")
                        }
                    )
                })
                .collect();
            return Err(format!(
                "the benchmark names {} that {} profile does not offer:\n{}",
                if lines.len() == 1 {
                    "a model"
                } else {
                    "models"
                },
                if lines.len() == 1 { "its" } else { "their" },
                lines.join("\n")
            )
            .into());
        }
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cells": cells.iter().map(|c| serde_json::json!({
                        "task": c.task,
                        "candidate": c.candidate(),
                        "reviewer": c.reviewer,
                        "stood_in": c.stood_in,
                    })).collect::<Vec<_>>(),
                    "unchecked": checked.unchecked.iter().map(|(profile, why)| serde_json::json!({
                        "profile": profile,
                        "why": why,
                    })).collect::<Vec<_>>(),
                }))?
            );
        } else {
            for (profile, why) in &checked.unchecked {
                println!("models on {profile} not checked: {why}");
            }
            // What it will spend, before it spends it. Every cell is a paid
            // authoring call and a paid review, and a matrix multiplies faster
            // than it reads.
            // Counted from the expanded cells rather than from `candidates`,
            // because a candidate listing three models is three of these — and
            // calling that "1 candidate" would understate the bill by three
            // times at exactly the moment somebody is deciding whether to pay
            // it. The word is what changes: these are the things being
            // measured, and a profile with two models is two of them.
            let per_task = cells.len() / suite.tasks.len().max(1);
            println!(
                "{} cells: {} x {}. Each one is an authoring run and a review.",
                cells.len(),
                plural(suite.tasks.len(), "task"),
                plural(per_task, "candidate/model pair"),
            );
            for cell in &cells {
                let note = if cell.stood_in { "  (stand-in)" } else { "" };
                println!(
                    "  {:<20} {:<28} reviewed by {}{note}",
                    cell.task,
                    cell.candidate(),
                    cell.reviewer
                );
            }
        }
        return Ok(true);
    }

    let results = measure(workspace, &suite, &cells, |cell, n, of| {
        if !json {
            eprintln!("[{n}/{of}] {} — {}", cell.task, cell.candidate());
        }
    });

    let kept = keep(workspace, &results)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report(&results))?);
    } else {
        println!();
        print!("{}", table(&results));
        println!();
        println!("kept at {}", relative(workspace, &kept));
    }
    // A benchmark that ran is a successful benchmark, whatever it found.
    Ok(true)
}

fn relative(workspace: &Workspace, path: &Path) -> String {
    path.strip_prefix(workspace.ostraka().parent().unwrap_or(path))
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUITE: &str = r#"
reviewers = ["ref", "alt"]

[[task]]
id = "one"
prompt = "do the first thing"

[[task]]
id = "two"
prompt = "do the second thing"

[[candidate]]
adapter = "cand"
models = ["m1", "m2"]

[[candidate]]
adapter = "ref"
"#;

    #[test]
    fn a_suite_expands_to_one_cell_per_task_candidate_and_model() {
        let suite = Suite::parse(SUITE).expect("parses");
        let cells = suite.cells();
        // Two tasks x (two models on the first candidate + one default on the
        // second).
        assert_eq!(cells.len(), 6, "{cells:#?}");
        assert_eq!(
            cells[0].candidate(),
            "cand:m1",
            "a named model is part of the candidate's name"
        );
        assert_eq!(
            cells[2].candidate(),
            "ref:default",
            "a candidate with no model is not pretending to know which one ran"
        );
        // Task-major: everything about task one before anything about task two.
        assert!(cells[..3].iter().all(|c| c.task == "one"), "{cells:#?}");
    }

    #[test]
    fn a_candidate_that_is_the_reviewer_gets_a_stand_in_and_is_marked() {
        let suite = Suite::parse(SUITE).expect("parses");
        let cells = suite.cells();
        // The gate refuses a reviewer that is the author, so this cell could
        // not otherwise be run at all.
        let against_ref = cells
            .iter()
            .find(|c| c.adapter == "ref")
            .expect("the reviewer is also a candidate here");
        assert_eq!(against_ref.reviewer, "alt");
        assert!(
            against_ref.stood_in,
            "a stand-in was used and the report would not have said so"
        );
        let ordinary = cells
            .iter()
            .find(|c| c.adapter == "cand")
            .expect("the other candidate");
        assert_eq!(ordinary.reviewer, "ref");
        assert!(!ordinary.stood_in);
    }

    #[test]
    fn a_suite_that_cannot_be_run_is_refused_before_anything_is_spent() {
        // Every one of these costs a vendor call per cell to discover at run
        // time, which is the wrong moment.
        for (text, want) in [
            ("reviewers = [\"a\"]\n", "no tasks"),
            (
                "reviewers = [\"a\"]\n[[task]]\nid=\"x\"\nprompt=\"y\"\n",
                "no candidates",
            ),
            (
                "reviewers = []\n[[task]]\nid=\"x\"\nprompt=\"y\"\n[[candidate]]\nadapter=\"a\"\n",
                "at least one reviewer",
            ),
            (
                "reviewers = [\"a\"]\n[[task]]\nid=\"x\"\nprompt=\"y\"\n[[candidate]]\nadapter=\"a\"\n",
                "the only reviewer named",
            ),
            (
                "reviewers = [\"a\"]\n[[task]]\nid=\"x\"\nprompt=\"y\"\n[[task]]\nid=\"x\"\nprompt=\"z\"\n[[candidate]]\nadapter=\"b\"\n",
                "share the id",
            ),
        ] {
            let err = Suite::parse(text).expect_err("should be refused");
            assert!(err.to_string().contains(want), "wanted {want:?} in {err}");
        }
    }

    fn result(candidate: &str, task: &str, verdict: Verdict, seconds: u64) -> Result_ {
        Result_ {
            task: task.to_string(),
            candidate: candidate.to_string(),
            adapter: candidate.to_string(),
            model: None,
            reviewer: "ref".to_string(),
            stood_in: false,
            verdict,
            run_id: None,
            seconds,
            author_tokens: None,
        }
    }

    #[test]
    fn a_run_that_never_reached_a_verdict_does_not_count_against_the_candidate() {
        // The failure this rules out: a vendor that hit a rate limit ranking
        // below one that wrote broken code, because both were counted as
        // failures. One of those is about the work.
        let results = vec![
            result("a", "one", Verdict::Approved, 1),
            result(
                "a",
                "two",
                Verdict::Incomplete {
                    said: "rate limited".into(),
                },
                1,
            ),
            result("b", "one", Verdict::Approved, 1),
            result(
                "b",
                "two",
                Verdict::ChecksFailed {
                    failed: vec!["test".into()],
                },
                1,
            ),
        ];
        let table = standings(&results);
        let a = table.iter().find(|s| s.candidate == "a").expect("a");
        let b = table.iter().find(|s| s.candidate == "b").expect("b");
        assert_eq!((a.judged, a.approved, a.incomplete), (1, 1, 1));
        assert_eq!((b.judged, b.approved, b.incomplete), (2, 1, 0));
        assert_eq!(a.rate(), Some(1.0));
        assert_eq!(b.rate(), Some(0.5));
        assert_eq!(table[0].candidate, "a", "{table:#?}");
    }

    #[test]
    fn a_candidate_that_never_arrived_has_no_rate_and_sorts_last() {
        // Zero percent would say its work was bad. Nothing of its work was
        // seen.
        let results = vec![
            result(
                "gone",
                "one",
                Verdict::Incomplete {
                    said: "not logged in".into(),
                },
                0,
            ),
            result(
                "here",
                "one",
                Verdict::ChecksFailed {
                    failed: vec!["fmt".into()],
                },
                9,
            ),
        ];
        let table = standings(&results);
        assert_eq!(table[0].candidate, "here", "{table:#?}");
        assert_eq!(table[1].candidate, "gone");
        assert_eq!(table[1].rate(), None);
        // And the table says so with a dash rather than a percentage. The
        // other row's 0% is earned: its checks were run and they failed.
        let printed = super::table(&results);
        let standing = |name: &str| -> String {
            printed
                .lines()
                .find(|l| l.contains(name) && l.contains("judged"))
                .unwrap_or_else(|| panic!("no standing for {name} in:\n{printed}"))
                .to_string()
        };
        assert!(standing("gone").contains('—'), "{printed}");
        assert!(!standing("gone").contains('%'), "{printed}");
        assert!(standing("here").contains("0%"), "{printed}");
    }

    #[test]
    fn the_gate_and_the_review_are_reported_apart() {
        // A change the project's own checks accepted and a reviewer did not is
        // a different finding from one that failed the checks, and collapsing
        // them would hide the only objective half of the measurement.
        let results = vec![
            result(
                "a",
                "one",
                Verdict::Rejected {
                    reason: "naming".into(),
                },
                1,
            ),
            result(
                "b",
                "one",
                Verdict::ChecksFailed {
                    failed: vec!["clippy".into()],
                },
                1,
            ),
        ];
        let table = standings(&results);
        let a = table.iter().find(|s| s.candidate == "a").expect("a");
        let b = table.iter().find(|s| s.candidate == "b").expect("b");
        assert_eq!((a.approved, a.gate_passed), (0, 1));
        assert_eq!((b.approved, b.gate_passed), (0, 0));
    }

    #[test]
    fn a_candidate_listing_models_is_counted_once_per_model() {
        // The dry run prints this count so somebody can decide whether to pay
        // for it, and it is derived from the expanded cells: a profile with two
        // models is two authoring runs and two reviews, not one of each.
        let suite = Suite::parse(SUITE).expect("parses");
        let per_task = suite.cells().len() / suite.tasks.len();
        assert_eq!(
            per_task, 3,
            "two candidates, one of which lists two models, is three cells a task"
        );
        assert_eq!(suite.candidates.len(), 2, "and only two candidates");
    }

    fn catalog(profile: &str, models: &[&str], note: Option<&str>) -> crate::models::Catalog {
        crate::models::Catalog {
            profile: profile.to_string(),
            models: models.iter().map(|m| (*m).to_string()).collect(),
            note: note.map(str::to_string),
        }
    }

    #[test]
    fn a_model_its_profile_does_not_offer_is_named_and_one_it_does_is_not() {
        let suite = Suite::parse(SUITE).expect("parses");
        // `cand` names m1 and m2; `ref` names none and runs its default.
        let all_there = check_models(&suite, &[catalog("cand", &["m1", "m2", "m3"], None)]);
        assert_eq!(all_there, ModelCheck::default());

        let one_missing = check_models(&suite, &[catalog("cand", &["m1", "m3"], None)]);
        assert_eq!(
            one_missing.unoffered,
            vec![(
                "cand".to_string(),
                "m2".to_string(),
                vec!["m1".to_string(), "m3".to_string()]
            )]
        );
        assert!(one_missing.unchecked.is_empty());
    }

    /// A profile that cannot say what it offers is not a profile whose models
    /// are wrong. Refusing would be a guess, and so would passing them.
    #[test]
    fn a_profile_that_cannot_list_its_models_is_said_to_be_unchecked() {
        let suite = Suite::parse(SUITE).expect("parses");

        let no_table = check_models(&suite, &[]);
        assert!(no_table.unoffered.is_empty(), "{no_table:?}");
        assert_eq!(no_table.unchecked.len(), 1, "{no_table:?}");
        assert_eq!(no_table.unchecked[0].0, "cand");
        assert!(
            no_table.unchecked[0]
                .1
                .contains("no readable [models] table"),
            "{no_table:?}"
        );

        let failed = check_models(
            &suite,
            &[catalog("cand", &[], Some("`cand models` could not be run"))],
        );
        assert!(failed.unoffered.is_empty(), "{failed:?}");
        assert_eq!(failed.unchecked[0].1, "`cand models` could not be run");
    }

    /// The same two paths through `ostraka bench --dry-run` itself, against a
    /// workspace whose profiles list their models with `known`, so nothing
    /// is run to find out.
    #[test]
    fn the_dry_run_refuses_a_model_by_name_and_reports_what_it_could_not_check() {
        // Named for this test as well as this process, so no other test in
        // the same run can be handed the same directory.
        let dir = std::env::temp_dir().join(format!(
            "ostraka-bench-models-{}-dry-run-refuses",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let adapters = dir.join(".ostraka/adapters");
        std::fs::create_dir_all(&adapters).expect("adapters");
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        let profile = |id: &str, extra: &str| {
            std::fs::write(
                adapters.join(format!("{id}.toml")),
                format!("id = \"{id}\"\ncommand = \"true\"\nargs = [\"{{{{prompt}}}}\"]\n{extra}"),
            )
            .expect("profile");
        };
        profile("cand", "\n[models]\nknown = [\"m1\", \"m2\"]\n");
        profile("plain", "");
        profile("ref", "");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"t\", cmd = \"true\", required = true }]\n",
        )
        .expect("config");
        let bench = |candidates: &str| {
            std::fs::write(
                dir.join(".ostraka").join(FILE),
                format!(
                    "reviewers = [\"ref\"]\n\n[[task]]\nid = \"one\"\nprompt = \"do it\"\n\n{candidates}"
                ),
            )
            .expect("bench");
        };
        let workspace = Workspace::at(&dir);

        bench("[[candidate]]\nadapter = \"cand\"\nmodels = [\"m1\", \"m9\"]\n");
        let refused = run(&workspace, true, true).expect_err("m9 is not offered");
        let said = refused.to_string();
        assert!(said.contains("cand"), "{said}");
        assert!(said.contains("`m9`"), "{said}");
        assert!(
            said.contains("m1, m2"),
            "it did not say what is offered: {said}"
        );

        bench("[[candidate]]\nadapter = \"cand\"\nmodels = [\"m1\", \"m2\"]\n");
        assert!(run(&workspace, true, true).expect("offered"));

        // A profile with no [models] table: not refused, and not waved through
        // silently either — the JSON says it was not checked.
        bench("[[candidate]]\nadapter = \"plain\"\nmodels = [\"anything\"]\n");
        assert!(run(&workspace, true, true).expect("unchecked is not a refusal"));
        let checked = check_models(
            &load(&workspace).expect("loads"),
            &crate::models::catalogs(&workspace.adapters()),
        );
        assert_eq!(checked.unchecked.len(), 1, "{checked:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_profile_the_workspace_does_not_have_is_named_before_anything_runs() {
        // A profile id is a string in a file somebody typed, so a typo is the
        // likeliest fault in a suite — and finding it from routing costs one
        // paid cell per occurrence. Reviewers are checked too: a misspelled
        // reviewer takes down every cell rather than one of them.
        let suite = Suite::parse(SUITE).expect("parses");
        assert!(unknown(&suite, &["cand".into(), "ref".into(), "alt".into()]).is_empty());
        assert_eq!(
            unknown(&suite, &["cand".into(), "ref".into()]),
            vec!["alt".to_string()],
            "a reviewer with no profile went unmentioned"
        );
        assert_eq!(
            unknown(&suite, &[]),
            vec!["cand".to_string(), "ref".to_string(), "alt".to_string()],
        );
    }

    #[test]
    fn the_table_names_the_run_a_cell_came_from_in_json() {
        // A number nobody can get back to the evidence for is a number nobody
        // should act on. `ostraka replay <id>` is the way back.
        let mut r = result("a", "one", Verdict::Approved, 3);
        r.run_id = Some("r-1".into());
        let json = report(&[r]).to_string();
        assert!(json.contains("r-1"), "{json}");
    }
}
