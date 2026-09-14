//! The modifier the browser's chords are held with: command on macOS, control
//! everywhere else.
//!
//! One place, because a chord is named in the key handler, on the screen, in
//! the steps out of a problem and in the tests, and a label that says `ctrl-x`
//! over a handler that wants command is a key nobody can find.
//!
//! Only the browser's own chords move — the leader, the palette and the panes.
//! `ctrl-c` stays control on every platform: it is the interrupt everywhere and
//! command-c is copy. `ctrl-j` stays too, because it is a line feed.
//!
//! A terminal delivers command only when it speaks the kitty keyboard protocol,
//! so on macOS the browser asks for it on the way in and gives it back on the
//! way out.

use ratatui::crossterm::event::KeyModifiers;

#[cfg(target_os = "macos")]
pub const MODIFIER: KeyModifiers = KeyModifiers::SUPER;
#[cfg(not(target_os = "macos"))]
pub const MODIFIER: KeyModifiers = KeyModifiers::CONTROL;

/// Whether a key was pressed with the chord modifier held.
pub fn held(modifiers: KeyModifiers) -> bool {
    modifiers.contains(MODIFIER)
}

/// A chord as it is written for a person to read, as a string literal:
/// `label!("x")` is `cmd-x` on macOS and `ctrl-x` elsewhere.
///
/// A literal rather than a function so it fits in `concat!` and in the
/// `&'static str` tables the screen is drawn from.
#[cfg(target_os = "macos")]
macro_rules! label {
    ($key:literal) => {
        concat!("cmd-", $key)
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! label {
    ($key:literal) => {
        concat!("ctrl-", $key)
    };
}
pub(crate) use label;

/// Asks the terminal to report command as a modifier, where that is the chord.
///
/// Without the kitty keyboard protocol a terminal keeps command for itself and
/// the browser never hears it. One that does not speak the protocol ignores the
/// request, which is no worse than not asking.
pub fn enter() {
    #[cfg(target_os = "macos")]
    {
        use ratatui::crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

/// Gives back what [`enter`] asked for. A shell left in the protocol reads
/// every key as an escape sequence.
pub fn leave() {
    #[cfg(target_os = "macos")]
    {
        use ratatui::crossterm::event::PopKeyboardEnhancementFlags;
        let _ = ratatui::crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_names_the_modifier_the_handler_wants() {
        let expected = if MODIFIER == KeyModifiers::SUPER {
            "cmd-x"
        } else {
            "ctrl-x"
        };
        assert_eq!(label!("x"), expected);
        assert!(held(MODIFIER));
        assert!(!held(KeyModifiers::ALT));
    }

    #[test]
    fn command_is_the_chord_on_macos_and_control_everywhere_else() {
        assert_eq!(
            MODIFIER == KeyModifiers::SUPER,
            cfg!(target_os = "macos"),
            "the chord modifier does not match the platform"
        );
    }
}
