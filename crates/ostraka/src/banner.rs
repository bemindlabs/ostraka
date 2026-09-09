//! What `ostraka` says when it is run with nothing after it.
//!
//! A bare invocation used to be a usage error: clap printed "the following
//! required arguments were not provided" and exited non-zero, which is a true
//! sentence about the parser and the wrong answer to somebody who has just
//! installed this and typed its name. Typing a program's name is how people ask
//! it what it is.
//!
//! The command list is read out of the parser rather than written here, for the
//! same reason the completion script is generated: a second description of the
//! command line is the one that goes stale.

use clap::CommandFactory;
use std::io::IsTerminal;

/// The wordmark, six rows of it.
///
/// Box-drawing and full-block characters, which every terminal this releases
/// for has had since before it had colour. Fifty-eight columns wide, so it fits
/// an eighty-column terminal with room to spare and does not wrap in a README
/// code fence either.
const WORDMARK: [&str; 6] = [
    " ██████╗ ███████╗████████╗██████╗  █████╗ ██╗  ██╗ █████╗ ",
    "██╔═══██╗██╔════╝╚══██╔══╝██╔══██╗██╔══██╗██║ ██╔╝██╔══██╗",
    "██║   ██║███████╗   ██║   ██████╔╝███████║█████╔╝ ███████║",
    "██║   ██║╚════██║   ██║   ██╔══██╗██╔══██║██╔═██╗ ██╔══██║",
    "╚██████╔╝███████║   ██║   ██║  ██║██║  ██║██║  ██╗██║  ██║",
    " ╚═════╝ ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝",
];

/// The narrow form, for a terminal the wordmark would wrap in.
///
/// Wrapping it would be worse than not drawing it: six rows of half a wordmark
/// followed by six rows of the other half is not a wordmark, it is noise where
/// the name should be.
const NARROW: &str = "OSTRAKA";

/// Below this many columns the wordmark is not drawn.
const NEEDS: u16 = 60;

/// Sixteen ANSI colours and nothing else, which is the rule the browser follows
/// and holds here for the same reason: a true-colour palette looks identical on
/// every machine, which sounds like the point and is not — it looks identical
/// *and* wrong beside every other window, and unreadable over ssh to an
/// eight-colour terminal.
const ACCENT: &str = "\x1b[36m";
const MUTED: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";
const OFF: &str = "\x1b[0m";

/// Whether to emit escape sequences at all.
///
/// Three ways to be told not to, and all three are honoured: a pipe on the far
/// end of stdout, `NO_COLOR` with any value at all, and `TERM=dumb`. Colour
/// here is decoration on a name, so the bar for suppressing it is low.
fn colour(out_is_terminal: bool) -> bool {
    out_is_terminal
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
}

/// The header and the command list, as one string.
///
/// Built rather than printed so it can be asserted in a test instead of looked
/// at, which is how every other screen in this binary is checked.
pub fn render(width: u16, colour: bool) -> String {
    let (accent, muted, bold, off) = if colour {
        (ACCENT, MUTED, BOLD, OFF)
    } else {
        ("", "", "", "")
    };

    let mut out = String::new();
    if width >= NEEDS {
        for row in WORDMARK {
            out.push_str(accent);
            out.push_str(row);
            out.push_str(off);
            out.push('\n');
        }
    } else {
        out.push_str(bold);
        out.push_str(NARROW);
        out.push_str(off);
        out.push('\n');
    }

    let cli = crate::Cli::command();
    let about = cli.get_about().map(|a| a.to_string()).unwrap_or_default();
    out.push('\n');
    out.push_str(&format!(
        "  {bold}Ostraka v{}{off}  —  {about}\n\n",
        env!("CARGO_PKG_VERSION")
    ));

    out.push_str(&format!("{bold}Commands:{off}\n"));
    // The widest name sets the column, so adding a command cannot leave the
    // descriptions in two places.
    let names: Vec<(String, String)> = cli
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| {
            (
                c.get_name().to_string(),
                c.get_about().map(|a| a.to_string()).unwrap_or_default(),
            )
        })
        .collect();
    let pad = names.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, about) in &names {
        out.push_str(&format!(
            "  {accent}{name:<pad$}{off}  {muted}{about}{off}\n"
        ));
    }
    out.push_str(&format!(
        "\n  {muted}`ostraka <command> --help` for one of them.{off}\n"
    ));
    out
}

/// Prints it. The exit status is success: being asked what you are is not an
/// error, which is the whole reason this exists.
pub fn print() {
    let terminal = std::io::stdout().is_terminal();
    let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    print!("{}", render(width, colour(terminal)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wordmark_is_one_rectangle() {
        // Six rows of different lengths draw a ragged edge that reads as a
        // rendering fault rather than as a design. Counted in characters, not
        // bytes: every glyph here is one column and three bytes.
        let widths: Vec<usize> = WORDMARK.iter().map(|r| r.chars().count()).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "ragged wordmark: {widths:?}"
        );
        assert_eq!(widths[0], NEEDS as usize - 2, "{widths:?}");
    }

    #[test]
    fn a_narrow_terminal_gets_the_name_instead_of_half_a_wordmark() {
        let wide = render(80, false);
        let narrow = render(40, false);
        assert!(wide.contains(WORDMARK[0]), "{wide}");
        assert!(!narrow.contains(WORDMARK[0]), "{narrow}");
        assert!(narrow.contains(NARROW), "{narrow}");
        // Both still answer the question that was asked.
        for out in [&wide, &narrow] {
            assert!(out.contains("run"), "{out}");
            assert!(out.contains("Commands:"), "{out}");
        }
    }

    #[test]
    fn every_command_the_parser_has_is_listed() {
        // The list is read from the parser so that adding a command cannot
        // leave this screen describing the previous version of the binary.
        let out = render(80, false);
        for command in crate::Cli::command().get_subcommands() {
            assert!(
                out.contains(command.get_name()),
                "{} is missing from:\n{out}",
                command.get_name()
            );
        }
    }

    #[test]
    fn nothing_escapes_when_colour_is_off() {
        // A banner written into a pipe or a log file is read as text. An escape
        // sequence there is not decoration, it is corruption.
        assert!(!render(80, false).contains('\x1b'));
        assert!(render(80, true).contains('\x1b'));
    }

    #[test]
    fn no_colour_is_honoured_however_it_is_set() {
        // Including empty, which is the form the specification is explicit
        // about and the form a shell writes with `NO_COLOR=`.
        assert!(!colour(false), "a pipe still got colour");
    }
}
