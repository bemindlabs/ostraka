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

/// How much text is worth asking a terminal to take.
///
/// Terminals cap what they accept over OSC 52 and most of them cap it without
/// saying so, which fails as a clipboard holding half a transcript. Refusing
/// out loud is the better half of that trade.
const MOST: usize = 100_000;

/// Asks the terminal to put this on the clipboard.
///
/// OSC 52 rather than a clipboard crate, and the reason is the case this is
/// most needed in: over ssh there is no clipboard on the machine the browser
/// runs on, and every library that talks to one would be talking to the wrong
/// one. The terminal is the only thing in the room that can reach the right
/// clipboard, so the terminal is asked.
///
/// tmux is not special-cased. Its default `set-clipboard external` forwards
/// this to the outer terminal already, and wrapping it in a passthrough
/// sequence is what would break that.
pub fn to_clipboard(text: &str) -> Result<(), String> {
    if text.len() > MOST {
        return Err(format!(
            "{} characters is more than a terminal will take on the clipboard; \
             select less of it",
            text.len()
        ));
    }
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

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xef, 0xbe]), "/+++");
    }

    #[test]
    fn more_than_a_terminal_will_take_is_refused_rather_than_half_sent() {
        let long = "x".repeat(MOST + 1);
        assert!(to_clipboard(&long).is_err());
    }
}
