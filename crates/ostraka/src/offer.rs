//! Offering the profiles a workspace is missing, at the moment it needs them.
//!
//! A run that cannot route has already been told what is installed and not
//! configured — [`crate::discover`] puts that on the error. This is the next
//! step: where somebody is standing at a terminal, ask instead of only telling.
//!
//! **Only the command line, and only at a terminal.** `run::execute` is shared
//! with the browser, which has its own selector and no stdin anybody is typing
//! into, so the offer lives in the command's wrapper rather than in the run.
//! And it is skipped without a terminal, because the failure mode of getting
//! that wrong is a CI job that waits for an answer until something kills it —
//! worse than the error it was trying to improve on.
//!
//! **Accepting writes profiles.** It does not route around the workspace: a
//! profile is what a run is allowed to depend on, so the way to use an
//! installed CLI is to configure it, and then the run is one the workspace
//! agreed to. That is also what makes the offer worth taking twice — the
//! second run does not ask again.

use crate::discover::{Found, NoAdapter};
use crate::workspace::Workspace;
use std::io::{BufRead, IsTerminal, Write};

/// What the operator chose.
pub enum Choice {
    /// Profiles were written, and said so as it went. The caller may try the
    /// run again.
    Wrote,
    /// Nothing was written, and the run should fail as it was going to.
    Declined,
    /// There was nobody to ask, or nothing to offer.
    NotAsked,
}

/// Offers the installed profiles this workspace has not configured.
///
/// `input` and `output` are taken rather than reached for so that the whole
/// exchange can be driven by a test, which is the only way to be sure the
/// parsing is right about the answers people actually give.
pub fn profiles(
    workspace: &Workspace,
    problem: &NoAdapter,
    input: &mut impl BufRead,
    output: &mut impl Write,
    interactive: bool,
) -> std::io::Result<Choice> {
    let ready: Vec<&Found> = problem.found.iter().filter(|f| f.ready()).collect();
    if !interactive || ready.is_empty() {
        return Ok(Choice::NotAsked);
    }

    writeln!(output, "\n{}", problem.said)?;
    writeln!(
        output,
        "\nInstalled here, with no profile in this workspace:"
    )?;
    for (n, f) in ready.iter().enumerate() {
        writeln!(output, "  {}  {}", n + 1, f.id)?;
    }
    write!(
        output,
        "\nWrite profiles and run? [a] all, or numbers like 1,3 — anything else stops: "
    )?;
    output.flush()?;

    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        // The stream ended without an answer. Treated as a refusal rather than
        // as consent: nobody said yes.
        return Ok(Choice::Declined);
    }

    let picked = parse(answer.trim(), ready.len());
    if picked.is_empty() {
        return Ok(Choice::Declined);
    }

    for i in picked {
        let id = &ready[i].id;
        crate::init::write_profile(workspace, id)?;
        writeln!(output, "wrote adapters/{id}.toml")?;
    }
    Ok(Choice::Wrote)
}

/// The offered numbers an answer names, as indices.
///
/// Deliberately narrow. `a` is everything; a list of numbers is those numbers;
/// anything else is nothing, including a bare enter. An answer that was meant
/// as "no" and an answer that was a typo both stop, which is the safe direction
/// when the alternative is writing files somebody did not ask for.
fn parse(answer: &str, count: usize) -> Vec<usize> {
    if answer.eq_ignore_ascii_case("a") || answer.eq_ignore_ascii_case("all") {
        return (0..count).collect();
    }
    let mut picked: Vec<usize> = Vec::new();
    for part in answer.split(',') {
        match part.trim().parse::<usize>() {
            Ok(n) if n >= 1 && n <= count => {
                if !picked.contains(&(n - 1)) {
                    picked.push(n - 1);
                }
            }
            // One unreadable entry voids the answer. Acting on the half that
            // parsed would write a set nobody typed.
            _ => return Vec::new(),
        }
    }
    picked
}

