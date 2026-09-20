//! What a directory still needs before it can run anything, in order.
//!
//! `init` writes two files and a listing. Whether the project can actually run
//! was found out later, by a run failing: not a git repository, a gate that is
//! still the placeholder written to fail on purpose, one CLI configured so no
//! independent reviewer exists, or a CLI that is installed and logged out.
//!
//! The steps are in the order of what costs most when it is wrong, and they are
//! **derived from what is on disk** rather than remembered. That is what makes
//! `init` resume: run it again and the steps already taken are done, whoever
//! took them and however. It is also why `init`, `check` and the browser can
//! all show the same list without keeping three copies of it.
//!
//! The first step asks [`crate::remedy`] rather than repeating it, so the
//! knowledge of what a directory needs to hold a run lives in one place.

use crate::workspace::Workspace;
use std::path::Path;

/// The steps, in the order they are asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    Repository,
    Gate,
    Pair,
    Auth,
    Policy,
    Extras,
}

/// Where a step stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing to do: what this step is about is already true.
    Done,
    /// Something to do, and this can offer to do it.
    Todo,
    /// Something to do that only a person can do, and why.
    Yours(String),
}

/// What taking a step would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// Nothing to offer: the step is done, or it is somebody's to take.
    Nothing,
    /// The commands `remedy.rs` would run, in order.
    Commands(Vec<Vec<String>>),
    /// Run the gate's checks once, here and now.
    RunGate,
    /// Write profiles for these installed CLIs.
    WriteProfiles(Vec<String>),
    /// Ask each chosen CLI whether it can run.
    Probe,
}

/// One step: what it is, why it matters, where it stands, what it would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: Id,
    pub title: &'static str,
    /// Why this step is worth taking, in one sentence. Read aloud by every
    /// command that lists the steps, because a step nobody understands is a
    /// step everybody skips.
    pub why: &'static str,
    pub state: State,
    /// What is true now: the checks there are, the pair a run would use, what a
    /// probe answered.
    pub detail: Vec<String>,
    pub offer: Offer,
}

impl Step {
    pub fn done(&self) -> bool {
        self.state == State::Done
    }
}

/// What is true on disk, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The workspace has an `ostraka.toml`.
    pub declared: bool,
    /// The repository the steps are about, where there is one.
    pub repository: Option<String>,
    /// What `remedy.rs` says is in the way, and the commands out of it.
    pub repository_problem: Option<String>,
    pub repository_commands: Vec<Vec<String>>,
    /// The gate's checks, by name.
    pub checks: Vec<String>,
    /// The gate is still the check `init` writes to fail on purpose.
    pub placeholder_gate: bool,
    /// Whether this exact gate has been run here, and whether it passed.
    /// `None` means it never has, or the checks have changed since.
    pub gate_tried: Option<bool>,
    /// The pair a run started now would use: author, reviewer.
    pub pair: Option<(String, String)>,
    /// CLIs installed here with no profile in this workspace.
    pub installed_unconfigured: Vec<String>,
    /// Chosen profiles that answered a probe with something other than yes.
    pub unready: Vec<(String, String)>,
    /// The policy's ceiling and path rule, where the configuration was read.
    pub policy: Option<(Option<u64>, usize, bool)>,
    pub notes: bool,
    pub tasks: bool,
}

/// The check `init` writes when it cannot tell what a project is. It fails on
/// purpose; a workspace still carrying it has not had its gate written.
pub const PLACEHOLDER_CHECK: &str = "declare-your-checks";

/// The steps, in order, from what is on disk.
///
/// Pure: every question about the world was answered by [`gather`] already.
pub fn steps(facts: &Facts) -> Vec<Step> {
    vec![
        repository(facts),
        gate(facts),
        pair(facts),
        auth(facts),
        policy(facts),
        extras(facts),
    ]
}

/// The steps as a script can read them.
///
/// `--json` is one object, and a command that says what is left in prose and
/// nothing in JSON leaves a script to parse the prose or go without.
pub fn as_json(steps: &[Step]) -> serde_json::Value {
    serde_json::Value::Array(
        steps
            .iter()
            .map(|step| {
                serde_json::json!({
                    "step": format!("{:?}", step.id).to_lowercase(),
                    "title": step.title,
                    "why": step.why,
                    "state": match &step.state {
                        State::Done => "done",
                        State::Todo => "todo",
                        State::Yours(_) => "yours",
                    },
                    "detail": step.detail,
                    "yours": match &step.state {
                        State::Yours(said) => Some(said.clone()),
                        _ => None,
                    },
                })
            })
            .collect(),
    )
}

