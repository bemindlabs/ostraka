//! The agents beside the work: every profile this workspace has, whether it
//! can run here, what it is doing now, and whether it is the one a run would
//! use next.
//!
//! This is the mapping, kept out of the view so that drawing stays a function
//! of state and nothing about it has to be worked out while drawing. Each fact
//! comes from something that already answers it:
//!
//! - which profiles there are: `.ostraka/adapters`, as `Workspace::profiles`
//!   reads it;
//! - whether one can run here: the probe `ostraka adapters` uses, taken once on
//!   a thread and held, never repeated per frame;
//! - what one is doing: `index::live`, which reads the `live.json` the runtime
//!   writes beside every run in progress — so a `drain` worker in another
//!   process shows up exactly as a pane in this one does;
//! - which pair a run would use: `run::would_route`, the same choice `execute`
//!   makes, taken on a thread.
//!
//! A profile is only ever doing what the runtime says it is doing. Between the
//! agents — making the worktree, running the checks — no profile is running,
//! and the row says idle rather than claiming work that is not happening.

use super::view::Agent;
use ostraka_runtime::progress::Phase;
use ostraka_runtime::record::Live;

/// One thing a profile is doing now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// Writing the change for this run.
    Writing(String),
    /// Reviewing this run's change.
    Reviewing(String),
}

/// Whether a profile can run on this machine, as its probe said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Not asked yet. Probing runs every CLI, so it happens on a thread and the
    /// row says so until it answers rather than guessing.
    Asking,
    /// It answered. The note is what the probe reported, a version where the
    /// CLI gave one.
    Ready(String),
    /// It did not: not installed, or it would not start. The note says which.
    Missing(String),
}

/// One profile, as the side pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub probe: Probe,
    /// Everything it is doing, in every pane and every worker. Empty is idle.
    /// A profile can be in several runs at once.
    pub doing: Vec<Doing>,
    /// The profile a run started now would write with.
    pub writes_next: bool,
    /// The profile a run started now would be reviewed by.
    pub reviews_next: bool,
}

/// The rows, one per configured profile, in id order.
///
/// `profiles` is what the workspace has; `probes` is what the probe answered,
/// which may be nothing yet; `live` is every run in progress; `next` is the
/// pair a run started now would use, where one could be made.
pub fn rows(
    profiles: &[String],
    probes: &[Agent],
    live: &[(String, Live)],
    next: Option<&(String, String)>,
) -> Vec<Row> {
    let mut ids = profiles.to_vec();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let probe = match probes.iter().find(|a| a.configured && a.id == id) {
                None => Probe::Asking,
                Some(agent) if agent.ready => Probe::Ready(agent.note.clone()),
                Some(agent) => Probe::Missing(agent.note.clone()),
            };
            let doing = live
                .iter()
                .filter_map(|(run, live)| match live.phase {
                    Phase::Authoring if live.author == id => Some(Doing::Writing(run.clone())),
                    Phase::Reviewing if live.reviewer == id => Some(Doing::Reviewing(run.clone())),
                    _ => None,
                })
                .collect();
            Row {
                writes_next: next.is_some_and(|(author, _)| *author == id),
                reviews_next: next.is_some_and(|(_, reviewer)| *reviewer == id),
                id,
                probe,
                doing,
            }
        })
        .collect()
}

/// Where a task on the list stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    /// Claimed and being run, with the phase its run is in where its run can
    /// be found. `None` is a task claimed by a run that has not reported yet.
    Going(Option<Phase>),
    /// On the list, not yet taken.
    Waiting,
    /// Taken and finished, with the outcome its run recorded.
    Done(Option<String>),
}

/// One task, as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: String,
    pub prompt: String,
    pub state: TaskState,
    /// Waiting on a person's decision. Nothing sets it yet: the decisions
    /// queue will, and the row already has the room for its mark.
    pub needs_person: bool,
}

/// One repository's tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGroup {
    pub repository: String,
    pub waiting: usize,
    pub going: usize,
    pub done: usize,
    /// Going, then waiting in the order they will be taken, then the most
    /// recently finished few.
    pub rows: Vec<TaskRow>,
}

