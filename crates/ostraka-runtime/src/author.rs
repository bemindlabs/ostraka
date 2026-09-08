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
//! **Only what is true of this run.** Each directory is described only where it
//! was actually linked, and a worktree with neither gets the task exactly as it
//! was typed. A standing preamble describing a directory half of all runs do
//! not have would teach agents to ignore the preamble.
//!
//! **Nothing about review.** The author is not told that another agent will
//! judge the change, and is never given the verdict marker: the marker is
//! derived after authoring has finished precisely so that a string written into
//! a file cannot match it, and a sentence about being reviewed invites writing
//! for the reviewer rather than for the task.

/// The instruction given to an author.
///
/// `notes` and `skills` say whether those directories were linked into this
/// worktree. They are the caller's facts to supply — read off the worktree,
/// not off the configuration — so a sentence cannot describe a link that was
/// never made.
///
/// The two are opposite directions of travel and are said in that order: what
/// the workspace asks of a run comes before what earlier runs left, because a
/// convention changes how the work is done and a note only informs it.
pub fn author_prompt(task: &str, notes: bool, skills: bool) -> String {
    if !notes && !skills {
        return task.to_string();
    }
    let mut said = format!("{task}\n\n----- about this worktree -----\n");

    if skills {
        said.push_str(
            "\n`skills/` is not part of this repository. It is the workspace's own \
             directory, linked in here, and it holds what this workspace expects of \
             a run: procedures, conventions, the way things are done here.\n\n\
             Read what applies before you start. It is written for you, and it is \
             not something the code can tell you.\n",
        );
    }

    if notes {
        said.push_str(
            "\n`notes/` is not part of this repository either. It is the workspace's \
             own directory, linked in here, and it is shared with every other run.\n\n\
             Read it before you start: what earlier runs worked out is in there, \
             including what was tried and did not work.\n\n\
             Write to it what the next run would want and cannot get from the diff \
             — a decision and why, a dead end worth not repeating, a fact about \
             this project that took effort to establish. What you write there is \
             kept whether or not this change lands, and it stays out of the change \
             itself.\n",
        );
    }

    // `skills/` is the workspace's word to a run, not a run's to edit. Said
    // rather than enforced: the link is a link, and a sentence that claimed it
    // was read-only would be a sentence the filesystem contradicts.
    //
    // Where to write instead is named only where it exists. Sending an author
    // to `notes/` in a worktree that has none is inventing a location, which is
    // the failure the whole "only what is true" rule is about — and it was
    // there, because the sentence said "Notes" where the test looked for
    // "`notes/`".
    if skills {
        said.push_str("\nLeave `skills/` as you found it.");
        said.push_str(if notes {
            " `notes/` is where a run writes.\n"
        } else {
            "\n"
        });
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_with_neither_hands_the_task_over_untouched() {
        // The author's instruction is the operator's sentence. Where neither
        // directory reached the worktree there is nothing else true about the
        // run, and adding anything would be putting words in their mouth.
        let task = "add a --json flag to the runs command";
        assert_eq!(author_prompt(task, false, false), task);
    }

    #[test]
    fn an_author_is_told_what_the_notes_directory_is_and_what_it_is_not() {
        let out = author_prompt("do the thing", true, false);
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
        let out = author_prompt("do the thing", true, false);
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
        let out = author_prompt("do the thing", true, false);
        for leak in ["review", "APPROVE", "reject", "verdict", "judge"] {
            assert!(
                !out.to_lowercase().contains(&leak.to_lowercase()),
                "the author was told about {leak}:\n{out}"
            );
        }
    }

    #[test]
    fn skills_are_described_as_what_the_workspace_asks_not_as_a_place_to_write() {
        let out = author_prompt("do the thing", false, true);
        assert!(out.starts_with("do the thing"), "{out}");
        assert!(
            out.contains("`skills/` is not part of this repository"),
            "{out}"
        );
        assert!(
            out.contains("what this workspace expects of a run"),
            "{out}"
        );
        assert!(out.contains("Leave `skills/` as you found it"), "{out}");
        // Without notes, nothing invites writing anywhere.
        assert!(!out.contains("Write to it"), "{out}");
    }

    #[test]
    fn what_the_workspace_asks_comes_before_what_earlier_runs_left() {
        // A convention changes how the work is done; a note only informs it.
        // An agent that reads the preamble top to bottom should meet the rule
        // before the history.
        let out = author_prompt("do the thing", true, true);
        let skills_at = out.find("`skills/`").expect("skills described");
        let notes_at = out.find("`notes/`").expect("notes described");
        assert!(skills_at < notes_at, "notes came first:\n{out}");
    }

    #[test]
    fn each_directory_is_named_only_where_it_was_linked() {
        // By word rather than by formatting. The first version of this looked
        // for "`notes/`" and missed a sentence that said "Notes" — which sent
        // an author to a directory the worktree did not have.
        let only_notes = author_prompt("t", true, false);
        assert!(
            !only_notes.to_lowercase().contains("skills"),
            "{only_notes}"
        );
        let only_skills = author_prompt("t", false, true);
        assert!(
            !only_skills.to_lowercase().contains("notes"),
            "{only_skills}"
        );
        // Both, where both are there.
        let both = author_prompt("t", true, true);
        assert!(both.to_lowercase().contains("skills") && both.to_lowercase().contains("notes"));
    }
}
