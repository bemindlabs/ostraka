//! Selecting text in a pane, and getting it out of the browser.
//!
//! The mouse is captured — a click places the cursor in the box and the wheel
//! scrolls the column under it — and a terminal that reports the mouse to a
//! program stops selecting on a drag. The answer for as long as that was true
//! was to hold shift, which is the terminal's own selection and is the wrong
//! selection here: panes sit side by side, so a terminal dragging across a row
//! takes the neighbouring pane's text with it and produces lines belonging to
//! two different runs. It also takes the rule between them.
//!
//! So the selection is the browser's. It is made in the coordinates the
//! transcript is written in — a line index into what `thread_lines` produced,
//! and a display column within that line — rather than in screen cells, so
//! scrolling under a selection moves the selection with the text rather than
//! leaving it behind on the rows it was drawn over.
//!
//! Getting it out is OSC 52: the terminal is asked to put the text on the
//! clipboard, which is the only way that works over ssh, where there is no
//! clipboard on the machine this is running on. Not every terminal answers it
//! — Terminal.app and VTE do not — so the status line says what was asked
//! rather than claiming it was done.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Where in a transcript one end of a selection sits.
///
/// `line` indexes the lines a transcript was rendered into; `column` is a
/// display column within that line, so a wide character is one position rather
/// than two and a selection cannot land inside one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Spot {
    pub line: usize,
    pub column: u16,
}

/// A selection in one pane's transcript.
///
/// The anchor is where the drag started and the head is where it is now, in
/// that order and not sorted: dragging back above the anchor is an ordinary
/// thing to do, and the pair has to survive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub pane: usize,
    pub anchor: Spot,
    pub head: Spot,
    /// The mouse button is still down. Drawn the same either way; what it
    /// decides is whether a drag extends this selection or starts another.
    pub dragging: bool,
}

impl Selection {
    pub fn new(pane: usize, at: Spot) -> Self {
        Self {
            pane,
            anchor: at,
            head: at,
            dragging: true,
        }
    }

