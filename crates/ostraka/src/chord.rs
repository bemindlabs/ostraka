//! The browser's own chords: which keys reach them, and how the screen names
//! them.
//!
//! **Control reaches every chord, on every platform.** A terminal decides what
//! a program hears, and most terminals on macOS keep command for themselves:
//! Terminal.app cuts on command-x, clears on command-k and opens a tab on
//! command-t, and never passes any of them on. A chord held with command alone
//! was a chord nobody on those terminals could press. So control always works,
//! and on macOS command works as well wherever the terminal speaks the kitty
//! keyboard protocol and has not bound the key itself. The screen names the
//! first binding of each chord, which by default is the one that works
//! everywhere.
//!
//! **Somebody else's terminal can rebind them.** `keys.toml` in the per-user
//! configuration directory names other keys for any of the five, because which
//! keys a terminal leaves alone is a fact about that person's terminal, not
//! about a workspace. A file that cannot be read as bindings is refused by name
//! when the browser opens, not quietly replaced by defaults, and a binding that
//! would take a key the browser needs is refused with it.
//!
//! `ctrl-c` and `ctrl-j` are not chords here and cannot be rebound. The first is
//! the way out and the second is a line feed.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a chord does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Leader,
    Commands,
    NewPane,
    NextPane,
    PreviousPane,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Leader,
        Action::Commands,
        Action::NewPane,
        Action::NextPane,
        Action::PreviousPane,
    ];

    /// Its name in `keys.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Leader => "leader",
            Action::Commands => "commands",
            Action::NewPane => "new_pane",
            Action::NextPane => "next_pane",
            Action::PreviousPane => "previous_pane",
        }
    }

    fn default_key(self) -> char {
        match self {
            Action::Leader => 'x',
            Action::Commands => 'k',
            Action::NewPane => 't',
            Action::NextPane => ']',
            Action::PreviousPane => '[',
        }
    }
}

/// One key, with the modifiers that must be held for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    modifiers: KeyModifiers,
    key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Char(char),
    F(u8),
}

/// The modifiers a binding may name. Shift is ignored when comparing, because a
/// terminal reports it for any key typed with it and `]` and `}` share a key.
const HELD: [KeyModifiers; 3] = [
    KeyModifiers::CONTROL,
    KeyModifiers::ALT,
    KeyModifiers::SUPER,
];

impl Binding {
    fn new(modifiers: KeyModifiers, key: char) -> Self {
        Self {
            modifiers,
            key: Key::Char(key),
        }
    }

    /// Reads `ctrl-x`, `cmd-k`, `alt-shift-t`, `f2` and the like.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim().to_lowercase();
        // The key is whatever follows the last separator, so `ctrl--` is
        // control and the minus key.
        let (mods, key) = match spec.strip_suffix("--") {
            Some(mods) => (mods, "-"),
            None => match spec.rsplit_once('-') {
                Some((mods, key)) => (mods, key),
                None => ("", spec.as_str()),
            },
        };
        let mut modifiers = KeyModifiers::NONE;
        for part in mods.split('-').filter(|p| !p.is_empty()) {
            modifiers |= match part {
                "ctrl" | "control" => KeyModifiers::CONTROL,
                "cmd" | "command" | "super" => KeyModifiers::SUPER,
                "alt" | "option" | "opt" | "meta" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                other => return Err(format!("{other:?} is not a modifier in {spec:?}")),
            };
        }
        let key = if let Some(n) = key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| (1..=24).contains(n))
        {
            Key::F(n)
        } else {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => return Err(format!("{key:?} is not one key in {spec:?}")),
            }
        };
        let binding = Self { modifiers, key };
        // A letter with nothing held is a letter somebody meant to type into
        // the task. Function keys type nothing, so they may stand alone.
        if matches!(key, Key::Char(_)) && !HELD.iter().any(|m| modifiers.contains(*m)) {
            return Err(format!(
                "{spec:?} holds no modifier, so it would take a key the task box types"
            ));
        }
        for reserved in ['c', 'j'] {
            if binding.key == Key::Char(reserved)
                && modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
            {
                return Err(format!(
                    "{spec:?} is ctrl-{reserved}, which the browser keeps: ctrl-c leaves and ctrl-j is a new line"
                ));
            }
        }
        Ok(binding)
    }

    /// Whether this key event is this binding.
    fn matches(&self, event: &KeyEvent) -> bool {
        let key_matches = match (self.key, event.code) {
            (Key::Char(want), KeyCode::Char(got)) => want == got.to_ascii_lowercase(),
            (Key::F(want), KeyCode::F(got)) => want == got,
            _ => false,
        };
        key_matches
            && HELD
                .iter()
                .all(|m| self.modifiers.contains(*m) == event.modifiers.contains(*m))
    }

    /// As a person reads it.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (flag, word) in [
            (KeyModifiers::CONTROL, "ctrl"),
            (KeyModifiers::SUPER, "cmd"),
            (KeyModifiers::ALT, "alt"),
            (KeyModifiers::SHIFT, "shift"),
        ] {
            if self.modifiers.contains(flag) {
                out.push_str(word);
                out.push('-');
            }
        }
        match self.key {
            Key::Char(c) => out.push(c),
            Key::F(n) => out.push_str(&format!("f{n}")),
        }
        out
    }
}