/// How many finished tasks a group shows. The rest are counted, not listed:
/// the list is for what is happening, and a long tail of done work would push
/// it off the pane.
pub const DONE_SHOWN: usize = 3;

/// The group for tasks that name no repository in a workspace with several.
pub const NO_REPOSITORY: &str = "(no repository)";

/// The task list, grouped by repository.
///
/// A task that names no repository belongs to the only one where the workspace
/// has exactly one — which is where a run would put it — and otherwise to its
/// own group, rather than to a repository it may not be about. A going task
/// shows its run's phase where its run can be found in `live`: a run taking a
/// queued task has an id that begins with the task's.
pub fn task_groups(
    pending: &[crate::tasks::Task],
    running: &[crate::tasks::Task],
    done: &[crate::tasks::Task],
    repositories: &[String],
    live: &[(String, Live)],
) -> Vec<TaskGroup> {
    let home = |task: &crate::tasks::Task| -> String {
        match (&task.repository, repositories) {
            (Some(named), _) => named.clone(),
            (None, [only]) => only.clone(),
            (None, _) => NO_REPOSITORY.to_string(),
        }
    };
    let mut names: Vec<String> = pending
        .iter()
        .chain(running)
        .chain(done)
        .map(home)
        .collect();
    names.sort();
    names.dedup();
    // The group of unplaced tasks last: it is the odd one out, not a repository.
    names.sort_by_key(|name| name == NO_REPOSITORY);

    names
        .into_iter()
        .map(|repository| {
            let mine = |tasks: &[crate::tasks::Task]| -> Vec<crate::tasks::Task> {
                tasks
                    .iter()
                    .filter(|t| home(t) == repository)
                    .cloned()
                    .collect()
            };
            let (mut going, mut waiting, mut finished) = (mine(running), mine(pending), mine(done));
            going.sort_by(|a, b| a.id.cmp(&b.id));
            // Oldest first: the order `run --next` and `drain` take them in.
            waiting.sort_by(|a, b| a.id.cmp(&b.id));
            // Most recently finished first, read off the run id's time.
            let finished_at = |t: &crate::tasks::Task| {
                t.run_id
                    .as_deref()
                    .and_then(|r| r.rsplit_once('-'))
                    .map(|(_, at)| at.to_string())
                    .unwrap_or_else(|| t.added_at.clone())
            };
            finished.sort_by_key(|t| std::cmp::Reverse(finished_at(t)));

            let phase_of = |task: &crate::tasks::Task| {
                let prefix = format!("{}-", task.id);
                live.iter()
                    .find(|(run, _)| run.starts_with(&prefix))
                    .map(|(_, live)| live.phase)
            };
            let row = |task: &crate::tasks::Task, state| TaskRow {
                id: task.id.clone(),
                prompt: task.prompt.clone(),
                state,
                needs_person: false,
            };
            let rows = going
                .iter()
                .map(|t| row(t, TaskState::Going(phase_of(t))))
                .chain(waiting.iter().map(|t| row(t, TaskState::Waiting)))
                .chain(
                    finished
                        .iter()
                        .take(DONE_SHOWN)
                        .map(|t| row(t, TaskState::Done(t.outcome.clone()))),
                )
                .collect();
            TaskGroup {
                waiting: waiting.len(),
                going: going.len(),
                done: finished.len(),
                repository,
                rows,
            }
        })
        .collect()
}

/// One line of the task section, before it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskLine {
    /// A repository's header. `open` is whether its tasks are under it on
    /// this draw, which is not only whether somebody closed it: a short pane
    /// closes groups too.
    Header {
        repository: String,
        waiting: usize,
        going: usize,
        done: usize,
        open: bool,
    },
    Task(TaskRow),
    /// Tasks of the group above that did not fit.
    More(usize),
}