/// The steps that are not done yet.
pub fn remaining(facts: &Facts) -> Vec<Step> {
    steps(facts).into_iter().filter(|s| !s.done()).collect()
}

fn repository(facts: &Facts) -> Step {
    let why = "Every run isolates its work in a git worktree, so without a repository \
               and one commit to branch from, nothing runs at all.";
    let Some(name) = &facts.repository else {
        return Step {
            id: Id::Repository,
            title: "a repository to work on",
            why,
            state: State::Yours(
                "Clone what you want worked on into the repositories directory. \
                 Nothing here knows the URL."
                    .to_string(),
            ),
            detail: Vec::new(),
            offer: Offer::Nothing,
        };
    };
    match &facts.repository_problem {
        None => Step {
            id: Id::Repository,
            title: "a repository to work on",
            why,
            state: State::Done,
            detail: vec![format!("{name} is a git repository with a commit")],
            offer: Offer::Nothing,
        },
        Some(problem) => Step {
            id: Id::Repository,
            title: "a repository to work on",
            why,
            state: if facts.repository_commands.is_empty() {
                State::Yours(problem.clone())
            } else {
                State::Todo
            },
            detail: vec![problem.clone()],
            offer: if facts.repository_commands.is_empty() {
                Offer::Nothing
            } else {
                Offer::Commands(facts.repository_commands.clone())
            },
        },
    }
}

fn gate(facts: &Facts) -> Step {
    let why = "The checks are what a change is judged by before any reviewer sees it. \
               A gate that declares nothing approves whatever a reviewer waves through.";
    let title = "the gate, run once";
    if !facts.declared {
        return Step {
            id: Id::Gate,
            title,
            why,
            state: State::Todo,
            detail: vec!["no configuration yet, so there are no checks".to_string()],
            offer: Offer::Nothing,
        };
    }
    if facts.placeholder_gate || facts.checks.is_empty() {
        return Step {
            id: Id::Gate,
            title,
            why,
            state: State::Yours(format!(
                "Replace the `{PLACEHOLDER_CHECK}` check in .ostraka/ostraka.toml with the \
                 commands you would want run before trusting a change you did not write."
            )),
            detail: vec!["the gate is the placeholder that fails on purpose".to_string()],
            offer: Offer::Nothing,
        };
    }
    let listed = format!("checks: {}", facts.checks.join(", "));
    // Declared is not the same as working. A gate nobody has run is a guess,
    // and a command that is wrong here refuses every run until somebody tries
    // it — so the step is done only once these checks have run and passed.
    match facts.gate_tried {
        Some(true) => Step {
            id: Id::Gate,
            title,
            why,
            state: State::Done,
            detail: vec![
                listed,
                "they passed when they were last run here".to_string(),
            ],
            offer: Offer::Nothing,
        },
        Some(false) => Step {
            id: Id::Gate,
            title,
            why,
            state: State::Todo,
            detail: vec![
                listed,
                "they did not pass when they were last run here".to_string(),
            ],
            offer: Offer::RunGate,
        },
        None => Step {
            id: Id::Gate,
            title,
            why,
            state: State::Todo,
            detail: vec![listed, "they have not been run here yet".to_string()],
            offer: Offer::RunGate,
        },
    }
}

/// Where the answer to "have these checks ever run here" is kept.
///
/// Under `.ostraka/cache/`, which `init` writes into `.gitignore`: it is a fact
/// about this machine, not about the project, and it must not reach a diff.
fn gate_record(workspace: &Workspace) -> std::path::PathBuf {
    workspace.ostraka().join("cache").join("gate-tried.json")
}

/// The checks as a line, so a gate that changed is a gate that has not been
/// run: editing a command is exactly when trying it matters again.
fn gate_shape(checks: &[ostraka_core::gate::Check]) -> String {
    checks
        .iter()
        .map(|c| format!("{}\u{1f}{}", c.name, c.cmd))
        .collect::<Vec<_>>()
        .join("\u{1e}")
}

