//! Independent review.
//!
//! Review is an ordinary task run through a *different* adapter profile. There
//! is no reviewer model and no special protocol: the reviewer sees the diff and
//! answers in one line.

use ostraka_core::gate::Verdict;

/// The instruction given to a reviewer.
///
/// Carries the task, because "did this stay inside what was asked" is not a
/// question a reviewer holding only a diff can answer — it would be scored as
/// plausibility instead, which approves any change that merely looks reasonable.
///
/// Deliberately free of vendor names and of any hint about which agent wrote the
/// change: a reviewer that knows the author is a reviewer with a reason to defer.
/// The task is what was asked; the author is who answered. Only the first is the
/// reviewer's business.
pub fn review_prompt(task: &str, diff: &str, marker: &str) -> String {
    format!(
        "Review the change below against the task it was meant to perform. Judge \
         two things: whether it is correct, and whether it stays inside what was \
         asked — an unrequested change is a rejection even when it is an \
         improvement.\n\n\
         Say whatever you need to. End with exactly one line of this form, on a \
         line of its own:\n\
         {marker} APPROVE\n\
         or\n\
         {marker} REJECT: <one sentence>\n\n\
         Use that marker exactly. An answer without it is read as a rejection, \
         and so is an answer with more than one of it.\n\n\
         ----- task -----\n{task}\n\n\
         ----- diff -----\n{diff}"
    )
}

/// The marker a reviewer must use for this one review.
///
/// It exists so that the verdict can be read from anywhere in the answer
/// without an author being able to plant one. The author ran to completion
/// before this was derived, and it draws on the clock at review time, so a
/// string written into a file during authoring cannot match it. Not a
/// cryptographic guarantee and not offered as one: the property needed is that
/// the value did not exist while the diff was being written.
pub fn verdict_marker(run_id: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    run_id.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    format!("VERDICT-{:016x}:", hasher.finish())
}

/// Reads a reviewer's answer. Fail-safe: silence, a crash, or prose is rejection.
pub fn parse_verdict(output: &str, marker: &str) -> Verdict {
    Verdict::parse(output, marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_no_vendor_and_no_author() {
        let prompt = review_prompt("rename a field", "--- a/x\n+++ b/x", "VERDICT-1:");
        for forbidden in ["claude", "codex", "copilot", "gemini", "archon", "ephor"] {
            assert!(
                !prompt.to_lowercase().contains(forbidden),
                "review prompt leaked {forbidden:?}"
            );
        }
    }

    #[test]
    fn the_reviewer_is_told_what_was_asked() {
        let prompt = review_prompt("rename a field", "--- a/x\n+++ b/x", "VERDICT-1:");
        assert!(prompt.contains("rename a field"));
        assert!(prompt.contains("--- a/x"));
    }

    #[test]
    fn an_empty_answer_is_a_rejection() {
        assert!(!parse_verdict("", "VERDICT-1:").is_approve());
    }

    #[test]
    fn the_marker_differs_between_reviews_of_the_same_run() {
        // Same run id, two reviews: an author that learned one marker cannot
        // reuse it, and it never saw either.
        assert_ne!(verdict_marker("t1"), verdict_marker("t1"));
        assert!(verdict_marker("t1").starts_with("VERDICT-"));
    }
}
