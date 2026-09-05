//! Independent review.
//!
//! Review is an ordinary task run through a *different* adapter profile. There
//! is no reviewer model and no special protocol: the reviewer sees the diff and
//! answers in one line.

use ostraka_core::gate::Verdict;

/// The instruction given to a reviewer.
///
/// Deliberately free of vendor names and of any hint about which agent wrote the
/// change: a reviewer that knows the author is a reviewer with a reason to defer.
pub fn review_prompt(diff: &str) -> String {
    format!(
        "Review the change below for correctness and for whether it stays inside \
         what was asked.\n\n\
         Answer with exactly one line first:\n\
         VERDICT: APPROVE\n\
         or\n\
         VERDICT: REJECT: <one sentence>\n\n\
         Anything else is read as a rejection.\n\n\
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
        let prompt = review_prompt("--- a/x\n+++ b/x");
        for forbidden in ["claude", "codex", "copilot", "gemini", "archon", "ephor"] {
            assert!(
                !prompt.to_lowercase().contains(forbidden),
                "review prompt leaked {forbidden:?}"
            );
        }
    }

    #[test]
    fn an_empty_answer_is_a_rejection() {
        assert!(!parse_verdict("").is_approve());
    }
}