/// The task section in `room` lines, headers first.
///
/// Every header is placed before any task, so a short pane loses tasks and
/// keeps the repositories and their counts; the room left goes to the open
/// groups in order. A group cut short says how many it did not show, rather
/// than ending as if it had no more.
pub fn task_lines(
    groups: &[TaskGroup],
    closed: &std::collections::HashSet<String>,
    room: usize,
) -> Vec<TaskLine> {
    let header = |group: &TaskGroup, open| TaskLine::Header {
        repository: group.repository.clone(),
        waiting: group.waiting,
        going: group.going,
        done: group.done,
        open,
    };
    if groups.len() > room {
        // Not even the headers fit: as many as do, and a count for the rest.
        let shown = room.saturating_sub(1);
        let mut lines: Vec<TaskLine> = groups[..shown].iter().map(|g| header(g, false)).collect();
        if room > 0 {
            lines.push(TaskLine::More(groups.len() - shown));
        }
        return lines;
    }
    let mut spare = room - groups.len();
    let mut lines = Vec::new();
    for group in groups {
        let wanted = if closed.contains(&group.repository) {
            0
        } else {
            group.rows.len()
        };
        let (shown, more) = if wanted <= spare {
            (wanted, false)
        } else if spare >= 2 {
            (spare - 1, true)
        } else {
            (0, false)
        };
        lines.push(header(group, shown > 0));
        lines.extend(group.rows[..shown].iter().cloned().map(TaskLine::Task));
        if more {
            lines.push(TaskLine::More(wanted - shown));
        }
        spare -= shown + usize::from(more);
    }
    lines
}

/// The part of a task id a narrow pane has room for: the time it was added,
/// with the counter only where it is not the first of its second.
/// `k20260920T142501Z-0` is `142501`; `ostraka task list` has the whole id.
pub fn short_id(id: &str) -> String {
    let Some((_, time)) = id.split_once('T') else {
        return id.to_string();
    };
    let time = time.replacen('Z', "", 1);
    time.strip_suffix("-0").unwrap_or(&time).to_string()
}

/// The narrowest terminal the pane appears on.
///
/// Below it, the width is the work's: a transcript is what this screen is for,
/// and one squeezed to make room for a list of profiles is one nobody can read.
/// Chosen so the work keeps at least the seventy-two columns below which its
/// detail already moves behind Enter, with the pane and its gap beside it.
pub const SHOWN_FROM: u16 = 100;

