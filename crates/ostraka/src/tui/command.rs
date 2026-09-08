//! What the browser can be asked to do, as data rather than as a match arm
//! scattered through the key handler.
//!
//! One list, and four things read it: the command palette lists it, the keys
//! dialog documents it, the leader key dispatches through it, and the status
//! line spells it out while a chord is half-finished. A command added here
//! appears in all four, which is the only way a screen with several ways to
//! reach the same action stays honest about what it can do.

/// What the browser is in the middle of, which decides what makes sense to
/// offer.
///
/// Offering an action that cannot work is worse than not offering it: someone
/// picks it, nothing happens, and now they do not trust the list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Situation {
    /// This directory cannot be run until something is written into it.
    pub unconfigured: bool,
    /// A run is going. There is one at a time — running several at once is the
    /// parallel-execution question, and it is not answered yet.
    pub running: bool,
}

/// Everything the browser does, named the way a person would ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NewRun,
    Stop,
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
    pub const ALL: [Command; 9] = [
        Command::NewRun,
        Command::Stop,
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
            Command::NewRun => "new run",
            Command::Stop => "stop the run",
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
            Command::NewRun => "n",
            Command::Stop => "s",
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
            Command::NewRun => 'n',
            Command::Stop => 's',
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
            Command::NewRun => "write a task and watch it run",
            Command::Stop => "ask the running agent to stop",
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
    pub fn offered(situation: Situation) -> Vec<Command> {
        Command::ALL
            .into_iter()
            .filter(|command| match command {
                // The whole screen in a directory that is not a project yet,
                // and a no-op in one that is.
                Command::Setup => situation.unconfigured,
                // Nothing to stop until something is going.
                Command::Stop => situation.running,
                // Not while one is going, and not where none can be: a run
                // needs a config and a profile, which is what the opening
                // screen is offering to write.
                Command::NewRun => !situation.running && !situation.unconfigured,
                _ => true,
            })
            .collect()
    }

    /// The offered commands matching what has been typed into the palette.
    pub fn matching(query: &str, situation: Situation) -> Vec<Command> {
        let needle = query.trim().to_lowercase();
        Command::offered(situation)
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

    const IDLE: Situation = Situation {
        unconfigured: false,
        running: false,
    };

    #[test]
    fn setup_is_offered_only_where_there_is_something_to_set_up() {
        let unconfigured = Situation {
            unconfigured: true,
            ..IDLE
        };
        assert!(Command::offered(unconfigured).contains(&Command::Setup));
        assert!(!Command::offered(IDLE).contains(&Command::Setup));
    }

    #[test]
    fn a_run_can_be_stopped_only_while_there_is_one() {
        let running = Situation {
            running: true,
            ..IDLE
        };
        assert!(Command::offered(running).contains(&Command::Stop));
        assert!(!Command::offered(IDLE).contains(&Command::Stop));
    }

    #[test]
    fn a_second_run_is_not_offered_while_the_first_is_going() {
        // One at a time. Several at once is the parallel-execution question,
        // and an offer the browser cannot honour is worse than no offer.
        let running = Situation {
            running: true,
            ..IDLE
        };
        assert!(!Command::offered(running).contains(&Command::NewRun));
        assert!(Command::offered(IDLE).contains(&Command::NewRun));
    }

    #[test]
    fn a_run_is_not_offered_in_a_directory_that_cannot_run_one() {
        let unconfigured = Situation {
            unconfigured: true,
            ..IDLE
        };
        assert!(!Command::offered(unconfigured).contains(&Command::NewRun));
    }

    #[test]
    fn the_palette_matches_on_what_a_command_does_not_only_on_its_name() {
        // Someone who wants to promote a run may well type "branch", which is
        // the word for what they want and appears nowhere in the name.
        let found = Command::matching("branch", IDLE);
        assert_eq!(found, vec![Command::Promote]);
    }

    #[test]
    fn an_empty_query_offers_everything_that_makes_sense() {
        assert_eq!(Command::matching("", IDLE).len(), Command::ALL.len() - 2);
    }

    #[test]
    fn every_command_has_a_leader_letter_and_a_key_of_its_own() {
        // Two commands on one letter would make the leader ambiguous, and the
        // one that lost would be unreachable without anybody noticing.
        let mut letters: Vec<char> = Command::ALL.iter().map(|c| c.leader()).collect();
        let mut keys: Vec<&str> = Command::ALL.iter().map(|c| c.key()).collect();
        for (what, count) in [("leader", letters.len()), ("key", keys.len())] {
            let unique = if what == "leader" {
                letters.sort_unstable();
                letters.dedup();
                letters.len()
            } else {
                keys.sort_unstable();
                keys.dedup();
                keys.len()
            };
            assert_eq!(unique, count, "two commands share a {what}");
        }
    }
}