/// Whether there is somebody at the other end to ask.
///
/// Both ends, because either one being a file is a run nobody is watching: a
/// question written to a redirected stdout is a question nobody sees, and a
/// prompt read from a redirected stdin is one nobody answered.
pub fn at_a_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(id: &str, ready: bool) -> Found {
        Found {
            id: id.to_string(),
            availability: if ready {
                ostraka_adapter::Availability::Ready {
                    version: Some("1".into()),
                }
            } else {
                ostraka_adapter::Availability::NotFound {
                    command: id.to_string(),
                }
            },
        }
    }

    #[test]
    fn every_answer_that_is_not_a_choice_stops() {
        // The safe direction. Writing files takes a yes, and only the two
        // shapes of yes count as one.
        for answer in ["", "n", "no", "y", "yes", "0", "9", "1,", "1,x", "-1", " "] {
            assert!(
                parse(answer, 3).is_empty(),
                "{answer:?} was read as a choice"
            );
        }
    }

    #[test]
    fn a_list_of_numbers_is_those_numbers_and_a_is_all_of_them() {
        assert_eq!(parse("a", 3), vec![0, 1, 2]);
        assert_eq!(parse("ALL", 2), vec![0, 1]);
        assert_eq!(parse("1,3", 3), vec![0, 2]);
        assert_eq!(parse(" 2 , 1 ", 3), vec![1, 0], "order is the answer's");
        assert_eq!(parse("2,2", 3), vec![1], "a repeat is not two profiles");
    }

    #[test]
    fn nobody_is_asked_without_a_terminal() {
        // The one that matters: a CI job asked a question waits for an answer
        // until something kills it, which is worse than the error this
        // improves on.
        let dir = std::env::temp_dir().join(format!("ostraka-offer-{}", std::process::id()));
        let workspace = Workspace::at(&dir);
        let problem = NoAdapter {
            said: "no usable adapter profile".into(),
            found: vec![found("claude-code", true)],
        };
        let mut input = std::io::Cursor::new(b"a\n".to_vec());
        let mut output: Vec<u8> = Vec::new();
        let choice = profiles(&workspace, &problem, &mut input, &mut output, false).expect("asks");
        assert!(matches!(choice, Choice::NotAsked));
        assert!(output.is_empty(), "a question was written to nobody");
    }

    #[test]
    fn nothing_installed_is_nothing_to_offer() {
        let dir = std::env::temp_dir().join(format!("ostraka-offer-none-{}", std::process::id()));
        let workspace = Workspace::at(&dir);
        let problem = NoAdapter {
            said: "no usable adapter profile".into(),
            found: vec![found("codex", false)],
        };
        let mut input = std::io::Cursor::new(b"a\n".to_vec());
        let mut output: Vec<u8> = Vec::new();
        let choice = profiles(&workspace, &problem, &mut input, &mut output, true).expect("asks");
        assert!(matches!(choice, Choice::NotAsked));
        assert!(output.is_empty());
    }

    #[test]
    fn a_stream_that_ends_without_an_answer_is_not_a_yes() {
        let dir = std::env::temp_dir().join(format!("ostraka-offer-eof-{}", std::process::id()));
        let workspace = Workspace::at(&dir);
        let problem = NoAdapter {
            said: "no usable adapter profile".into(),
            found: vec![found("claude-code", true)],
        };
        let mut input = std::io::Cursor::new(Vec::new());
        let mut output: Vec<u8> = Vec::new();
        let choice = profiles(&workspace, &problem, &mut input, &mut output, true).expect("asks");
        assert!(matches!(choice, Choice::Declined));
    }

    #[test]
    fn accepting_writes_exactly_what_was_chosen() {
        // The half the manual walkthrough covered and nothing pinned: an answer
        // naming two of three writes those two, and the third stays absent.
        let dir = std::env::temp_dir().join(format!("ostraka-offer-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let workspace = Workspace::at(&dir);
        let problem = NoAdapter {
            said: "no adapter profiles found".into(),
            found: vec![
                found("agy", true),
                found("claude-code", true),
                found("codex", true),
            ],
        };
        let mut input = std::io::Cursor::new(b"1,3\n".to_vec());
        let mut output: Vec<u8> = Vec::new();
        let choice = profiles(&workspace, &problem, &mut input, &mut output, true).expect("asks");
        assert!(matches!(choice, Choice::Wrote));

        let adapters = workspace.adapters();
        assert!(
            adapters.join("agy.toml").is_file(),
            "the first was not written"
        );
        assert!(
            adapters.join("codex.toml").is_file(),
            "the third was not written"
        );
        assert!(
            !adapters.join("claude-code.toml").exists(),
            "a profile nobody chose was written"
        );

        // And it is the profile `init` would have written, not an invention.
        let written = std::fs::read_to_string(adapters.join("agy.toml")).expect("readable");
        let shipped = crate::init::TEMPLATES
            .iter()
            .find(|(name, _)| *name == "agy.toml")
            .map(|(_, text)| *text)
            .expect("shipped");
        assert_eq!(written, shipped);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