fn remember_gate(workspace: &Workspace, shape: &str, passed: bool) {
    let path = gate_record(workspace);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        &path,
        serde_json::json!({ "checks": shape, "passed": passed }).to_string(),
    );
}

fn gate_tried(workspace: &Workspace, shape: &str) -> Option<bool> {
    let text = std::fs::read_to_string(gate_record(workspace)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    (value.get("checks")?.as_str()? == shape).then(|| value.get("passed")?.as_bool())?
}

fn pair(facts: &Facts) -> Step {
    let why = "A change is written by one agent and reviewed by another, and the review \
               is only independent if the two are different — a different vendor where \
               there is one.";
    let title = "who writes and who reviews";
    match &facts.pair {
        Some((author, reviewer)) => Step {
            id: Id::Pair,
            title,
            why,
            state: State::Done,
            detail: vec![format!("{author} writes, {reviewer} reviews")],
            offer: if facts.installed_unconfigured.is_empty() {
                Offer::Nothing
            } else {
                // More installed than configured is not a problem, but it is
                // the difference between one reviewer and a choice of them.
                Offer::WriteProfiles(facts.installed_unconfigured.clone())
            },
        },
        None if !facts.installed_unconfigured.is_empty() => Step {
            id: Id::Pair,
            title,
            why,
            state: State::Todo,
            detail: vec![format!(
                "installed here with no profile: {}",
                facts.installed_unconfigured.join(", ")
            )],
            offer: Offer::WriteProfiles(facts.installed_unconfigured.clone()),
        },
        None => Step {
            id: Id::Pair,
            title,
            why,
            state: State::Yours(
                "No two profiles here can run. Install a second coding CLI, or \
                 configure one this workspace does not have."
                    .to_string(),
            ),
            detail: Vec::new(),
            offer: Offer::Nothing,
        },
    }
}

fn auth(facts: &Facts) -> Step {
    let why = "Installed and logged out is the failure that looks like a bad change: \
               the agent never answers, and the run is refused for it.";
    let title = "can each of them actually run";
    if facts.pair.is_none() {
        return Step {
            id: Id::Auth,
            title,
            why,
            state: State::Yours("Nothing to probe until there is a pair.".to_string()),
            detail: Vec::new(),
            offer: Offer::Nothing,
        };
    }
    if facts.unready.is_empty() {
        return Step {
            id: Id::Auth,
            title,
            why,
            state: State::Done,
            detail: vec!["both answered".to_string()],
            offer: Offer::Probe,
        };
    }
    Step {
        id: Id::Auth,
        title,
        why,
        state: State::Yours(
            "Log in to the CLI, or point the profile at one that is logged in. \
             A login is not something this can take for you."
                .to_string(),
        ),
        detail: facts
            .unready
            .iter()
            .map(|(id, why)| format!("{id}: {why}"))
            .collect(),
        offer: Offer::Probe,
    }
}

fn policy(facts: &Facts) -> Step {
    let why = "The ceiling stops an agent waiting on a prompt nobody will answer, and \
               the path rule is checked against git rather than the agent's own account \
               of what it changed.";
    let title = "policy and risk paths";
    let Some((timeout, paths, enforced)) = facts.policy else {
        return Step {
            id: Id::Policy,
            title,
            why,
            state: State::Todo,
            detail: vec!["no configuration yet".to_string()],
            offer: Offer::Nothing,
        };
    };
    let mut detail = vec![match timeout {
        Some(secs) => format!("an agent is stopped after {secs}s"),
        None => "no ceiling on an agent: a stalled one waits forever".to_string(),
    }];
    detail.push(match (paths, enforced) {
        (0, _) => "no path limit".to_string(),
        (n, true) => format!("{n} allowed path(s), enforced"),
        (n, false) => format!("{n} allowed path(s), not enforced"),
    });
    Step {
        id: Id::Policy,
        title,
        why,
        // Defaults are fine, which is why this is done as soon as there is a
        // configuration. It is listed so that what they are is not something
        // to be discovered by a run hitting one.
        state: State::Done,
        detail,
        offer: Offer::Nothing,
    }
}

fn extras(facts: &Facts) -> Step {
    let missing: Vec<&str> = [("notes", facts.notes), ("task list", facts.tasks)]
        .iter()
        .filter(|(_, there)| !*there)
        .map(|(name, _)| *name)
        .collect();
    Step {
        id: Id::Extras,
        title: "extras",
        why: "Nothing depends on these: notes for what a run should know, a task list \
              for work waiting to be taken.",
        state: if missing.is_empty() {
            State::Done
        } else {
            State::Todo
        },
        detail: if missing.is_empty() {
            vec!["notes and the task list are there".to_string()]
        } else {
            vec![format!("not there yet: {}", missing.join(", "))]
        },
        offer: Offer::Nothing,
    }
}

/// Reads what is on disk. Probes, so not on any drawing path.
pub fn gather(workspace: &Workspace, project: Option<&Path>) -> Facts {
    let repositories = workspace.repositories();
    let repository = project
        .map(std::path::Path::to_path_buf)
        .or_else(|| repositories.first().map(|r| r.path.clone()));
    let remedy = repository.as_ref().and_then(|path| {
        crate::remedy::Remedy::diagnose(path).map(|remedy| {
            let commands = remedy
                .steps
                .iter()
                .flat_map(|step| step.commands.clone())
                .collect::<Vec<_>>();
            (remedy.problem, commands)
        })
    });
    // The configuration of the repository these steps are about, not of
    // whichever one sorts first: a repository may carry its own, and the gate
    // and policy reported here have to be the ones that would actually run.
    let config = repository
        .as_ref()
        .and_then(|path| repositories.iter().find(|repo| &repo.path == path))
        .or_else(|| repositories.first())
        .and_then(|repo| workspace.config_for(repo).ok());
    let configured: Vec<String> = workspace
        .profiles()
        .map(|profiles| profiles.iter().map(|p| p.id.clone()).collect())
        .unwrap_or_default();
    let pair = crate::run::would_route(workspace, None, None);
    let unready = pair
        .as_ref()
        .map(|(author, reviewer)| {
            let mut asked: Vec<String> = vec![author.clone(), reviewer.clone()];
            asked.dedup();
            asked
                .into_iter()
                .filter_map(|id| match probe(workspace, &id) {
                    Ok(()) => None,
                    Err(why) => Some((id, why)),
                })
                .collect()
        })
        .unwrap_or_default();

    Facts {
        declared: workspace.declared(),
        repository: repository
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        repository_problem: remedy.as_ref().map(|(problem, _)| problem.clone()),
        repository_commands: remedy.map(|(_, commands)| commands).unwrap_or_default(),
        checks: config
            .as_ref()
            .map(|c| c.gate.checks.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default(),
        gate_tried: config
            .as_ref()
            .and_then(|c| gate_tried(workspace, &gate_shape(&c.gate.checks))),
        placeholder_gate: config.as_ref().is_some_and(|c| {
            c.gate
                .checks
                .iter()
                .any(|check| check.name == PLACEHOLDER_CHECK)
        }),
        pair,
        installed_unconfigured: crate::discover::unconfigured(&configured)
            .into_iter()
            .filter(crate::discover::Found::ready)
            .map(|found| found.id)
            .collect(),
        unready,
        policy: config.as_ref().map(|c| {
            (
                c.policy.timeout_secs,
                c.policy.allowed_paths.len(),
                c.policy.enforce_paths,
            )
        }),
        notes: workspace.notes_if_present().is_some(),
        tasks: workspace.ostraka().join("tasks").is_dir(),
    }
}

/// Walks the steps that are not done, asking before each one.
///
/// `input` and `output` are taken rather than reached for, so a test can drive
/// the whole exchange — the only way to be sure that a walk which commits
/// somebody's directory, or spends a vendor's time, does it when they said so.
///
/// Every step can be skipped, and stopping leaves what was taken taken: the
/// steps are read off disk, so the next `init` resumes here.
pub fn walk(
    workspace: &Workspace,
    input: &mut impl std::io::BufRead,
    output: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let mut facts = gather(workspace, None);
    let left = remaining(&facts);
    if left.is_empty() {
        writeln!(output, "\nEvery step is taken: this project can run.")?;
        return Ok(());
    }
    writeln!(
        output,
        "\n{} step(s) left. Each one can be skipped.",
        left.len()
    )?;

    let repository = workspace.repositories().first().map(|r| r.path.clone());
    // Each step is taken against what is on disk *now*, not against what was
    // there when the walk started: writing the profiles that make a pair makes
    // the next step's "nothing to probe until there is a pair" untrue, and a
    // walk that said it anyway would be reporting its own first reading back.
    // Re-read only after something changed, since reading probes.
    let mut taken: Vec<Id> = Vec::new();
    while let Some(step) = remaining(&facts)
        .into_iter()
        .find(|step| !taken.contains(&step.id))
    {
        taken.push(step.id);
        let mut changed = false;
        writeln!(output, "\n{}", step.title)?;
        writeln!(output, "  {}", step.why)?;
        for said in &step.detail {
            writeln!(output, "  {said}")?;
        }
        if let State::Yours(said) = &step.state {
            writeln!(output, "\n  This one is yours: {said}")?;
            continue;
        }
        let facts_before = &facts;
        match &step.offer {
            Offer::Nothing => continue,
            Offer::Commands(commands) => {
                let Some(path) = &repository else { continue };
                let Some(remedy) = crate::remedy::Remedy::diagnose(path) else {
                    continue;
                };
                for command in commands {
                    writeln!(output, "  $ {}", command.join(" "))?;
                }
                // The remedy asks for itself, step by step, with its own
                // warnings: it is the one that commits somebody's files.
                let outcome = crate::fix::walk(path, remedy, input, output)?;
                changed = true;
                if outcome != crate::fix::Outcome::Fixed {
                    writeln!(
                        output,
                        "\nStopped here. `ostraka init` resumes from this step."
                    )?;
                    return Ok(());
                }
            }
            Offer::RunGate => {
                let Some(path) = &repository else { continue };
                if !asked(
                    input,
                    output,
                    "Run the checks now? [y] yes, anything else skips: ",
                )? {
                    continue;
                }
                run_gate(workspace, path, output)?;
                changed = true;
            }
            Offer::WriteProfiles(_) => {
                // The same offer a run makes when it cannot route, from the
                // same module: it lists them, takes numbers or `a`, and writes
                // only what was named. Writing profiles for every CLI on one
                // yes is not the question this step is asking.
                let configured: Vec<String> = workspace
                    .profiles()
                    .map(|profiles| profiles.iter().map(|p| p.id.clone()).collect())
                    .unwrap_or_default();
                let problem = crate::discover::NoAdapter {
                    said: "This workspace has no pair that can write and review.".to_string(),
                    found: crate::discover::unconfigured(&configured),
                };
                changed = matches!(
                    crate::offer::profiles(workspace, &problem, input, output, true)?,
                    crate::offer::Choice::Wrote
                );
            }
            Offer::Probe => {
                if !asked(
                    input,
                    output,
                    "Ask each of them now? [y] yes, anything else skips: ",
                )? {
                    continue;
                }
                for id in facts_before
                    .pair
                    .iter()
                    .flat_map(|(a, b)| [a.clone(), b.clone()])
                {
                    match probe(workspace, &id) {
                        Ok(()) => writeln!(output, "  {id}: answered")?,
                        Err(why) => writeln!(output, "  {id}: {why}")?,
                    }
                }
            }
        }
        if changed {
            facts = gather(workspace, None);
        }
    }
    // Counted again from disk rather than from what this walk did, so what it
    // says is what the next `init` will find.
    let left = remaining(&gather(workspace, None));
    if left.is_empty() {
        writeln!(output, "\nEvery step is taken: this project can run.")?;
    } else {
        writeln!(
            output,
            "\n{} step(s) still left. `ostraka init` picks up from there.",
            left.len()
        )?;
    }
    Ok(())
}

/// One yes-or-no question. Anything but yes is no, because the cost of
/// misreading an answer here is running something nobody asked for.
fn asked(
    input: &mut impl std::io::BufRead,
    output: &mut impl std::io::Write,
    question: &str,
) -> std::io::Result<bool> {
    write!(output, "\n{question}")?;
    output.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Runs the project's checks once, in the repository itself.
///
/// Not in a worktree: this is the question "do these commands work here", and
/// a worktree would answer it about a fresh checkout instead. Nothing is
/// recorded — a gate belongs to a run, and this is not one.
fn run_gate(
    workspace: &Workspace,
    repository: &Path,
    output: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let Some(repo) = workspace
        .repositories()
        .into_iter()
        .find(|r| r.path == repository)
    else {
        return Ok(());
    };
    let Ok(config) = workspace.config_for(&repo) else {
        return Ok(());
    };
    let mut said = Ok(());
    let outcome = ostraka_runtime::gate::run_checks(&config.gate, repository, &mut |record| {
        if said.is_ok() {
            said = writeln!(
                output,
                "  {}  {} ({}ms)",
                if record.passed() { "pass" } else { "FAIL" },
                record.name,
                record.duration_ms
            );
        }
    });
    said?;
    remember_gate(workspace, &gate_shape(&config.gate.checks), outcome.is_ok());
    match outcome {
        Ok(_) => writeln!(output, "  the gate passes here")?,
        Err(_) => writeln!(
            output,
            "  the gate does not pass here yet \u{2014} until it does, every run is refused"
        )?,
    }
    Ok(())
}

/// Asks one profile's CLI whether it can run, the way `ostraka adapters` does.
fn probe(workspace: &Workspace, id: &str) -> Result<(), String> {
    use ostraka_adapter::{Availability, VendorAdapter};
    let profiles = workspace.profiles().map_err(|e| e.to_string())?;
    let profile = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no profile {id}"))?;
    let adapter = ostraka_adapter::process::ProcessAdapter::new(profile.clone());
    match adapter.probe() {
        Availability::Ready { .. } => Ok(()),
        other => Err(crate::discover::describe(&other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything true: a repository with a commit, a real gate, a pair that
    /// answers, a configuration, notes and a task list.
    fn ready() -> Facts {
        Facts {
            declared: true,
            repository: Some("work".into()),
            repository_problem: None,
            repository_commands: Vec::new(),
            checks: vec!["test".into()],
            placeholder_gate: false,
            gate_tried: Some(true),
            pair: Some(("writer".into(), "reader".into())),
            installed_unconfigured: Vec::new(),
            unready: Vec::new(),
            policy: Some((Some(900), 0, false)),
            notes: true,
            tasks: true,
        }
    }

    fn step_of(facts: &Facts, id: Id) -> Step {
        steps(facts)
            .into_iter()
            .find(|s| s.id == id)
            .expect("every step is listed")
    }

    /// A workspace with a repository that has a commit and a gate of one
    /// check that passes.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-setup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("ostraka");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"test\", cmd = \"true\", required = true }]\n\n             [policy]\ntimeout_secs = 900\n\n[gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        let repo = dir.join("repositories/work");
        std::fs::create_dir_all(&repo).expect("repository");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .expect("git runs");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "x\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "seed"]);
        dir
    }

    /// End to end: the steps are read off a real directory, and the gate step
    /// runs the project's checks there when the answer is yes.
    #[test]
    fn the_walk_reads_a_real_directory_and_runs_its_checks_when_asked() {
        let dir = scratch("walk");
        let workspace = Workspace::at(&dir);
        let facts = gather(&workspace, None);
        assert!(step_of(&facts, Id::Repository).done(), "{facts:#?}");
        assert!(step_of(&facts, Id::Policy).done(), "{facts:#?}");

        // A declared gate is not a working one, so it is asked about until it
        // has run and passed.
        assert_eq!(step_of(&facts, Id::Gate).state, State::Todo);
        assert_eq!(step_of(&facts, Id::Gate).offer, Offer::RunGate);

        // Yes to the gate, no to everything after it.
        let mut said = Vec::new();
        walk(&workspace, &mut "y\nn\nn\nn\n".as_bytes(), &mut said).expect("walks");
        let said = String::from_utf8(said).expect("utf-8");
        assert!(said.contains("Run the checks now?"), "{said}");
        assert!(said.contains("pass  test"), "{said}");
        assert!(said.contains("the gate passes here"), "{said}");
        // Steps nobody but a person can take are said, not asked about.
        assert!(said.contains("This one is yours"), "{said}");

        // Counted from disk at both ends, not from what the walk did: the
        // gate ran during this walk, so one fewer step is left after it than
        // the walk was told about at the start.
        let before: usize = said
            .split(" step(s) left")
            .next()
            .and_then(|s| s.rsplit('\n').next())
            .and_then(|s| s.trim().parse().ok())
            .expect("a count to start with");
        let after: usize = said
            .split(" step(s) still left")
            .next()
            .and_then(|s| s.rsplit('\n').next())
            .and_then(|s| s.trim().parse().ok())
            .expect("a count to end with");
        assert_eq!(
            after,
            before - 1,
            "the gate still counts as a step:\n{said}"
        );

        // And it is remembered, so the next init does not ask again: that is
        // what "init resumes where it stopped" means for a step that runs
        // something.
        assert_eq!(gather(&workspace, None).gate_tried, Some(true));
        assert!(step_of(&gather(&workspace, None), Id::Gate).done());

        // Editing a check is exactly when trying it matters again.
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"test\", cmd = \"true --now\", required = true }]\n\n             [gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        assert_eq!(gather(&workspace, None).gate_tried, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gate whose command does not work here is the thing this step exists
    /// to find: every run would be refused by it until somebody noticed.
    #[test]
    fn a_gate_that_does_not_pass_says_so_rather_than_looking_taken() {
        let dir = scratch("badgate");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"test\", cmd = \"exit 3\", required = true }]\n\n             [gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        let workspace = Workspace::at(&dir);
        let mut said = Vec::new();
        walk(&workspace, &mut "y\nn\nn\nn\n".as_bytes(), &mut said).expect("walks");
        let said = String::from_utf8(said).expect("utf-8");
        assert!(said.contains("FAIL  test"), "{said}");
        assert!(said.contains("every run is refused"), "{said}");
        // Remembered as not passing, so it stays a step rather than looking
        // taken because somebody once ran it.
        let facts = gather(&workspace, None);
        assert_eq!(facts.gate_tried, Some(false));
        assert_eq!(step_of(&facts, Id::Gate).state, State::Todo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Raised in review: a repository that carries its own configuration is
    /// judged by that one, not by whichever repository sorts first.
    #[test]
    fn the_gate_reported_is_the_one_that_would_run() {
        let dir = scratch("ownconfig");
        // A second repository, sorting before "work", with a gate of its own.
        let other = dir.join("repositories/apart");
        std::fs::create_dir_all(&other).expect("repository");
        std::fs::write(
            // A repository's own configuration sits at its root.
            other.join("ostraka.toml"),
            "[gate]\nchecks = [{ name = \"theirs\", cmd = \"true\", required = true }]\n\n             [gate.review]\nmust_differ_from_author = true\n",
        )
        .expect("config");
        let workspace = Workspace::at(&dir);

        // Asked about that repository, the steps report its checks.
        let facts = gather(&workspace, Some(&other));
        assert_eq!(facts.checks, ["theirs"]);
        // Asked about the other one, the workspace's.
        let facts = gather(&workspace, Some(&dir.join("repositories/work")));
        assert_eq!(facts.checks, ["test"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Raised in review: a script reading `--json` sees the steps too.
    #[test]
    fn the_steps_are_readable_as_json() {
        let json = as_json(&steps(&Facts::default()));
        let rows = json.as_array().expect("an array");
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0]["step"], "repository");
        assert_eq!(rows[0]["state"], "yours");
        assert!(
            rows[0]["yours"]
                .as_str()
                .is_some_and(|s| s.contains("Clone"))
        );
        assert_eq!(rows[1]["step"], "gate");
        assert_eq!(rows[1]["state"], "todo");
        assert_eq!(as_json(&steps(&ready()))[0]["state"], "done");
    }

    #[test]
    fn the_steps_are_in_the_order_of_what_costs_most_when_it_is_wrong() {
        let ids: Vec<Id> = steps(&ready()).iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                Id::Repository,
                Id::Gate,
                Id::Pair,
                Id::Auth,
                Id::Policy,
                Id::Extras
            ]
        );
        assert!(steps(&ready()).iter().all(|s| !s.why.is_empty()));
        assert!(
            remaining(&ready()).is_empty(),
            "a ready workspace has steps left"
        );
    }

    /// A fresh directory: nothing is done, and every step says why it matters.
    #[test]
    fn nothing_configured_leaves_every_step_to_take() {
        let steps = steps(&Facts::default());
        assert!(steps.iter().all(|s| !s.done()), "{steps:#?}");
        assert!(matches!(
            step_of(&Facts::default(), Id::Repository).state,
            State::Yours(_)
        ));
    }

    /// The first step is `remedy.rs`'s, not a second copy of it.
    #[test]
    fn a_directory_that_is_not_a_repository_offers_the_remedy_commands() {
        let facts = Facts {
            repository_problem: Some("not a git repository".into()),
            repository_commands: vec![vec!["git".into(), "init".into()]],
            ..ready()
        };
        let step = step_of(&facts, Id::Repository);
        assert_eq!(step.state, State::Todo);
        assert_eq!(
            step.offer,
            Offer::Commands(vec![vec!["git".into(), "init".into()]])
        );

        // A problem with no command out of it is a person's to take.
        let manual = Facts {
            repository_commands: Vec::new(),
            ..facts
        };
        assert!(matches!(
            step_of(&manual, Id::Repository).state,
            State::Yours(_)
        ));
    }

    /// The gate written for a project nobody could identify fails on purpose,
    /// and a workspace still carrying it has not had its gate written.
    #[test]
    fn the_placeholder_gate_is_not_a_gate() {
        let facts = Facts {
            checks: vec![PLACEHOLDER_CHECK.into()],
            placeholder_gate: true,
            ..ready()
        };
        let step = step_of(&facts, Id::Gate);
        assert!(matches!(step.state, State::Yours(_)));
        assert_eq!(step.offer, Offer::Nothing);

        // A gate that has run and passed is done, and asks nothing further.
        let step = step_of(&ready(), Id::Gate);
        assert!(step.done());
        assert_eq!(step.offer, Offer::Nothing);
        assert_eq!(step.detail[0], "checks: test");
    }

    /// Declared, run, and passing are three different things.
    #[test]
    fn a_gate_is_done_only_once_it_has_run_and_passed() {
        let untried = Facts {
            gate_tried: None,
            ..ready()
        };
        assert_eq!(step_of(&untried, Id::Gate).state, State::Todo);
        assert!(step_of(&untried, Id::Gate).detail[1].contains("not been run"));

        let failed = Facts {
            gate_tried: Some(false),
            ..ready()
        };
        assert_eq!(step_of(&failed, Id::Gate).state, State::Todo);
        assert!(step_of(&failed, Id::Gate).detail[1].contains("did not pass"));

        assert!(step_of(&ready(), Id::Gate).done());
        assert_eq!(step_of(&ready(), Id::Gate).offer, Offer::Nothing);
    }

    #[test]
    fn no_pair_offers_the_clis_installed_here_and_nothing_to_probe() {
        let facts = Facts {
            pair: None,
            installed_unconfigured: vec!["codex".into()],
            ..ready()
        };
        let pair = step_of(&facts, Id::Pair);
        assert_eq!(pair.state, State::Todo);
        assert_eq!(pair.offer, Offer::WriteProfiles(vec!["codex".into()]));
        assert!(matches!(step_of(&facts, Id::Auth).state, State::Yours(_)));

        // Nothing installed to offer: only a person can install one.
        let bare = Facts {
            installed_unconfigured: Vec::new(),
            ..facts
        };
        assert!(matches!(step_of(&bare, Id::Pair).state, State::Yours(_)));
    }

    /// A pair that cannot answer is the failure that looks like a bad change.
    #[test]
    fn a_cli_that_cannot_run_is_named_with_what_it_said() {
        let facts = Facts {
            unready: vec![("reader".into(), "not logged in".into())],
            ..ready()
        };
        let step = step_of(&facts, Id::Auth);
        assert!(matches!(step.state, State::Yours(_)));
        assert_eq!(step.detail, ["reader: not logged in"]);
    }

    #[test]
    fn the_policy_step_says_what_the_defaults_are() {
        let step = step_of(&ready(), Id::Policy);
        assert!(step.done());
        assert_eq!(step.detail[0], "an agent is stopped after 900s");
        assert_eq!(step.detail[1], "no path limit");

        let strict = Facts {
            policy: Some((None, 2, true)),
            ..ready()
        };
        let step = step_of(&strict, Id::Policy);
        assert_eq!(
            step.detail[0],
            "no ceiling on an agent: a stalled one waits forever"
        );
        assert_eq!(step.detail[1], "2 allowed path(s), enforced");
    }

    #[test]
    fn extras_name_what_is_missing_and_are_never_in_the_way() {
        let facts = Facts {
            notes: false,
            tasks: true,
            ..ready()
        };
        let step = step_of(&facts, Id::Extras);
        assert_eq!(step.state, State::Todo);
        assert_eq!(step.detail, ["not there yet: notes"]);
        assert!(step_of(&ready(), Id::Extras).done());
    }
}
