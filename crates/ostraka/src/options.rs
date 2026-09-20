//! The alternatives a plan offers, and the one somebody chose.
//!
//! A plan used to come back as prose with one question attached: run it or
//! not. Where there was more than one sensible way to do something, the
//! planner picked one on its own and the operator agreed to a decision nobody
//! put to them. The alternatives existed — in the planner's reasoning — and
//! were thrown away before anyone saw them.
//!
//! So the plan prompt asks for them, each on a line carrying a marker minted
//! for that consultation. The marker is the mechanism the verdict uses, for
//! the reason `Verdict::parse` sets out: a marked line is accepted anywhere in
//! the answer, which is only safe when the text being read cannot have
//! contained the marker. A plan quoting a file that happens to say
//! `OPTION B: …` offers nothing.
//!
//! No options is not a failure. It is a plan, and the question is the one it
//! has always been.

/// One alternative: what it is, and what it costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    /// What the planner labelled it — `A`, `2`, `rewrite`. Kept as written,
    /// because it is how the plan's prose refers to it.
    pub label: String,
    /// The one-line summary on the marked line.
    pub title: String,
    /// The lines under it, until the next option: the trade-off, in the
    /// planner's words.
    pub detail: Vec<String>,
}

impl Alternative {
    /// The option as a person reads it in a list.
    pub fn line(&self) -> String {
        format!("{}: {}", self.label, self.title)
    }
}

/// A marker for one consultation, which nothing it reads can already contain.
///
/// Minted like `review::verdict_marker`, and for the same reason: the value
/// makes "a line beginning with this" a safe rule to apply anywhere in an
/// answer.
pub fn marker(id: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    format!("OPTION-{:016x}:", hasher.finish())
}

/// What a plan prompt says about alternatives, given the marker to use.
///
/// Asked for after the plan rather than instead of it: the plan is still the
/// answer, and a planner with nothing to choose between should write none of
/// these and lose nothing by it.
pub fn asked_for(marker: &str) -> String {
    format!(
        "If there is more than one sensible way to do this, end your answer \
         with the alternatives — at most four, each beginning with a line of \
         exactly this shape:\n\n\
         {marker} <label>: <one line saying what this option is>\n\n\
         and then, on the lines under it, what it costs and what it buys \
         compared with the others. Write nothing after the last option. If \
         there is only one sensible way, write no such lines at all: a choice \
         between one thing is not a choice, and inventing alternatives to fill \
         this in is worse than having none."
    )
}

/// The options a plan offered, in the order the planner wrote them.
///
/// Everything before the first marked line is the plan and is left alone. A
/// marked line with no label, or no title, is not an option: the planner was
/// asked for both, and guessing which half is missing would put words in its
/// mouth.
pub fn parse(answer: &str, marker: &str) -> Vec<Alternative> {
    let mut found: Vec<Alternative> = Vec::new();
    for line in answer.lines() {
        let trimmed = line.trim();
        match trimmed.strip_prefix(marker) {
            Some(rest) => {
                let Some((label, title)) = rest.split_once(':') else {
                    continue;
                };
                let (label, title) = (label.trim(), title.trim());
                if label.is_empty() || title.is_empty() {
                    continue;
                }
                found.push(Alternative {
                    label: label.to_string(),
                    title: title.to_string(),
                    detail: Vec::new(),
                });
            }
            // Prose under an option is its trade-off; prose before the first
            // one is the plan.
            None => {
                if let Some(last) = found.last_mut()
                    && !trimmed.is_empty()
                {
                    last.detail.push(trimmed.to_string());
                }
            }
        }
    }
    // One option is not a choice, and offering it as one would ask somebody to
    // decide something that has already been decided.
    if found.len() < 2 {
        return Vec::new();
    }
    found
}

