//! What could run here, as opposed to what has been configured to.
//!
//! A workspace answers "which adapters do I have" by reading `adapters/`. That
//! is the right answer for a run — a profile is the unit of "how this agent is
//! invoked", and inventing one at runtime would make a run depend on what
//! happened to be installed rather than on what the workspace agreed to.
//!
//! It is the wrong answer for a person who cannot get started. A directory with
//! no `adapters/` reports that it has no `adapters/`, which they can see; what
//! they cannot see is that three of the CLIs Ostraka knows how to drive are
//! already on their PATH. The same gap opens on a workspace whose profiles were
//! trimmed, and on one where every configured profile names a CLI that is not
//! installed — the state in which routing fails and there is apparently nothing
//! to choose between.
//!
//! Candidates come from the profiles this binary already ships, the ones
//! `ostraka init` writes. No vendor name is introduced here that was not
//! already data in `templates/`.

use ostraka_adapter::process::ProcessAdapter;
use ostraka_adapter::{Availability, Profile, VendorAdapter};

/// A profile this binary can write, and whether its CLI answers here.
#[derive(Debug)]
pub struct Found {
    pub id: String,
    pub availability: Availability,
}

impl Found {
    pub fn ready(&self) -> bool {
        self.availability.is_ready()
    }
}

/// Every shipped profile whose id is not among `configured`, probed.
///
/// Probing costs a process launch each, so this is for the commands that are
/// already reporting — `adapters`, and the point at which a run has failed to
/// route — rather than for the path a successful run takes.
pub fn unconfigured(configured: &[String]) -> Vec<Found> {
    let mut found: Vec<Found> = crate::init::TEMPLATES
        .iter()
        .filter_map(|(name, text)| {
            let id = name.strip_suffix(".toml").unwrap_or(name);
            if configured.iter().any(|c| c == id) {
                return None;
            }
            let profile = Profile::parse(text).ok()?;
            let adapter = ProcessAdapter::new(profile);
            Some(Found {
                id: adapter.id().to_string(),
                availability: adapter.probe(),
            })
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// What to say when a workspace has nothing usable and something is installed.
///
/// One sentence, and it names the command that writes the profiles rather than
/// describing it: the gap this closes is between "there is nothing here" and
/// "there is something here and one command away".
pub fn suggestion(found: &[Found]) -> Option<String> {
    let ready: Vec<&str> = found
        .iter()
        .filter(|f| f.ready())
        .map(|f| f.id.as_str())
        .collect();
    if ready.is_empty() {
        return None;
    }
    Some(format!(
        "found on PATH but not configured here: {}. `ostraka init` writes a profile for each",
        ready.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_already_configured_is_not_offered_again() {
        // The listing is about the gap. A profile the workspace has is not part
        // of it, whether or not its CLI is installed.
        let all: Vec<String> = crate::init::TEMPLATES
            .iter()
            .map(|(name, _)| name.strip_suffix(".toml").unwrap_or(name).to_string())
            .collect();
        assert!(
            unconfigured(&all).is_empty(),
            "a fully configured workspace was offered profiles it already has"
        );
        assert_eq!(
            unconfigured(&[]).len(),
            crate::init::TEMPLATES.len(),
            "an empty workspace was not offered everything this binary ships"
        );
    }

    #[test]
    fn nothing_is_suggested_when_nothing_answers() {
        // The sentence exists to say "one command away". Printing it when every
        // candidate is absent would send somebody to `init` to write profiles
        // for CLIs they do not have, which is the opposite of help.
        let absent = vec![Found {
            id: "nowhere".into(),
            availability: Availability::NotFound {
                command: "nowhere".into(),
            },
        }];
        assert!(suggestion(&absent).is_none());
        assert!(suggestion(&[]).is_none());
    }

    #[test]
    fn only_what_answers_is_named() {
        let mixed = vec![
            Found {
                id: "here".into(),
                availability: Availability::Ready {
                    version: Some("1.0".into()),
                },
            },
            Found {
                id: "gone".into(),
                availability: Availability::NotFound {
                    command: "gone".into(),
                },
            },
        ];
        let said = suggestion(&mixed).expect("something answered");
        assert!(said.contains("here"), "{said}");
        assert!(!said.contains("gone"), "{said}");
        assert!(said.contains("ostraka init"), "{said}");
    }
}

/// The error a run fails with when nothing could be routed to.
///
/// A type rather than a sentence, so that the command line can recognise the
/// one case worth offering a way out of without matching on wording somebody
/// will reasonably reword. It carries what routing said, because that is still
/// the accurate account of what was tried.
#[derive(Debug)]
pub struct NoAdapter {
    pub said: String,
    /// The installed profiles this workspace has not configured, already
    /// probed — so that whoever offers them does not probe a second time.
    pub found: Vec<Found>,
}

impl std::fmt::Display for NoAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.said)?;
        if let Some(said) = suggestion(&self.found) {
            write!(f, "\n\n{said}")?;
        }
        Ok(())
    }
}

impl std::error::Error for NoAdapter {}
