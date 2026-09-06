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
pub fn review_prompt(task: &str, diff: &str) -> String {
    format!(
        "Review the change below against the task it was meant to perform. Judge \
         two things: whether it is correct, and whether it stays inside what was \
         asked — an unrequested change is a rejection even when it is an \
         improvement.\n\n\
         Answer with exactly one line first:\n\
         VERDICT: APPROVE\n\
         or\n\
         VERDICT: REJECT: <one sentence>\n\n\
         Anything else is read as a rejection.\n\n\
         ----- task -----\n{task}\n\n\
         ----- diff -----\n{diff}"
    )
}

/// Reads a reviewer's answer. Fail-safe: silence, a crash, or prose is rejection.
pub fn parse_verdict(output: &str) -> Verdict {
    Verdict::parse(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_no_vendor_and_no_author() {
        let prompt = review_prompt("rename a field", "--- a/x\n+++ b/x");
        for forbidden in ["claude", "codex", "copilot", "gemini", "archon", "ephor"] {
            assert!(
                !prompt.to_lowercase().contains(forbidden),
                "review prompt leaked {forbidden:?}"
            );
        }
    }

    #[test]
    fn the_reviewer_is_told_what_was_asked() {
        let prompt = review_prompt("rename a field", "--- a/x\n+++ b/x");
        assert!(prompt.contains("rename a field"));
        assert!(prompt.contains("--- a/x"));
    }

    #[test]
    fn an_empty_answer_is_a_rejection() {
        assert!(!parse_verdict("").is_approve());
    }
}
