//! Choosing who writes and who reviews.
//!
//! Routing is on capability and independence, never on habit. The one rule that
//! is not negotiable: the reviewer must not be the author. The gate enforces
//! that again on identities; enforcing it here as well means a run fails before
//! any work is done rather than after.

use crate::{Error, Result};
use ostraka_adapter::{Profile, VendorAdapter, process::ProcessAdapter};

/// The pair of adapters a run will use.
pub struct Routing {
    pub author: Box<dyn VendorAdapter>,
    pub reviewer: Box<dyn VendorAdapter>,
}

// Trait objects are not Debug, but the only thing worth printing about a
// routing decision is who was picked for what.
impl std::fmt::Debug for Routing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Routing")
            .field("author", &self.author.id())
            .field("reviewer", &self.reviewer.id())
            .finish()
    }
}

/// Picks an author and a reviewer from the available profiles.
///
/// Explicit ids are honoured when given. Otherwise the first two profiles that
/// can actually run here are used, in id order, so the choice is reproducible
/// rather than dependent on directory listing order.
pub fn select(
    profiles: &[Profile],
    author_id: Option<&str>,
    reviewer_id: Option<&str>,
) -> Result<Routing> {
    if profiles.is_empty() {
        return Err(Error::Other("no adapter profiles found".to_string()));
    }

    let find = |id: &str| -> Result<Profile> {
        profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| Error::Other(format!("no adapter profile with id {id:?}")))
    };

    let (author, reviewer) = match (author_id, reviewer_id) {
        (Some(a), Some(r)) => (find(a)?, find(r)?),
        (Some(a), None) => {
            let author = find(a)?;
            let reviewer = first_other(profiles, &author.id)?;
            (author, reviewer)
        }
        (None, Some(r)) => {
            let reviewer = find(r)?;
            let author = first_other(profiles, &reviewer.id)?;
            (author, reviewer)
        }
        (None, None) => {
            let mut sorted: Vec<&Profile> = profiles.iter().collect();
            sorted.sort_by(|a, b| a.id.cmp(&b.id));
            let author = sorted[0].clone();
            let reviewer = first_other(profiles, &author.id)?;
            (author, reviewer)
        }
    };

    if author.id == reviewer.id {
        return Err(Error::Other(format!(
            "author and reviewer are the same adapter ({:?}); a change cannot review itself",
            author.id
        )));
    }

    Ok(Routing {
        author: Box::new(ProcessAdapter::new(author)),
        reviewer: Box::new(ProcessAdapter::new(reviewer)),
    })
}

fn first_other(profiles: &[Profile], exclude: &str) -> Result<Profile> {
    let mut others: Vec<&Profile> = profiles.iter().filter(|p| p.id != exclude).collect();
    others.sort_by(|a, b| a.id.cmp(&b.id));
    others.first().map(|p| (*p).clone()).ok_or_else(|| {
        Error::Other(
            "only one adapter profile is configured, so no independent reviewer exists; \
             add a second profile in adapters/"
                .to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str) -> Profile {
        Profile::parse(&format!(
            r#"
            id = "{id}"
            command = "true"
            args = ["{{{{prompt}}}}"]
            "#
        ))
        .expect("valid")
    }

    #[test]
    fn a_lone_adapter_cannot_review_itself() {
        let err = select(&[profile("solo")], None, None).expect_err("must refuse");
        assert!(err.to_string().contains("no independent reviewer"));
    }

    #[test]
    fn naming_the_same_adapter_twice_is_refused() {
        let profiles = [profile("a"), profile("b")];
        let err = select(&profiles, Some("a"), Some("a")).expect_err("must refuse");
        assert!(err.to_string().contains("cannot review itself"));
    }

    #[test]
    fn selection_is_deterministic_not_listing_order() {
        let forward = select(&[profile("b"), profile("a")], None, None).expect("routes");
        let reverse = select(&[profile("a"), profile("b")], None, None).expect("routes");
        assert_eq!(forward.author.id(), "a");
        assert_eq!(forward.author.id(), reverse.author.id());
        assert_eq!(forward.reviewer.id(), "b");
    }

    #[test]
    fn an_unknown_id_is_reported_by_name() {
        let err = select(&[profile("a")], Some("nope"), None).expect_err("must refuse");
        assert!(err.to_string().contains("nope"));
    }
}