/// How wide the pane is when it is shown.
pub const WIDTH: u16 = 26;

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, ready: bool, note: &str) -> Agent {
        Agent {
            id: id.to_string(),
            ready,
            note: note.to_string(),
            configured: true,
        }
    }

    fn live(author: &str, reviewer: &str, phase: Phase) -> Live {
        Live {
            author: author.to_string(),
            reviewer: reviewer.to_string(),
            phase,
        }
    }

    #[test]
    fn every_configured_profile_is_a_row_whatever_it_is_doing() {
        let rows = rows(&["codex".into(), "agy".into()], &[], &[], None);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["agy", "codex"]);
        // Not probed yet is said, not guessed.
        assert!(rows.iter().all(|r| r.probe == Probe::Asking));
        assert!(rows.iter().all(|r| r.doing.is_empty()));
    }

    #[test]
    fn a_probe_answers_ready_or_missing_with_what_it_said() {
        let probes = [
            agent("codex", true, "codex 0.41.0"),
            agent("agy", false, "not installed"),
        ];
        let rows = rows(&["agy".into(), "codex".into()], &probes, &[], None);
        assert_eq!(rows[0].probe, Probe::Missing("not installed".into()));
        assert_eq!(rows[1].probe, Probe::Ready("codex 0.41.0".into()));
    }

    /// An installed CLI with no profile here is offered elsewhere, and is not
    /// part of this workspace's agents.
    #[test]
    fn a_cli_with_no_profile_here_is_not_a_row() {
        let mut stray = agent("grok", true, "grok 1.0");
        stray.configured = false;
        let rows = rows(&["codex".into()], &[stray], &[], None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].probe, Probe::Asking);
    }

    /// Doing is read from the phase. Writing while the author runs, reviewing
    /// while the reviewer runs, and idle in between, when no agent is.
    #[test]
    fn what_a_profile_is_doing_follows_the_phase_of_each_run() {
        let profiles = ["writer".into(), "reader".into()];
        let at = |phase| {
            rows(
                &profiles,
                &[],
                &[("t1".into(), live("writer", "reader", phase))],
                None,
            )
        };

        let authoring = at(Phase::Authoring);
        let writer = authoring.iter().find(|r| r.id == "writer").expect("writer");
        let reader = authoring.iter().find(|r| r.id == "reader").expect("reader");
        assert_eq!(writer.doing, [Doing::Writing("t1".into())]);
        assert!(reader.doing.is_empty(), "the reviewer is not reviewing yet");

        let reviewing = at(Phase::Reviewing);
        let writer = reviewing.iter().find(|r| r.id == "writer").expect("writer");
        let reader = reviewing.iter().find(|r| r.id == "reader").expect("reader");
        assert!(writer.doing.is_empty(), "the author has finished");
        assert_eq!(reader.doing, [Doing::Reviewing("t1".into())]);

        for phase in [Phase::Isolating, Phase::Preparing, Phase::Gating] {
            assert!(
                at(phase).iter().all(|r| r.doing.is_empty()),
                "{phase:?}: no agent runs in this phase"
            );
        }
    }

    /// Runs in every pane and every worker, a profile in several of them at
    /// once, and each one named.
    #[test]
    fn a_profile_can_be_in_several_runs_at_once() {
        let runs = [
            (
                "t1".to_string(),
                live("codex", "claude-code", Phase::Authoring),
            ),
            (
                "t2".to_string(),
                live("claude-code", "codex", Phase::Reviewing),
            ),
            (
                "t3".to_string(),
                live("codex", "claude-code", Phase::Authoring),
            ),
        ];
        let rows = rows(&["claude-code".into(), "codex".into()], &[], &runs, None);
        let codex = rows.iter().find(|r| r.id == "codex").expect("codex");
        assert_eq!(
            codex.doing,
            [
                Doing::Writing("t1".into()),
                Doing::Reviewing("t2".into()),
                Doing::Writing("t3".into()),
            ]
        );
        let claude = rows
            .iter()
            .find(|r| r.id == "claude-code")
            .expect("claude-code");
        assert!(claude.doing.is_empty(), "{claude:?}");
    }

    fn task(id: &str, repository: Option<&str>) -> crate::tasks::Task {
        crate::tasks::Task {
            id: id.to_string(),
            prompt: format!("do {id}"),
            added_at: id.to_string(),
            repository: repository.map(str::to_string),
            adapter: None,
            run_id: None,
            outcome: None,
        }
    }

    #[test]
    fn tasks_are_grouped_by_repository_with_counts() {
        let pending = [task("k1", Some("web")), task("k2", Some("api"))];
        let running = [task("k3", Some("web"))];
        let done = [task("k4", Some("web"))];
        let groups = task_groups(
            &pending,
            &running,
            &done,
            &["api".into(), "web".into()],
            &[],
        );
        let shape: Vec<(&str, usize, usize, usize)> = groups
            .iter()
            .map(|g| (g.repository.as_str(), g.waiting, g.going, g.done))
            .collect();
        assert_eq!(shape, [("api", 1, 0, 0), ("web", 1, 1, 1)]);
    }

    /// With one repository, a task that names none is that repository's, as a
    /// run would make it. With several it is nobody's, and says so.
    #[test]
    fn a_task_with_no_repository_joins_the_only_one_or_its_own_group() {
        let unplaced = [task("k1", None)];
        let one = task_groups(&unplaced, &[], &[], &["site".into()], &[]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].repository, "site");

        let several = task_groups(
            &[task("k1", None), task("k2", Some("api"))],
            &[],
            &[],
            &["api".into(), "web".into()],
            &[],
        );
        let names: Vec<&str> = several.iter().map(|g| g.repository.as_str()).collect();
        assert_eq!(
            names,
            ["api", NO_REPOSITORY],
            "the unplaced group comes last"
        );
    }

    /// Going first, then waiting oldest first, then the few most recently
    /// finished — and a going task shows its run's phase where it can be found.
    #[test]
    fn a_group_lists_going_then_waiting_then_the_last_few_done() {
        let pending = [task("k5", Some("r")), task("k2", Some("r"))];
        let running = [task("k3", Some("r"))];
        let mut done = Vec::new();
        for n in 0..5 {
            let mut t = task(&format!("d{n}"), Some("r"));
            t.run_id = Some(format!("d{n}-2026092{n}T000000Z"));
            t.outcome = Some("approved".into());
            done.push(t);
        }
        let live_runs = [(
            "k3-20260920T120000Z".to_string(),
            live("writer", "reader", Phase::Gating),
        )];
        let groups = task_groups(&pending, &running, &done, &["r".into()], &live_runs);
        let rows = &groups[0].rows;
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["k3", "k2", "k5", "d4", "d3", "d2"]);
        assert_eq!(rows[0].state, TaskState::Going(Some(Phase::Gating)));
        assert_eq!(rows[1].state, TaskState::Waiting);
        assert_eq!(rows[3].state, TaskState::Done(Some("approved".into())));
        assert_eq!(groups[0].done, 5, "all five are counted, three are listed");
        assert!(rows.iter().all(|r| !r.needs_person));
    }

    fn group(repository: &str, tasks: usize) -> TaskGroup {
        TaskGroup {
            repository: repository.to_string(),
            waiting: tasks,
            going: 0,
            done: 0,
            rows: (0..tasks)
                .map(|n| TaskRow {
                    id: format!("{repository}{n}"),
                    prompt: String::new(),
                    state: TaskState::Waiting,
                    needs_person: false,
                })
                .collect(),
        }
    }

    fn shape(lines: &[TaskLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                TaskLine::Header {
                    repository, open, ..
                } => {
                    format!("{}{repository}", if *open { "v" } else { ">" })
                }
                TaskLine::Task(row) => row.id.clone(),
                TaskLine::More(n) => format!("+{n}"),
            })
            .collect()
    }

    #[test]
    fn the_task_section_shows_everything_where_it_fits() {
        let groups = [group("a", 2), group("b", 1)];
        let lines = task_lines(&groups, &Default::default(), 10);
        assert_eq!(shape(&lines), ["va", "a0", "a1", "vb", "b0"]);
    }

    /// Short of room, tasks go before headers do: every repository keeps its
    /// header and counts, and a group cut short says how many it hid.
    #[test]
    fn a_short_task_section_collapses_to_its_headers_first() {
        let groups = [group("a", 4), group("b", 2)];
        assert_eq!(
            shape(&task_lines(&groups, &Default::default(), 5)),
            ["va", "a0", "a1", "+2", ">b"]
        );
        assert_eq!(
            shape(&task_lines(&groups, &Default::default(), 2)),
            [">a", ">b"]
        );
        assert_eq!(shape(&task_lines(&groups, &Default::default(), 1)), ["+2"]);
        assert!(task_lines(&groups, &Default::default(), 0).is_empty());
    }

    #[test]
    fn a_closed_group_shows_only_its_header_and_gives_its_room_away() {
        let groups = [group("a", 2), group("b", 2)];
        let closed = std::iter::once("a".to_string()).collect();
        assert_eq!(
            shape(&task_lines(&groups, &closed, 4)),
            [">a", "vb", "b0", "b1"]
        );
    }

    #[test]
    fn a_task_id_is_shortened_to_its_time() {
        assert_eq!(short_id("k20260920T142501Z-0"), "142501");
        assert_eq!(short_id("k20260920T142501Z-3"), "142501-3");
        assert_eq!(short_id("custom"), "custom");
    }

    #[test]
    fn the_pair_a_run_would_use_is_marked_and_nothing_else_is() {
        let next = ("codex".to_string(), "claude-code".to_string());
        let marked = rows(
            &["agy".into(), "claude-code".into(), "codex".into()],
            &[],
            &[],
            Some(&next),
        );
        let marks: Vec<(&str, bool, bool)> = marked
            .iter()
            .map(|r| (r.id.as_str(), r.writes_next, r.reviews_next))
            .collect();
        assert_eq!(
            marks,
            [
                ("agy", false, false),
                ("claude-code", false, true),
                ("codex", true, false),
            ]
        );
        // Where no pair could be made, nothing is marked.
        assert!(
            rows(&["codex".into()], &[], &[], None)
                .iter()
                .all(|r| !r.writes_next && !r.reviews_next)
        );
    }
}
