//! What a task is for: a question, a plan, a change that keeps trying, or a
//! change.
//!
//! Four ways into one pipeline, and none of them is a way around it. Asking and
//! planning never produce a change, so there is nothing to gate: the agent is
//! invoked read-only, and a worktree that changes anyway is refused and kept as
//! evidence. Looping and auto both produce changes, and every attempt is gated
//! and reviewed on its own. None of them merges. Promotion stays a person's
//! act.

/// What pressing enter does with a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Mode {
    /// Answered from the repository, read-only. Nothing is written.
    Ask,
    /// A plan written read-only, and run only once somebody agrees to it.
    Plan,
    /// A run that is tried again, with the reason, when it is refused.
    Loop,
    /// A run: written, gated, reviewed, and never merged.
    #[default]
    Auto,
}

impl Mode {
    /// In the order shift-tab walks them.
    pub const ALL: [Mode; 4] = [Mode::Auto, Mode::Ask, Mode::Plan, Mode::Loop];

    pub fn word(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Plan => "plan",
            Mode::Loop => "loop",
            Mode::Auto => "auto",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Mode::Ask => "a question, answered read-only; nothing is written",
            Mode::Plan => "a plan, written read-only and run once you agree to it",
            Mode::Loop => "a run tried again with the reason, until it is approved",
            Mode::Auto => "a run, gated and reviewed, and never merged",
        }
    }

    /// The next mode, wrapping.
    pub fn next(self) -> Mode {
        let at = Mode::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Mode::ALL[(at + 1) % Mode::ALL.len()]
    }

    /// The mode a typed word names, with or without its slash.
    pub fn named(typed: &str) -> Option<Mode> {
        let word = typed.trim().trim_start_matches('/').to_lowercase();
        Mode::ALL.into_iter().find(|mode| mode.word() == word)
    }

    /// True for the modes that consult an agent rather than change anything.
    pub fn consults(self) -> bool {
        matches!(self, Mode::Ask | Mode::Plan)
    }

    /// The instruction an agent is consulted with.
    ///
    /// The read-only invocation is the guarantee, and the worktree check after
    /// it is the proof. This sentence is neither. It is here so that an agent
    /// that could write does not waste the turn trying to.
    pub fn consulting(self, task: &str) -> String {
        match self {
            Mode::Plan => format!(
                "{task}\n\n----- a plan, not the change -----\n\n\
                 Do not make this change yet. Read what you need in this repository \
                 and write the plan you would follow: the files involved, the steps \
                 in order, and how the result should be checked. Do not create, edit \
                 or delete any file. The plan is the whole of your answer."
            ),
            _ => format!(
                "{task}\n\n----- a question, not a change -----\n\n\
                 Answer from what is in this repository. Do not create, edit or \
                 delete any file."
            ),
        }
    }

    /// The task a run is given once its plan has been agreed.
    pub fn planned(task: &str, plan: &str) -> String {
        format!(
            "{task}\n\n----- the agreed plan -----\n\n\
             This plan was written for this task and agreed before you started. \
             Follow it. Where the code shows a step to be wrong, do what the task \
             needs instead.\n\n{}",
            plan.trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_is_reached_by_walking_and_by_name() {
        let mut seen = vec![Mode::default()];
        let mut mode = Mode::default();
        for _ in 1..Mode::ALL.len() {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(mode.next(), Mode::default(), "walking did not wrap");
        for mode in Mode::ALL {
            assert!(seen.contains(&mode), "{mode:?} is never reached");
            assert_eq!(Mode::named(mode.word()), Some(mode));
            assert_eq!(Mode::named(&format!("/{}", mode.word())), Some(mode));
        }
        assert_eq!(Mode::named("/ASK"), Some(Mode::Ask));
        assert_eq!(Mode::named("/settings"), None);
    }

    #[test]
    fn only_asking_and_planning_consult() {
        assert!(Mode::Ask.consults() && Mode::Plan.consults());
        assert!(!Mode::Loop.consults() && !Mode::Auto.consults());
    }

    #[test]
    fn a_planned_task_carries_the_task_and_the_plan() {
        let said = Mode::planned("add a flag", "  1. edit main.rs\n");
        assert!(said.starts_with("add a flag"));
        assert!(said.ends_with("1. edit main.rs"));
        assert!(
            Mode::Plan
                .consulting("add a flag")
                .contains("Do not create")
        );
    }
}
