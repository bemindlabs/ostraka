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
    /// A run is going in the pane in front. Other panes may have runs of their
    /// own: each run has a stop of its own, so a run in one pane is no reason to
    /// refuse a run in another.
    pub running: bool,
    /// How many lines of work are open.
    pub panes: usize,
    /// The panes are drawn side by side, which is when a width means anything.
    pub split: bool,
    /// Something outside the project's own configuration is in the way, and
    /// the browser knows the steps out of it.
    pub blocked: bool,
    /// Text is selected in a pane. Copying it is the one command that needs
    /// something to have been done with the mouse first.
    pub selected: bool,
    /// A question has been answered here, and can be run without retyping it.
    pub ready: bool,
}

/// Everything the browser does, named the way a person would ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NewRun,
    Go,
    Stop,
    Fix,
    Runs,
    Repos,
    Agents,
    Settings,
    Mode,
    Models,
    Fresh,
    NewPane,
    NextPane,
    ClosePane,
    MovePaneLeft,
    MovePaneRight,
    WiderPane,
    NarrowerPane,
    EvenPanes,
    NextDetail,
    Copy,
    Promote,
    Prune,
    Reload,
    Setup,
    Keys,
    Quit,
}

impl Command {
    /// In the order the palette offers them: what someone reaches for most,
    /// first.
    pub const ALL: [Command; 27] = [
        Command::NewRun,
        Command::Go,
        Command::Stop,
        Command::Fix,
        Command::Runs,
        Command::Repos,
        Command::Agents,
        Command::Settings,
        Command::Mode,
        Command::Models,
        Command::Fresh,
        Command::NewPane,
        Command::NextPane,
        Command::ClosePane,
        Command::MovePaneLeft,
        Command::MovePaneRight,
        Command::WiderPane,
        Command::NarrowerPane,
        Command::EvenPanes,
        Command::NextDetail,
        Command::Copy,
        Command::Promote,
        Command::Prune,
        Command::Reload,
        Command::Setup,
        Command::Keys,
        Command::Quit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::NewRun => "write a task",
            Command::Go => "do what was just asked",
            Command::Stop => "stop the run",
            Command::Fix => "fix what is in the way",
            Command::Runs => "runs",
            Command::Repos => "repositories",
            Command::Agents => "agents",
            Command::Settings => "settings",
            Command::Mode => "switch mode",
            Command::Models => "models",
            Command::Fresh => "start a fresh thread",
            Command::NewPane => "new pane",
            Command::NextPane => "next pane",
            Command::ClosePane => "close this pane",
            Command::MovePaneLeft => "move pane left",
            Command::MovePaneRight => "move pane right",
            Command::WiderPane => "widen this pane",
            Command::NarrowerPane => "narrow this pane",
            Command::EvenPanes => "even out the panes",
            Command::NextDetail => "next section",
            Command::Copy => "copy what is selected",
            Command::Promote => "promote run",
            Command::Prune => "prune worktrees",
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
            Command::Go => "e",
            Command::Stop => "s",
            Command::Fix => "x",
            Command::Runs => "l",
            Command::Repos => "w",
            Command::Agents => "a",
            Command::Settings => ",",
            Command::Mode => "m",
            Command::Models => "o",
            Command::Fresh => "f",
            Command::NewPane => "t",
            Command::NextPane => "]",
            Command::ClosePane => "X",
            Command::MovePaneLeft => "<",
            Command::MovePaneRight => ">",
            Command::WiderPane => "+",
            Command::NarrowerPane => "-",
            Command::EvenPanes => "=",
            Command::NextDetail => "tab",
            Command::Copy => "y",
            Command::Promote => "p",
            Command::Prune => "u",
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
            // Not `g`, which is the top of the runs list, and not `d`, which
            // is the leader for the next section. `e` for execute: free as a
            // bare key and as a leader, so the two agree.
            Command::Go => 'e',
            Command::Stop => 's',
            Command::Fix => 'x',
            Command::Runs => 'l',
            Command::Repos => 'w',
            Command::Agents => 'a',
            Command::Settings => ',',
            Command::Mode => 'm',
            Command::Models => 'o',
            Command::Fresh => 'f',
            Command::NewPane => 't',
            Command::NextPane => ']',
            Command::ClosePane => 'c',
            // The shapes of what they do, which is easier to find again than a
            // letter chosen because it was free.
            Command::MovePaneLeft => '<',
            Command::MovePaneRight => '>',
            Command::WiderPane => '+',
            Command::NarrowerPane => '-',
            Command::EvenPanes => '=',
            Command::NextDetail => 'd',
            // What every editor since vi has called yanking, and `c` is the
            // letter that closes a pane.
            Command::Copy => 'y',
            Command::Promote => 'p',
            // `p` is taken and prune is the second thing on this row; `u` is
            // in the word and free.
            Command::Prune => 'u',
            Command::Reload => 'r',
            Command::Setup => 'i',
            Command::Keys => 'h',
            Command::Quit => 'q',
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Command::NewRun => "say what the agent should do next",
            Command::Go => {
                "run the question just answered as a change: written, gated and reviewed"
            }
            Command::Stop => "ask the running agent to stop",
            Command::Fix => "walk through what is stopping a run from working",
            Command::Runs => "look up a run recorded here",
            Command::Repos => "choose which repository to work in",
            Command::Agents => "choose who writes and who reviews",
            Command::Settings => "what this thread and this project are set to",
            Command::Mode => "ask, plan, loop or auto \u{2014} what enter does next",
            Command::Models => {
                "pick the profile and model that write, from what each profile lists"
            }
            Command::Fresh => "forget the chain; start again from HEAD",
            Command::NewPane => "another line of work, open beside this one",
            Command::NextPane => "move to the next line of work",
            Command::ClosePane => "close this line of work",
            Command::MovePaneLeft => "swap this pane with the one to its left",
            Command::MovePaneRight => "swap this pane with the one to its right",
            Command::WiderPane => "give this pane more of the width",
            Command::NarrowerPane => "give this pane less of the width",
            Command::EvenPanes => "give every pane the same width",
            Command::NextDetail => "checks, then events, then the diff",
            Command::Copy => {
                "put the text selected in this pane on the clipboard, terminal permitting"
            }
            Command::Promote => "give an approved run a branch; merges nothing",
            // No mention of branches here. The dialog says what is kept, at the
            // moment it matters; saying it in a one-line description makes
            // "branch" match this as well as promoting, and somebody typing that
            // word wants the other one.
            Command::Prune => "clear the worktrees finished runs left behind",
            Command::Reload => "read the run records again",
            Command::Setup => "write the files a project needs to be run",
            Command::Keys => "every key this screen answers to",
            Command::Quit => "leave the browser",
        }
    }

    /// The word this command answers to when it is typed rather than pressed.
    ///
    /// Short and unambiguous, and not derived from the name: "write a task"
    /// and "start a fresh thread" are sentences, and a slug taken from a
    /// sentence changes whenever somebody improves the wording.
    pub fn slug(self) -> &'static str {
        match self {
            Command::NewRun => "new",
            Command::Go => "go",
            Command::Stop => "stop",
            Command::Fix => "fix",
            Command::Runs => "runs",
            Command::Repos => "repos",
            Command::Agents => "agents",
            Command::Settings => "settings",
            Command::Mode => "mode",
            Command::Models => "models",
            Command::Fresh => "fresh",
            Command::NewPane => "pane",
            Command::NextPane => "next",
            Command::ClosePane => "close",
            Command::MovePaneLeft => "left",
            Command::MovePaneRight => "right",
            Command::WiderPane => "wider",
            Command::NarrowerPane => "narrower",
            Command::EvenPanes => "even",
            Command::NextDetail => "detail",
            Command::Copy => "copy",
            Command::Promote => "promote",
            Command::Prune => "prune",
            Command::Reload => "reload",
            Command::Setup => "init",
            Command::Keys => "keys",
            Command::Quit => "quit",
        }
    }

    /// The command a typed word names, if any.
    ///
    /// The box takes tasks, so a word only reaches here behind a `/` — which
    /// is what everybody types first anyway, and typing it used to produce a
    /// run whose task was the word "settings".
    pub fn named(word: &str) -> Option<Command> {
        let word = word.trim().trim_start_matches('/').to_lowercase();
        if word == "help" || word == "?" {
            return Some(Command::Keys);
        }
        Command::ALL
            .into_iter()
            .find(|command| command.slug() == word || command.key() == word)
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
                // Choosing between profiles needs profiles to choose between.
                Command::Agents => !situation.unconfigured,
                // Only where there is something the browser knows the way out
                // of. An offer to fix nothing is an offer nobody can take.
                Command::Fix => situation.blocked,
                // Throwing the chain away underneath a run that is standing on
                // it is not something to offer.
                Command::Fresh => !situation.running,
                // Only where there is more than one line of work to move
                // between or close.
                Command::NextPane
                | Command::ClosePane
                | Command::MovePaneLeft
                | Command::MovePaneRight => situation.panes > 1,
                // A width is a share of a screen the panes are sharing. Switched
                // between, each one has all of it.
                Command::WiderPane | Command::NarrowerPane | Command::EvenPanes => situation.split,
                // Listing models needs profiles to list them from.
                Command::Models => !situation.unconfigured,
                // Nothing to copy until something is selected. Offered while
                // it is, so the palette is where somebody finds out this is
                // possible at all.
                Command::Copy => situation.selected,
                // Only once there is an answer to act on, and not while a run is
                // going. Offered while there is, so somebody who asked a
                // question is told, there and then, that they can have it done.
                Command::Go => situation.ready && !situation.running && !situation.unconfigured,
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
        blocked: false,
        panes: 1,
        split: false,
        selected: false,
        ready: false,
    };

    #[test]
    fn going_is_offered_only_once_there_is_an_answer_to_act_on() {
        let ready = Situation {
            ready: true,
            ..IDLE
        };
        assert!(Command::offered(ready).contains(&Command::Go));
        assert!(!Command::offered(IDLE).contains(&Command::Go));
        // Not while a run is going: there is one thread, and it is busy.
        let busy = Situation {
            running: true,
            ..ready
        };
        assert!(!Command::offered(busy).contains(&Command::Go));
    }

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
        // Everything but stopping a run that is not going, setting up a
        // directory that is already set up, and fixing what is not broken.
        // Everything but stopping a run that is not going, setting up a
        // directory that is already set up, fixing what is not broken,
        // moving between, closing or moving panes there is only one of, and
        // sizing panes that are not side by side.
        assert_eq!(Command::matching("", IDLE).len(), Command::ALL.len() - 12);
    }

    #[test]
    fn a_pane_is_sized_only_while_the_panes_share_the_screen() {
        let several = Situation { panes: 2, ..IDLE };
        assert!(Command::offered(several).contains(&Command::MovePaneLeft));
        assert!(!Command::offered(several).contains(&Command::WiderPane));
        let split = Situation {
            split: true,
            ..several
        };
        for command in [
            Command::WiderPane,
            Command::NarrowerPane,
            Command::EvenPanes,
        ] {
            assert!(Command::offered(split).contains(&command), "{command:?}");
        }
    }

    #[test]
    fn fixing_is_offered_only_where_there_is_something_in_the_way() {
        let blocked = Situation {
            blocked: true,
            ..IDLE
        };
        assert!(Command::offered(blocked).contains(&Command::Fix));
        assert!(!Command::offered(IDLE).contains(&Command::Fix));
    }

    #[test]
    fn moving_between_panes_is_offered_only_where_there_are_several() {
        let several = Situation { panes: 3, ..IDLE };
        assert!(Command::offered(several).contains(&Command::NextPane));
        assert!(Command::offered(several).contains(&Command::ClosePane));
        assert!(!Command::offered(IDLE).contains(&Command::NextPane));
        // Opening one is always on offer; there is always room for another.
        assert!(Command::offered(IDLE).contains(&Command::NewPane));
    }

    #[test]
    fn a_thread_is_not_thrown_away_from_under_a_running_run() {
        let running = Situation {
            running: true,
            ..IDLE
        };
        assert!(!Command::offered(running).contains(&Command::Fresh));
        assert!(Command::offered(IDLE).contains(&Command::Fresh));
    }

    #[test]
    fn a_typed_word_reaches_the_command_it_names() {
        assert_eq!(Command::named("/settings"), Some(Command::Settings));
        assert_eq!(Command::named("settings"), Some(Command::Settings));
        assert_eq!(Command::named("/QUIT"), Some(Command::Quit));
        // The keys work too, because somebody who knows `l` will type `/l`.
        assert_eq!(Command::named("/l"), Some(Command::Runs));
        // And the word everyone tries first.
        assert_eq!(Command::named("/help"), Some(Command::Keys));
        assert_eq!(Command::named("/nonsense"), None);
    }

    #[test]
    fn every_command_has_a_slug_of_its_own() {
        let mut slugs: Vec<&str> = Command::ALL.iter().map(|c| c.slug()).collect();
        let count = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), count, "two commands share a slug");
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

#[cfg(test)]
mod copy_tests {
    use super::*;

    const IDLE: Situation = Situation {
        unconfigured: false,
        running: false,
        blocked: false,
        panes: 1,
        split: false,
        selected: false,
        ready: false,
    };

    #[test]
    fn copying_is_offered_only_while_something_is_selected() {
        let selected = Situation {
            selected: true,
            ..IDLE
        };
        assert!(Command::offered(selected).contains(&Command::Copy));
        assert!(!Command::offered(IDLE).contains(&Command::Copy));
    }
}
