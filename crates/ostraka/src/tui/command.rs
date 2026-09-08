//! What the browser can be asked to do, as data rather than as a match arm
//! scattered through the key handler.
//!
//! One list, and three things read it: the command palette lists it, the keys
//! dialog documents it, and the leader key dispatches through it. A command
//! added here appears in all three, which is the only way a screen with three
//! ways to reach the same action stays honest about what it can do.

/// Everything the browser does, named the way a person would ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Filter,
    NextPane,
    Promote,
    Reload,
    Setup,
    Keys,
    Quit,
}

impl Command {
    /// In the order the palette offers them: what someone reaches for most,
    /// first.
    pub const ALL: [Command; 7] = [
        Command::Filter,
        Command::NextPane,
        Command::Promote,
        Command::Reload,
        Command::Setup,
        Command::Keys,
        Command::Quit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Filter => "filter runs",
            Command::NextPane => "next pane",
            Command::Promote => "promote run",
            Command::Reload => "reload runs",
            Command::Setup => "set up this directory",
            Command::Keys => "keys",
            Command::Quit => "quit",
        }
    }

    /// The key that does the same thing without opening anything.
    pub fn key(self) -> &'static str {
        match self {
            Command::Filter => "/",
            Command::NextPane => "tab",
            Command::Promote => "p",
            Command::Reload => "r",
            Command::Setup => "i",
            Command::Keys => "?",
            Command::Quit => "q",
        }
    }

    /// The letter that follows the leader.
    ///
    /// Deliberately the same letter as the bare key where there is one: a
    /// leader that renamed every action would be a second set of keys to learn
    /// rather than a way to reach the ones that exist.
    pub fn leader(self) -> char {
        match self {
            Command::Filter => '/',
            Command::NextPane => 't',
            Command::Promote => 'p',
            Command::Reload => 'r',
            Command::Setup => 'i',
            Command::Keys => 'h',
            Command::Quit => 'q',
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Command::Filter => "narrow the list by task, id or outcome",
            Command::NextPane => "checks, then events, then the diff",
            Command::Promote => "give an approved run a branch; merges nothing",
            Command::Reload => "read the run records again",
            Command::Setup => "write the files a project needs to be run",
            Command::Keys => "every key this screen answers to",
            Command::Quit => "leave the browser",
        }
    }

    /// The commands worth offering here and now.
    ///
    /// Setup is the one that comes and goes: it is the whole screen in a
    /// directory that is not a project yet, and does nothing at all in one that
    /// is, so offering it there would be offering a no-op.
    pub fn offered(setup_pending: bool) -> Vec<Command> {
        Command::ALL
            .into_iter()
            .filter(|c| *c != Command::Setup || setup_pending)
            .collect()
    }

    /// The offered commands matching what has been typed into the palette.
    pub fn matching(query: &str, setup_pending: bool) -> Vec<Command> {
        let needle = query.trim().to_lowercase();
        Command::offered(setup_pending)
            .into_iter()
            .filter(|c| {
                needle.is_empty()
                    || c.name().to_lowercase().contains(&needle)
                    || c.about().to_lowercase().contains(&needle)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_is_offered_only_where_there_is_something_to_set_up() {
        assert!(Command::offered(true).contains(&Command::Setup));
        assert!(!Command::offered(false).contains(&Command::Setup));
    }

    #[test]
    fn the_palette_matches_on_what_a_command_does_not_only_on_its_name() {
        // Someone who wants to promote a run may well type "branch", which is
        // the word for what they want and appears nowhere in the name.
        let found = Command::matching("branch", false);
        assert_eq!(found, vec![Command::Promote]);
    }

    #[test]
    fn an_empty_query_offers_everything_rather_than_nothing() {
        assert_eq!(Command::matching("", true).len(), Command::ALL.len());
    }

    #[test]
    fn every_command_has_a_leader_letter_of_its_own() {
        // Two commands on one letter would make the leader ambiguous, and the
        // one that lost would be unreachable without anybody noticing.
        let mut letters: Vec<char> = Command::ALL.iter().map(|c| c.leader()).collect();
        letters.sort_unstable();
        let count = letters.len();
        letters.dedup();
        assert_eq!(letters.len(), count, "two commands share a leader letter");
    }
}