    /// The two ends in reading order.
    pub fn range(&self) -> (Spot, Spot) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Whether anything is actually selected.
    ///
    /// A click is a drag of no distance, and it means "put the keys in this
    /// pane" rather than "select nothing" — so an empty selection is dropped
    /// rather than drawn as a selection covering one cell.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The selected text, as it would be pasted.
///
/// Trailing blanks on each line are cut. A transcript is padded to the width
/// of the pane, so keeping them would paste a rectangle of spaces — and the
/// selection people make is "these lines", not "this rectangle".
pub fn text(lines: &[Line<'_>], selection: &Selection) -> String {
    let (from, to) = selection.range();
    let mut out: Vec<String> = Vec::new();
    for index in from.line..=to.line.min(lines.len().saturating_sub(1)) {
        let Some(line) = lines.get(index) else { break };
        let start = if index == from.line { from.column } else { 0 };
        let end = if index == to.line {
            to.column
        } else {
            u16::MAX
        };
        out.push(slice(line, start, end).trim_end().to_string());
    }
    out.join("\n")
}

/// The same lines, with the selected cells reversed.
///
/// Reversed rather than given a colour of its own: this is the terminal's own
/// way of saying "selected", it reads the same on every palette, and colour
/// here already means which part of a run spoke.
pub fn highlight(lines: Vec<Line<'static>>, selection: &Selection) -> Vec<Line<'static>> {
    let (from, to) = selection.range();
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index < from.line || index > to.line {
                return line;
            }
            let start = if index == from.line { from.column } else { 0 };
            let end = if index == to.line {
                to.column
            } else {
                u16::MAX
            };
            reverse(line, start, end)
        })
        .collect()
}

/// The display column a click at `cell` lands on, for a line that may hold
/// characters wider than one cell.
///
/// Clicking the right half of a wide character selects that character, which
/// is what every editor does and what stops a selection ending inside one.
pub fn column_at(line: Option<&Line<'_>>, cell: u16) -> u16 {
    let Some(line) = line else { return cell };
    let mut at = 0u16;
    for (character, _) in characters(line) {
        let width = UnicodeWidthChar::width(character).unwrap_or(0) as u16;
        if cell < at + width.max(1) {
            return at;
        }
        at += width;
    }
    at
}

/// Every character of a line, with the style it is drawn in.
fn characters<'a>(line: &'a Line<'a>) -> impl Iterator<Item = (char, Style)> + 'a {
    line.spans
        .iter()
        .flat_map(|span| span.content.chars().map(|c| (c, span.style)))
}

/// A line's text between two display columns.
fn slice(line: &Line<'_>, from: u16, to: u16) -> String {
    let mut out = String::new();
    let mut at = 0u16;
    for (character, _) in characters(line) {
        let width = UnicodeWidthChar::width(character).unwrap_or(0) as u16;
        if at >= to {
            break;
        }
        if at >= from {
            out.push(character);
        }
        at = at.saturating_add(width);
    }
    out
}

/// One line with the cells between two display columns reversed.
fn reverse(line: Line<'static>, from: u16, to: u16) -> Line<'static> {
    let style = line.style;
    let alignment = line.alignment;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut at = 0u16;
    for (character, span_style) in characters(&line) {
        let width = UnicodeWidthChar::width(character).unwrap_or(0) as u16;
        let wanted = if at >= from && at < to {
            span_style.add_modifier(Modifier::REVERSED)
        } else {
            span_style
        };
        // Grown rather than pushed one span per character: a transcript is
        // redrawn every tick, and a span per cell is a per-frame allocation
        // for every character on the screen.
        match spans.last_mut() {
            Some(last) if last.style == wanted => last.content.to_mut().push(character),
            _ => spans.push(Span::styled(character.to_string(), wanted)),
        }
        at = at.saturating_add(width);
    }
    let mut line = Line::from(spans).style(style);
    line.alignment = alignment;
    line
}

/// How much text is worth asking a terminal to take, in bytes.
///
/// Terminals cap what they accept over OSC 52 and most of them cap it without
/// saying so, which fails as a clipboard holding half a transcript. Refusing
/// out loud is the better half of that trade.
///
/// Bytes rather than characters because that is what the cap is about: the
/// sequence carries base64 of the bytes, and a line of Thai is three times the
/// payload of a line of English the same length on screen.
const MOST: usize = 100_000;

/// How the text reached a clipboard, for the sentence that reports it.
///
/// Which route was taken is worth saying, because the two can be trusted to
/// different depths: tmux answers, and a bare terminal does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Handed to tmux, which forwards it. `warning` is what will stop it
    /// reaching the system clipboard anyway, where that is knowable.
    Tmux { warning: Option<String> },
    /// Written straight at the terminal.
    Osc52,
}

impl Route {
    /// What the status line says. Never more than was actually established.
    pub fn describe(&self) -> String {
        match self {
            Self::Tmux { warning: None } => {
                "copied to the tmux buffer; tmux asked the terminal for the clipboard".to_string()
            }
            Self::Tmux { warning: Some(why) } => format!("copied to the tmux buffer, but {why}"),
            Self::Osc52 => "asked the terminal for the clipboard".to_string(),
        }
    }
}

/// Puts this on the clipboard, by whichever route can reach one.
///
/// OSC 52 rather than a clipboard crate, and the reason is the case this is
/// most needed in: over ssh there is no clipboard on the machine the browser
/// runs on, and every library that talks to one would be talking to the wrong
/// one. The terminal is the only thing in the room that can reach the right
/// clipboard, so the terminal is asked.
///
/// **tmux has to be asked through tmux.** This shipped saying the opposite —
/// that tmux's default `set-clipboard external` forwards an application's
/// OSC 52 already — and that is not what `external` means. tmux's own manual:
/// "If set to `external`, tmux will attempt to set the terminal clipboard but
/// ignore attempts by applications to set tmux buffers." So the sequence was
/// swallowed and nothing reached the clipboard, which on a machine reached
/// over ssh inside tmux is every copy anybody makes. Wrapping it in a DCS
/// passthrough is not the fix either: `allow-passthrough` is off by default in
/// tmux 3.3 and later, so that is swallowed too.
///
/// `tmux load-buffer -w -` is the one route `external` permits, because the
/// clipboard request then comes from tmux rather than from an application
/// inside it. It also reports: unlike the escape sequence, it exits non-zero
/// when it did not work.
pub fn to_clipboard(text: &str) -> Result<Route, String> {
    if text.len() > MOST {
        return Err(format!(
            "{} bytes is more than a terminal will take on the clipboard; \
             select less of it",
            text.len()
        ));
    }
    if std::env::var_os("TMUX").is_some() {
        return through_tmux(text).map(|warning| Route::Tmux { warning });
    }
    osc52(text).map(|()| Route::Osc52)
}

/// Hands the text to tmux, and says what will still stop it if anything will.
fn through_tmux(text: &str) -> Result<Option<String>, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("tmux could not be run: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("tmux took no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| format!("tmux would not take the text: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("tmux did not finish: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(format!("tmux refused it: {}", said.trim()));
    }
    Ok(tmux_will_not_forward())
}

/// Why tmux will keep this to itself, when it will.
///
/// `set-clipboard off` means the buffer is set and the terminal is never
/// asked, so the text is in tmux and nowhere else. That is a real outcome and
/// an invisible one — the copy looks like it worked — so it is named.
fn tmux_will_not_forward() -> Option<String> {
    let out = std::process::Command::new("tmux")
        .args(["show", "-gv", "set-clipboard"])
        .output()
        .ok()?;
    let setting = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (setting == "off").then(|| {
        "tmux `set-clipboard` is off, so it will not pass it on \u{2014} \
         `tmux set -g set-clipboard on`, or paste with tmux's own paste-buffer"
            .to_string()
    })
}

/// The escape sequence, written straight at the terminal.
fn osc52(text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

/// Standard base64 with padding, which is what OSC 52 carries.
///
/// Written here rather than depended on. It is twenty lines against a crate in
/// a binary whose distribution advantage is that it needs nothing, and the
/// alphabet has not changed since 1987.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (triple >> (18 - 6 * i)) & 0x3f;
                out.push(ALPHABET[index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn lines() -> Vec<Line<'static>> {
        vec![
            Line::from(vec![
                Span::styled("author", Style::default().fg(Color::Green)),
                Span::raw(" wrote a file"),
            ]),
            Line::from("gate: format passed"),
            Line::from("reviewer: approved"),
        ]
    }

    #[test]
    fn a_selection_across_lines_is_the_text_between_its_ends() {
        let selection = Selection {
            pane: 0,
            anchor: Spot { line: 0, column: 7 },
            head: Spot { line: 2, column: 9 },
            dragging: false,
        };
        assert_eq!(
            text(&lines(), &selection),
            "wrote a file\ngate: format passed\nreviewer:"
        );
    }

    /// Dragging upwards is an ordinary thing to do, and the pair has to
    /// survive it: the ends are ordered when they are read, not when they are
    /// set.
    #[test]
    fn dragging_backwards_selects_the_same_text() {
        let down = Selection {
            pane: 0,
            anchor: Spot { line: 0, column: 0 },
            head: Spot { line: 1, column: 4 },
            dragging: false,
        };
        let up = Selection {
            anchor: down.head,
            head: down.anchor,
            ..down
        };
        assert_eq!(text(&lines(), &down), text(&lines(), &up));
    }

    /// A transcript is padded to the width of the pane. Keeping that would
    /// paste a rectangle of spaces.
    #[test]
    fn the_padding_a_pane_draws_is_not_part_of_what_is_copied() {
        let padded = vec![Line::from("short              "), Line::from("next")];
        let selection = Selection {
            pane: 0,
            anchor: Spot { line: 0, column: 0 },
            head: Spot { line: 1, column: 4 },
            dragging: false,
        };
        assert_eq!(text(&padded, &selection), "short\nnext");
    }

    #[test]
    fn highlighting_reverses_the_selected_cells_and_nothing_else() {
        let selection = Selection {
            pane: 0,
            anchor: Spot { line: 1, column: 0 },
            head: Spot { line: 1, column: 4 },
            dragging: false,
        };
        let drawn = highlight(lines(), &selection);
        let reversed: String = drawn[1]
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(reversed, "gate");
        // The line above it keeps the colour that says who spoke.
        assert_eq!(drawn[0].spans[0].style.fg, Some(Color::Green));
        assert!(
            !drawn[0]
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        );
    }

    /// A selection is made in cells and the text is made of characters, and a
    /// wide one is two cells. Landing inside one would cut a character in half.
    #[test]
    fn a_click_on_either_half_of_a_wide_character_selects_the_character() {
        let wide = vec![Line::from("ab\u{4e16}cd")];
        assert_eq!(column_at(wide.first(), 2), 2);
        assert_eq!(column_at(wide.first(), 3), 2);
        assert_eq!(column_at(wide.first(), 4), 4);
        let selection = Selection {
            pane: 0,
            anchor: Spot { line: 0, column: 2 },
            head: Spot { line: 0, column: 4 },
            dragging: false,
        };
        assert_eq!(text(&wide, &selection), "\u{4e16}");
    }

    /// End to end, against the tmux this is running under.
    ///
    /// The unit tests above prove the text that comes out of a selection; this
    /// proves it arrives somewhere a person can paste from, which is the thing
    /// that was broken. Nothing is stubbed: `to_clipboard` runs the real
    /// `tmux load-buffer -w -`, and the buffer is read back with
    /// `tmux show-buffer`.
    ///
    /// One test rather than two, because the buffer they would read back is
    /// one buffer: written as a pair they overwrote each other whenever the
    /// harness ran them at the same time, which is a race in the test and not
    /// in what it is about.
    ///
    /// Outside tmux there is no clipboard to read back — a terminal never
    /// answers OSC 52 — so it says so rather than passing on nothing.
    #[test]
    fn under_tmux_the_text_lands_in_a_buffer_that_can_be_read_back() {
        if std::env::var_os("TMUX").is_none() {
            eprintln!(
                "not under tmux: the clipboard round trip is not asserted here. \
                 The OSC 52 route cannot be: a terminal never answers it."
            );
            return;
        }

        fn buffer() -> String {
            let out = std::process::Command::new("tmux")
                .arg("show-buffer")
                .output()
                .expect("tmux show-buffer runs");
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).trim_end().to_string()
        }

        let id = std::process::id();

        let one = format!("ostraka clipboard {id}");
        let route = to_clipboard(&one).expect("tmux took it");
        assert!(matches!(route, Route::Tmux { .. }), "{route:?}");
        assert_eq!(buffer(), one, "the buffer does not hold what was copied");

        // Several lines survive as several lines. A buffer holding one
        // run-together line would paste as one, which is not what was on the
        // screen.
        let many = format!("first {id}\nsecond {id}");
        to_clipboard(&many).expect("tmux took it");
        assert_eq!(buffer(), many, "the lines did not survive the round trip");
    }

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xef, 0xbe]), "/+++");
    }

    /// The cap is on the payload, and the payload is bytes. Raised in review:
    /// the message used to call them characters, which for anything but ASCII
    /// names a threshold nobody can check against what they selected.
    #[test]
    fn more_than_a_terminal_will_take_is_refused_rather_than_half_sent() {
        let long = "x".repeat(MOST + 1);
        let refused = to_clipboard(&long).expect_err("too long");
        assert!(refused.contains("bytes"), "{refused}");

        // Well under the cap in characters, over it in bytes.
        let thai = "\u{0e01}".repeat(MOST / 3 + 1);
        assert!(thai.chars().count() < MOST);
        assert!(
            to_clipboard(&thai).is_err(),
            "counted characters, not bytes"
        );
    }
}
