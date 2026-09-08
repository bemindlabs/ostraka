//! What an author is told beyond the task.
//!
//! A reviewer gets a composed instruction — [`crate::review::review_prompt`]
//! explains what to judge and how to answer, because neither is guessable from
//! a diff. An author got the task and nothing else, which was right while the
//! task was the only thing true about the run.
//!
//! It stopped being the only thing. The workspace's notes directory is linked
//! into every worktree, it is shared across runs, it survives a refusal, and it
//! is outside the change a reviewer judges. All four are useful and none are
//! discoverable: a directory called `notes` beside the source reads like part of
//! the repository, and an agent that treats it that way will neither read what
//! earlier runs left nor leave anything for the next one.
//!
//! **Only what is true of this run.** Where a workspace has no notes there is
//! nothing to say, and the author gets the task exactly as it was typed —
//! today's behaviour, unchanged. A standing preamble that described a directory
//! half of all runs do not have would teach agents to ignore the preamble.
//!
//! **Nothing about review.** The author is not told that another agent will
//! judge the change, and is never given the verdict marker: the marker is
//! derived after authoring has finished precisely so that a string written into
//! a file cannot match it, and a sentence about being reviewed invites writing
//! for the reviewer rather than for the task.

/// The instruction given to an author.
///
/// `notes` says whether the workspace's notes directory was linked into this
/// worktree. It is the caller's fact to supply — the same `Option` the worktree
/// was prepared from — rather than something inferred here by looking at the
/// filesystem, so the sentence cannot describe a link that failed to be made.
pub fn author_prompt(task: &str, notes: bool) -> String {
    if !notes {
        return task.to_string();
    }
    format!(
        "{task}\n\n\
         ----- about this worktree -----\n\
         `notes/` is not part of this repository. It is the workspace's own \
         directory, linked in here, and it is shared with every other run.\n\n\
         Read it before you start: what earlier runs worked out is in there, \
         including what was tried and did not work.\n\n\
         Write to it what the next run would want and cannot get from the diff \
         — a decision and why, a dead end worth not repeating, a fact about \
         this project that took effort to establish. What you write there is \
         kept whether or not this change lands, and it stays out of the change \
         itself."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_without_notes_hands_the_task_over_untouched() {
        // The author's instruction is the operator's sentence. Where there is
        // nothing else true about the run, adding anything at all would be
        // putting words in the operator's mouth.
        let task = "add a --json flag to the runs command";
        assert_eq!(author_prompt(task, false), task);
    }

    #[test]
    fn an_author_is_told_what_the_notes_directory_is_and_what_it_is_not() {
        let out = author_prompt("do the thing", true);
        assert!(
            out.starts_with("do the thing"),
            "the task comes first: {out}"
        );
        // The four properties that make it worth using, and that none of them
        // can be read off the directory itself.
        assert!(out.contains("not part of this repository"), "{out}");
        assert!(out.contains("shared with every other run"), "{out}");
        assert!(out.contains("whether or not this change lands"), "{out}");
        assert!(out.contains("stays out of the change itself"), "{out}");
    }

    #[test]
    fn an_author_is_told_to_read_before_it_is_told_to_write() {
        // Reading is the half that pays on the first run of a fleet; writing
        // pays later. An instruction that led with writing would be asking for
        // a diary rather than for continuity.
        let out = author_prompt("do the thing", true);
        let read_at = out.find("Read it").expect("asked to read");
        let write_at = out.find("Write to it").expect("asked to write");
        assert!(read_at < write_at, "writing was asked for first:\n{out}");
    }

    #[test]
    fn the_author_is_told_nothing_about_being_reviewed() {
        // The verdict marker is derived after authoring so that nothing written
        // during it can match. Mentioning review at all invites writing for the
        // reviewer, and mentioning the marker would undo the property outright.
        //
        // This caught its own author: a first draft promised the notes survive
        // "even if this change is rejected", which is a sentence about review
        // wearing a different hat. The substring match is deliberate — `reject`
        // is what found `rejected`.
        let out = author_prompt("do the thing", true);
        for leak in ["review", "APPROVE", "reject", "verdict", "judge"] {
            assert!(
                !out.to_lowercase().contains(&leak.to_lowercase()),
                "the author was told about {leak}:\n{out}"
            );
        }
    }
}
