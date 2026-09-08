//! The palette and the glyphs, in one place.
//!
//! Sixteen colours and no more: every value here is an ANSI base colour, so the
//! hues come from the theme the operator already chose for their terminal
//! rather than from a table shipped inside this binary. A true-colour palette
//! would look identical on every machine, which sounds like the point and is
//! not — it would look identical *and* wrong beside every other window on that
//! desktop, and it would be unreadable over ssh to the eight-colour terminals
//! that are still out there.
//!
//! The drawn shapes are box characters rather than a border widget's defaults,
//! because a rounded box and a block cursor are the two marks this screen uses
//! to say "here" — and both are in the Unicode block-drawing range, which is
//! unambiguously one cell wide everywhere. The two outcome ticks are not: they
//! are East Asian Ambiguous, so a terminal configured for CJK width draws them
//! two cells wide and shifts that column by one. Accepted knowingly: they are
//! what every other tool in this terminal already uses for the same thing, and
//! the alternative is a screen that reads as a compiler's.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

/// Ordinary text. `Reset` rather than white: the terminal's own foreground is
/// the only colour guaranteed to be readable on the terminal's own background.
pub const TEXT: Color = Color::Reset;
/// Everything secondary — labels, chrome, hints, the parts you read second.
pub const MUTED: Color = Color::DarkGray;
/// The one colour that means "this is what you are looking at".
pub const ACCENT: Color = Color::Cyan;
/// A check that passed, a run that was approved.
pub const OK: Color = Color::Green;
/// A check that failed, a run that was refused.
pub const BAD: Color = Color::Red;
/// Something the operator did, or is about to.
pub const WARN: Color = Color::Yellow;
/// A run that never reached a verdict, which is neither pass nor fail.
pub const HALTED: Color = Color::Magenta;

/// The bar in the left gutter marking the selected row.
pub const CURSOR: &str = "\u{258c}";
/// The rule under a selected row, tying its second line to its first.
pub const CONTINUE: &str = "\u{2502}";
/// A run that was approved.
pub const PASSED: &str = "\u{2713}";
/// A run that was refused.
pub const FAILED: &str = "\u{2717}";

/// One column of air at the left edge, so nothing starts against the frame.
pub const GUTTER: u16 = 1;

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn on(colour: Color) -> Style {
    Style::default().fg(colour)
}

/// The rounded box every framed thing on this screen is framed in.
///
/// There is exactly one, and only two things use it: the input line and a
/// dialog. Everything else is separated by space, which is why those two read
/// as the places something is happening.
pub fn panel(active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if active { accent() } else { muted() })
}

/// A rectangle of the given size in the middle of `area`, clamped to fit.
pub fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Moves an area in from the left by the gutter, so text has room to breathe.
pub fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(GUTTER),
        width: area.width.saturating_sub(GUTTER),
        ..area
    }
}
