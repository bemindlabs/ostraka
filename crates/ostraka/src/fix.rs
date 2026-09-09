//! Walking a remedy from a shell.
//!
//! The browser has been able to take these steps since it learned to diagnose
//! anything: `ctrl-x x` names the problem, shows the way out of it, and takes
//! one step per keypress. The command line could only ever say what was wrong.
//!
//! The steps themselves are [`crate::remedy`], which is why they moved out of
//! `tui/` — nothing about them was ever about a terminal UI, and two
//! implementations of "what to do about a repository with no commits" would
//! eventually be two different answers.
//!
//! **One step per answer, and each one asked for.** The second step commits
//! whatever is lying in the operator's directory. That is not a thing to do on
//! somebody's behalf because they typed a flag, so it is offered, described,
//! and taken only on a yes.
//!
//! **Never without a terminal.** A `--fix` in CI that met the commit step would
//! either hang on a question nobody answers or commit a working directory
//! unattended, and both are worse than the error it was asked to remove.

use crate::remedy::Remedy;
use std::io::{BufRead, Write};
use std::path::Path;

/// How the walk ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every step was taken and the diagnosis is now clean.
    Fixed,
    /// A step was declined, or one was reached that only a person can take.
    Stopped,
    /// A step ran and failed. The reason was printed as the tool gave it.
    Failed,
}

/// Offers each step of a remedy in turn.
///
/// `input` and `output` are taken rather than reached for, so the whole
/// exchange can be driven by a test — which is the only way to be sure that a
/// walk which commits somebody's directory does it exactly when they said so.
pub fn walk(
    project: &Path,
    mut remedy: Remedy,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> std::io::Result<Outcome> {
    writeln!(output, "{}", remedy.problem)?;

    while !remedy.done() && !remedy.failed {
        let Some(step) = remedy.steps.get(remedy.at) else {
            break;
        };
        writeln!(output, "\n{}. {}", remedy.at + 1, step.said)?;
        if let Some(warns) = &step.warns {
            writeln!(output, "   {warns}")?;
        }

        // A step with nothing to run is not one this can take. Saying so beats
        // asking a question whose yes would do nothing.
        if step.commands.is_empty() {
            writeln!(output, "\n   This one is yours to do.")?;
            return Ok(Outcome::Stopped);
        }
        for command in &step.commands {
            writeln!(output, "   $ {}", command.join(" "))?;
        }

        write!(output, "\nRun it? [y] yes, anything else stops: ")?;
        output.flush()?;
        let mut answer = String::new();
        // A stream that ended is not a yes.
        if input.read_line(&mut answer)? == 0 || !answer.trim().eq_ignore_ascii_case("y") {
            return Ok(Outcome::Stopped);
        }

        remedy.take_step(project);
        if remedy.failed {
            if let Some(said) = &remedy.said {
                writeln!(output, "\n{said}")?;
            }
            return Ok(Outcome::Failed);
        }
    }

    // Asked again rather than assumed. The steps are what somebody worked out
    // ought to fix it; whether it did is a question for the diagnosis.
    match Remedy::diagnose(project) {
        Some(still) => {
            writeln!(output, "\nstill: {}", still.problem)?;
            Ok(Outcome::Stopped)
        }
        None => {
            writeln!(output, "\nnothing is in the way now")?;
            Ok(Outcome::Fixed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-fix-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_walk_nobody_says_yes_to_changes_nothing() {
        // The property that matters most here: the second step commits whatever
        // is in the directory, and a flag is not consent.
        let dir = scratch("declined");
        std::fs::write(dir.join("seed.txt"), "seed\n").expect("write");
        let remedy = Remedy::diagnose(&dir).expect("a problem");
        let mut input = std::io::Cursor::new(b"n\n".to_vec());
        let mut out: Vec<u8> = Vec::new();
        let ended = walk(&dir, remedy, &mut input, &mut out).expect("walks");
        assert_eq!(ended, Outcome::Stopped);
        assert!(!dir.join(".git").exists(), "a step ran without a yes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stream_that_ends_is_not_a_yes() {
        let dir = scratch("eof");
        std::fs::write(dir.join("seed.txt"), "seed\n").expect("write");
        let remedy = Remedy::diagnose(&dir).expect("a problem");
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(
            walk(&dir, remedy, &mut input, &mut out).expect("walks"),
            Outcome::Stopped
        );
        assert!(!dir.join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}");
    }

    #[test]
    fn yes_to_every_step_leaves_a_directory_with_nothing_wrong_with_it() {
        // The same claim the browser's walk makes, from the other path, and
        // measured the same way: by asking the diagnosis again rather than by
        // trusting that the steps did what they said.
        //
        // The repository and its identity are made here rather than by the
        // first step, because the step this test is about is the second one and
        // a machine may have no identity of its own — which is every runner.
        let dir = scratch("fixed");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@example.invalid"]);
        git(&dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("seed.txt"), "seed\n").expect("write");

        let remedy = Remedy::diagnose(&dir).expect("a problem");
        let mut input = std::io::Cursor::new(b"y\ny\ny\n".to_vec());
        let mut out: Vec<u8> = Vec::new();
        let ended = walk(&dir, remedy, &mut input, &mut out).expect("walks");
        assert_eq!(ended, Outcome::Fixed, "{}", String::from_utf8_lossy(&out));
        assert!(Remedy::diagnose(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