/// Every chord's bindings, and the labels the screen draws from them.
#[derive(Debug, Clone)]
pub struct Keys {
    bindings: Vec<(Action, Vec<Binding>)>,
    labels: Vec<(Action, String)>,
    panes: String,
}

impl Keys {
    /// Control and the chord's letter everywhere, and on macOS command as well.
    pub fn defaults() -> Self {
        Self::from(
            Action::ALL
                .into_iter()
                .map(|action| {
                    let key = action.default_key();
                    let mut bindings = vec![Binding::new(KeyModifiers::CONTROL, key)];
                    if cfg!(target_os = "macos") {
                        bindings.push(Binding::new(KeyModifiers::SUPER, key));
                    }
                    (action, bindings)
                })
                .collect(),
        )
    }

    fn from(bindings: Vec<(Action, Vec<Binding>)>) -> Self {
        let labels: Vec<(Action, String)> = bindings
            .iter()
            .map(|(action, keys)| {
                (
                    *action,
                    keys.first().map(Binding::label).unwrap_or_default(),
                )
            })
            .collect();
        let find = |action: Action| {
            labels
                .iter()
                .find(|(a, _)| *a == action)
                .map(|(_, l)| l.clone())
                .unwrap_or_default()
        };
        let panes = format!(
            "{} / {}",
            find(Action::NextPane),
            find(Action::PreviousPane)
        );
        Self {
            bindings,
            labels,
            panes,
        }
    }

    /// The defaults with whatever `text` rebinds laid over them.
    ///
    /// Each entry is one binding or a list of them, and replaces that chord's
    /// defaults entirely: somebody rebinding the leader because command-x cuts
    /// in their terminal does not want command-x kept.
    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut keys = Self::defaults().bindings;
        for (name, value) in &table {
            let Some(action) = Action::ALL.into_iter().find(|a| a.name() == name) else {
                let known: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
                return Err(format!(
                    "{name:?} is not a chord; the chords are {}",
                    known.join(", ")
                ));
            };
            let specs: Vec<&str> = match value {
                toml::Value::String(one) => vec![one.as_str()],
                toml::Value::Array(many) => many
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| format!("{name}: every binding is a string"))
                    })
                    .collect::<Result<_, _>>()?,
                _ => return Err(format!("{name}: a binding is a string or a list of them")),
            };
            if specs.is_empty() {
                return Err(format!(
                    "{name}: an empty list would leave the chord unreachable"
                ));
            }
            let bindings = specs
                .into_iter()
                .map(Binding::parse)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("{name}: {e}"))?;
            if let Some(slot) = keys.iter_mut().find(|(a, _)| *a == action) {
                slot.1 = bindings;
            }
        }
        // One key cannot do two things: the one that lost would be unreachable
        // without anybody noticing.
        for (i, (action, bindings)) in keys.iter().enumerate() {
            for binding in bindings {
                for (other, others) in &keys[i + 1..] {
                    if others.contains(binding) {
                        return Err(format!(
                            "{} is bound to both {} and {}",
                            binding.label(),
                            action.name(),
                            other.name()
                        ));
                    }
                }
            }
        }
        Ok(Self::from(keys))
    }

    /// The file somebody's own bindings are read from.
    pub fn path() -> Option<PathBuf> {
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        };
        Some(base.join("ostraka").join("keys.toml"))
    }

    /// The bindings for this person: their file where there is one, the
    /// defaults where there is not.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::defaults());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::defaults()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn action(&self, event: &KeyEvent) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, bindings)| bindings.iter().any(|b| b.matches(event)))
            .map(|(action, _)| *action)
    }

    fn label(&self, action: Action) -> &str {
        self.labels
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, l)| l.as_str())
            .unwrap_or_default()
    }

    fn uses_command(&self) -> bool {
        self.bindings
            .iter()
            .flat_map(|(_, b)| b)
            .any(|b| b.modifiers.contains(KeyModifiers::SUPER))
    }
}

static KEYS: OnceLock<Keys> = OnceLock::new();
static PUSHED: AtomicBool = AtomicBool::new(false);

/// Sets the bindings this process uses. Only the first call counts: the browser
/// reads the file once, before it draws anything.
pub fn install(keys: Keys) {
    let _ = KEYS.set(keys);
}