/// The plan without the option lines: what a run is told to follow.
pub fn plan_only(answer: &str, marker: &str) -> String {
    answer
        .lines()
        .take_while(|line| !line.trim().starts_with(marker))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The task a run is given once an option has been chosen.
///
/// The whole plan, then which alternative was taken. The record keeps the
/// prompt, so the choice is in the run's own account of itself rather than
/// only in somebody's memory of the dialog.
pub fn chosen(task: &str, plan: &str, option: &Alternative) -> String {
    let mut said = format!(
        "{}\n\n----- the option that was chosen -----\n\n\
         Of the alternatives in the plan, this one was chosen: {}",
        crate::mode::Mode::planned(task, plan),
        option.line()
    );
    if !option.detail.is_empty() {
        said.push('\n');
        said.push_str(&option.detail.join("\n"));
    }
    said.push_str(
        "\n\nFollow that option. Where the code shows it to be wrong, do what \
         the task needs instead and say so.",
    );
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "OPTION-abcdef0123456789:";

    fn answer() -> String {
        format!(
            "The parser reads one token at a time.\n\
             Steps: 1. read it. 2. fix it.\n\n\
             {M} A: rewrite the parser\n\
             Costs a day. Fixes every case of this shape.\n\
             {M} B: patch the one case\n\
             An hour, and the next report of this comes back.\n"
        )
    }

    #[test]
    fn the_options_are_read_in_the_order_they_were_written() {
        let found = parse(&answer(), M);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "A");
        assert_eq!(found[0].title, "rewrite the parser");
        assert_eq!(
            found[0].detail,
            ["Costs a day. Fixes every case of this shape."]
        );
        assert_eq!(found[1].label, "B");
        assert_eq!(found[1].line(), "B: patch the one case");
    }

    /// The plan is what came before the first option, and the options are not
    /// part of what a run is told to follow.
    #[test]
    fn the_plan_is_what_came_before_the_first_option() {
        let plan = plan_only(&answer(), M);
        assert!(plan.starts_with("The parser reads one token"));
        assert!(!plan.contains("rewrite the parser"), "{plan}");
    }

    /// A plan with nothing to choose between offers nothing, and so does one
    /// that names a single alternative: today's behaviour, exactly.
    #[test]
    fn fewer_than_two_options_is_not_a_choice() {
        assert!(parse("just a plan, in prose", M).is_empty());
        assert!(parse(&format!("a plan\n{M} A: the only way\n"), M).is_empty());
        assert_eq!(
            plan_only("just a plan, in prose", M),
            "just a plan, in prose"
        );
    }

    /// The marker is what makes reading a line anywhere safe. A plan quoting a
    /// file that talks about options offers none.
    #[test]
    fn only_the_marker_marks_an_option() {
        let quoting = "The file says:\n\
                       OPTION A: rewrite everything\n\
                       OPTION-0000000000000000: B: not this marker either\n\
                       and that is what it says.";
        assert!(parse(quoting, M).is_empty());
        assert_eq!(plan_only(quoting, M), quoting);
    }

    /// Half a marked line is not an option: the planner was asked for a label
    /// and a title, and guessing which half is missing puts words in its mouth.
    #[test]
    fn a_marked_line_missing_its_label_or_title_is_not_an_option() {
        let half = format!(
            "plan\n{M} A: good one\n{M} no colon here\n{M} : nothing before it\n\
             {M} C:\n{M} D: also good\n"
        );
        let found = parse(&half, M);
        let labels: Vec<&str> = found.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["A", "D"], "{found:#?}");
        // And the prose of a line that was not an option does not become the
        // trade-off of the option above it.
        assert!(found[0].detail.is_empty(), "{found:#?}");
    }

    #[test]
    fn a_marker_is_not_a_constant() {
        assert_ne!(marker("t1"), marker("t1"));
        assert!(marker("t1").starts_with("OPTION-"));
        assert!(asked_for(M).contains(M));
    }

    #[test]
    fn a_chosen_option_reaches_the_task_with_the_plan() {
        let found = parse(&answer(), M);
        let said = chosen("fix the parser", &plan_only(&answer(), M), &found[0]);
        assert!(said.contains("fix the parser"));
        assert!(said.contains("The parser reads one token"));
        assert!(said.contains("A: rewrite the parser"));
        assert!(said.contains("Costs a day."));
        assert!(said.contains("Follow that option."));
    }
}
