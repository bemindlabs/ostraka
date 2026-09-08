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
/// The second agent, reading what the first one wrote.
pub const REVIEW: Color = Color::Blue;

/// The bar in the left gutter marking the selected row.
pub const CURSOR: &str = "\u{258c}";
/// The rule under a selected row, tying its second line to its first.
pub const CONTINUE: &str = "\u{2502}";
/// A run that was approved.
pub const PASSED: &str = "\u{2713}";
/// A run that was refused.
pub const FAILED: &str = "\u{2717}";

/// Frames for the mark that says something is still happening.
///
/// Braille rather than a spinning slash: the Braille Patterns block is neutral
/// width, so it is one cell everywhere, and it turns without the jitter a
/// four-frame ASCII spinner has. Which frame is showing is a function of the
/// event loop's tick, not of the clock — drawing has to stay assertable.
pub const SPINNER: [&str; 8] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
];

/// The line that separates one region from the next.
///
/// Space alone turned out not to be enough. A screen with four regions and no
/// rules between them reads as one region with gaps in it, and the eye has to
/// work out where the transcript stops and the input starts every time it
/// looks. Muted, so it divides without competing.
pub const RULE: &str = "\u{2500}";

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

/// A full-width divider.
pub fn rule(width: u16) -> ratatui::text::Line<'static> {
    ratatui::text::Line::from(ratatui::text::Span::styled(
        RULE.repeat(width as usize),
        muted(),
    ))
}

/// A divider with something written into its right-hand end.
///
/// Used where the rule is also a heading — the top of one task in a thread,
/// which is both "a new thing starts here" and "the last one ended like this".
pub fn labelled_rule(width: u16, label: &str, colour: Color) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};
    let width = width as usize;
    let label = label.trim();
    if label.is_empty() || width < 12 {
        return rule(width as u16);
    }
    // Cut to fit rather than dropped. The label that overflows is the long
    // one, and the long one is a vendor explaining why it stopped — which is
    // the sentence somebody is looking for. Dropping it left a bare rule where
    // the reason should have been.
    let after = 2;
    let room = width - after - 2;
    let label: String = if label.chars().count() > room {
        label
            .chars()
            .take(room.saturating_sub(1))
            .chain(std::iter::once('\u{2026}'))
            .collect()
    } else {
        label.to_string()
    };
    let before = width - label.chars().count() - after - 2;
    Line::from(vec![
        Span::styled(RULE.repeat(before), muted()),
        Span::styled(format!(" {label} "), on(colour)),
        Span::styled(RULE.repeat(after), muted()),
    ])
}

/// Moves an area in from the left by the gutter, so text has room to breathe.
pub fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(GUTTER),
        width: area.width.saturating_sub(GUTTER),
        ..area
    }
}