fn keys() -> &'static Keys {
    KEYS.get_or_init(Keys::defaults)
}

/// The chord a key event is, if it is one.
pub fn action(event: &KeyEvent) -> Option<Action> {
    keys().action(event)
}

/// A chord as the screen names it.
pub fn label(action: Action) -> &'static str {
    keys().label(action)
}

/// The two pane chords, as one row of the keys dialog names them.
pub fn panes_label() -> &'static str {
    &keys().panes
}

/// Asks the terminal to report command as a modifier, where a binding uses it.
///
/// Without the kitty keyboard protocol a terminal keeps command for itself and
/// the browser never hears it. One that does not speak the protocol ignores the
/// request, and control still reaches every chord.
pub fn enter() {
    if !keys().uses_command() {
        return;
    }
    use ratatui::crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    if ratatui::crossterm::execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok()
    {
        PUSHED.store(true, Ordering::SeqCst);
    }
}

/// Gives back what [`enter`] asked for. A shell left in the protocol reads
/// every key as an escape sequence.
pub fn leave() {
    if PUSHED.swap(false, Ordering::SeqCst) {
        use ratatui::crossterm::event::PopKeyboardEnhancementFlags;
        let _ = ratatui::crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(modifiers: KeyModifiers, c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), modifiers)
    }

    #[test]
    fn control_reaches_every_chord_on_every_platform() {
        let keys = Keys::defaults();
        for action in Action::ALL {
            let event = press(KeyModifiers::CONTROL, action.default_key());
            assert_eq!(keys.action(&event), Some(action), "{action:?}");
            assert_eq!(keys.label(action), format!("ctrl-{}", action.default_key()));
        }
        assert_eq!(keys.action(&press(KeyModifiers::NONE, 'x')), None);
        assert_eq!(keys.action(&press(KeyModifiers::CONTROL, 'c')), None);
    }

    #[test]
    fn command_also_reaches_them_on_macos_and_nowhere_else() {
        let keys = Keys::defaults();
        let command = keys.action(&press(KeyModifiers::SUPER, 'x'));
        assert_eq!(
            command.is_some(),
            cfg!(target_os = "macos"),
            "command-x reached the leader on the wrong platform"
        );
        // Holding both is neither binding, rather than whichever sorts first.
        assert_eq!(
            keys.action(&press(KeyModifiers::CONTROL | KeyModifiers::SUPER, 'x')),
            None
        );
    }

    #[test]
    fn a_file_rebinds_a_chord_and_the_screen_names_the_new_key() {
        let keys =
            Keys::parse("leader = \"alt-x\"\ncommands = [\"f2\", \"ctrl-p\"]\n").expect("keys");
        assert_eq!(
            keys.action(&press(KeyModifiers::ALT, 'x')),
            Some(Action::Leader)
        );
        assert_eq!(
            keys.action(&press(KeyModifiers::CONTROL, 'x')),
            None,
            "the default was kept"
        );
        assert_eq!(keys.label(Action::Leader), "alt-x");
        assert_eq!(
            keys.action(&KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Action::Commands)
        );
        assert_eq!(keys.label(Action::Commands), "f2");
        // A chord the file does not mention keeps its defaults.
        assert_eq!(
            keys.action(&press(KeyModifiers::CONTROL, 't')),
            Some(Action::NewPane)
        );
        assert_eq!(keys.label(Action::NextPane), "ctrl-]");
    }

    #[test]
    fn a_binding_that_would_take_a_key_the_browser_needs_is_refused() {
        for (text, says) in [
            ("leader = \"x\"", "holds no modifier"),
            ("leader = \"ctrl-c\"", "ctrl-c leaves"),
            ("new_pane = \"ctrl-j\"", "new line"),
            ("leader = \"hyper-x\"", "not a modifier"),
            ("leader = \"ctrl-xy\"", "not one key"),
            ("leader = []", "unreachable"),
            ("lead = \"ctrl-x\"", "not a chord"),
            ("leader = \"ctrl-t\"", "bound to both"),
        ] {
            let refused = Keys::parse(text)
                .err()
                .unwrap_or_else(|| panic!("{text} was accepted"));
            assert!(refused.contains(says), "{text}: {refused}");
        }
    }

    #[test]
    fn a_minus_key_and_shift_are_read_and_shift_does_not_get_in_the_way() {
        let minus = Binding::parse("ctrl--").expect("minus");
        assert_eq!(minus.label(), "ctrl--");
        let keys = Keys::parse("next_pane = \"ctrl-shift-]\"").expect("keys");
        assert_eq!(
            keys.action(&press(KeyModifiers::CONTROL | KeyModifiers::SHIFT, ']')),
            Some(Action::NextPane)
        );
    }
}
