//! Choosing who writes and who reviews.
//!
//! Routing is on capability and independence, never on habit. The one rule that
//! is not negotiable: the reviewer must not be the author. The gate enforces
//! that again on identities; enforcing it here as well means a run fails before
//! any work is done rather than after.

use crate::{Error, Result};
use ostraka_adapter::{Availability, Profile, VendorAdapter, process::ProcessAdapter};
use std::path::Path;

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
/// Explicit ids are honoured as given: naming an adapter is a decision, and a
/// decision that turns out to be unrunnable should fail by name rather than be
/// quietly substituted. Everything chosen automatically is drawn only from the
/// profiles that can actually run here, in id order, so the choice is
/// reproducible rather than dependent on directory listing order.
///
/// `timeout` is the ceiling either side gets before it is killed, bound here
/// for the same reason as the rest: what a run is allowed to do is decided when
/// the routing is, not by whatever calls `launch` later.
///
/// `isolation_root` is where a profile that relocates its vendor's home
/// directory may put it. It is bound here, alongside the read-only review
/// invocation, for the same reason: what a run is allowed to read is decided
/// when the routing is, not by whatever calls `launch` later.
pub fn select(
    profiles: &[Profile],
    author_id: Option<&str>,
    reviewer_id: Option<&str>,
    isolation_root: &Path,
    timeout: Option<std::time::Duration>,
) -> Result<Routing> {
    select_until(
        profiles,
        author_id,
        reviewer_id,
        isolation_root,
        timeout,
        &ostraka_adapter::interrupt::Stop::new(),
    )
}

/// [`select`], with both adapters answering to one run's stop.
///
/// Pass the same stop to [`crate::orchestrator::run_task_until`], which uses it
/// for the gate: the adapters get theirs here because this is where they are
/// built, and a run whose author stopped but whose checks did not has not
/// stopped.
pub fn select_until(
    profiles: &[Profile],
    author_id: Option<&str>,
    reviewer_id: Option<&str>,
    isolation_root: &Path,
    timeout: Option<std::time::Duration>,
    stop: &ostraka_adapter::interrupt::Stop,
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
            let reviewer = first_other(profiles, Some(&author), Side::Review)?;
            (author, reviewer)
        }
        (None, Some(r)) => {
            let reviewer = find(r)?;
            let author = first_other(profiles, Some(&reviewer), Side::Author)?;
            (author, reviewer)
        }
        (None, None) => {
            let author = first_other(profiles, None, Side::Author)?;
            let reviewer = first_other(profiles, Some(&author), Side::Review)?;
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
        author: Box::new(
            ProcessAdapter::new(author)
                .isolated_under(isolation_root)
                .within(timeout)
                .stopped_by(stop.clone()),
        ),
        reviewer: Box::new(
            ProcessAdapter::reviewing(reviewer)
                .isolated_under(isolation_root)
                .within(timeout)
                .stopped_by(stop.clone()),
        ),
    })
}

/// Which side of a run an automatic choice is being made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Author,
    Review,
}

/// The profile to pair with another one, or the first to pick when there is no
/// other one yet.
///
/// Availability is asked, not assumed. A profile whose CLI is absent or refuses
/// to answer would otherwise be routed to and fail after a worktree had been
/// created and a task dispatched.
///
/// Among the usable ones, a profile invoking a *different binary* is preferred.
/// Independence is expressed between profiles, so two profiles of one vendor on
/// two models are a legitimate pair — but they share a lineage, a system prompt
/// and a set of blind spots, and picking them over an actually different vendor
/// because their ids happen to sort first would weaken every unattended run.
/// Naming one explicitly still gets it: this orders a choice nobody made.
///
/// When the choice is a reviewer, one thing outranks even that: whether the
/// profile has a review invocation at all. A profile that declares no
/// `review_args` reviews with its author invocation, which can write — and a
/// reviewer that can write can alter the change it is judging. Two shipped
/// profiles are in that position, and routing used to pick one as a reviewer
/// whenever its id sorted first. Posture comes before binary because one is a
/// question of whether the verdict can be trusted and the other of how good it
/// is. It is still an ordering, not a refusal: a workspace whose only other
/// profile has no review invocation gets that profile, and the run refuses on
/// its own if the worktree changes under review.
fn first_other(profiles: &[Profile], exclude: Option<&Profile>, side: Side) -> Result<Profile> {
    let excluded_id = exclude.map(|p| p.id.as_str()).unwrap_or_default();
    let excluded_command = exclude.map(|p| p.command.as_str());
    let mut others: Vec<&Profile> = profiles.iter().filter(|p| p.id != excluded_id).collect();
    others.sort_by(|a, b| {
        // Named for what is compared, not for what it implies. A profile with
        // no review invocation reviews with its author one, which is usually
        // the one that can write — usually, not by definition, so the name says
        // the fact and the doc comment above says why it matters.
        let reviews_with_author_invocation =
            |p: &Profile| side == Side::Review && p.review_args.is_none();
        let same_binary_as_other = |p: &Profile| excluded_command == Some(p.command.as_str());
        reviews_with_author_invocation(a)
            .cmp(&reviews_with_author_invocation(b))
            .then_with(|| same_binary_as_other(a).cmp(&same_binary_as_other(b)))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut unusable: Vec<String> = Vec::new();
    for candidate in &others {
        match ProcessAdapter::new((*candidate).clone()).probe() {
            a if a.is_ready() => return Ok((*candidate).clone()),
            Availability::NotFound { command } => {
                unusable.push(format!("{}: {command} not on PATH", candidate.id));
            }
            Availability::Unusable { reason } => {
                unusable.push(format!("{}: {reason}", candidate.id));
            }
            // `is_ready` covered this arm; kept exhaustive rather than
            // unreachable so a new variant is a compile error, not a silent pass.
            Availability::Ready { .. } => return Ok((*candidate).clone()),
        }
    }

    if others.is_empty() {
        return Err(Error::Other(
            "only one adapter profile is configured, so no independent reviewer exists; \
             add a second profile in adapters/"
                .to_string(),
        ));
    }
    let besides = if excluded_id.is_empty() {
        String::new()
    } else {
        format!(" besides {excluded_id:?}")
    };
    Err(Error::Other(format!(
        "no usable adapter profile{besides}; run `ostraka adapters` for detail. Checked — {}",
        unusable.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Routing never launches anything, so any path will do here.
    fn root() -> &'static Path {
        Path::new("/nonexistent-isolation-root")
    }

    fn command_profile(id: &str, command: &str) -> Profile {
        Profile::parse(&format!(
            r#"
            id = "{id}"
            command = "{command}"
            args = ["{{{{prompt}}}}"]
            "#
        ))
        .expect("valid")
    }

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
        let err = select(&[profile("solo")], None, None, root(), None).expect_err("must refuse");
        assert!(err.to_string().contains("no independent reviewer"));
    }

    #[test]
    fn naming_the_same_adapter_twice_is_refused() {
        let profiles = [profile("a"), profile("b")];
        let err = select(&profiles, Some("a"), Some("a"), root(), None).expect_err("must refuse");
        assert!(err.to_string().contains("cannot review itself"));
    }

    #[test]
    fn selection_is_deterministic_not_listing_order() {
        let forward =
            select(&[profile("b"), profile("a")], None, None, root(), None).expect("routes");
        let reverse =
            select(&[profile("a"), profile("b")], None, None, root(), None).expect("routes");
        assert_eq!(forward.author.id(), "a");
        assert_eq!(forward.author.id(), reverse.author.id());
        assert_eq!(forward.reviewer.id(), "b");
    }

    #[test]
    fn a_profile_whose_cli_is_absent_is_not_routed_to() {
        let missing = Profile::parse(
            r#"
            id = "aaa-missing"
            command = "definitely-not-a-real-binary-xyz"
            args = ["{{prompt}}"]
            "#,
        )
        .expect("valid");
        // "aaa-missing" sorts first, so id order alone would pick it.
        let routing = select(
            &[missing, profile("b"), profile("c")],
            None,
            None,
            root(),
            None,
        )
        .expect("routes");
        assert_eq!(routing.author.id(), "b");
        assert_eq!(routing.reviewer.id(), "c");
    }

    #[test]
    fn an_unrunnable_choice_is_still_honoured_when_named() {
        // Naming an adapter is a decision. Substituting a different one behind
        // the caller's back would make the run record say something untrue.
        let missing = Profile::parse(
            r#"
            id = "missing"
            command = "definitely-not-a-real-binary-xyz"
            args = ["{{prompt}}"]
            "#,
        )
        .expect("valid");
        let routing = select(
            &[missing, profile("b")],
            Some("missing"),
            None,
            root(),
            None,
        )
        .expect("routes");
        assert_eq!(routing.author.id(), "missing");
    }

    #[test]
    fn when_nothing_is_runnable_the_reason_is_named() {
        let missing = Profile::parse(
            r#"
            id = "gone"
            command = "definitely-not-a-real-binary-xyz"
            args = ["{{prompt}}"]
            "#,
        )
        .expect("valid");
        let err = select(&[missing], None, None, root(), None).expect_err("must refuse");
        assert!(err.to_string().contains("definitely-not-a-real-binary-xyz"));
    }

    #[test]
    fn an_unpicked_reviewer_prefers_a_different_binary_over_a_lower_id() {
        // Two profiles of one vendor on two models are a legitimate pair, and
        // the ids here would sort them together ahead of "zz". Chosen for
        // someone rather than by them, the genuinely different vendor wins.
        let same_vendor_a = command_profile("aa-vendor-fast", "true");
        let same_vendor_b = command_profile("ab-vendor-slow", "true");
        let other_vendor = command_profile("zz-other", "echo");

        let routing = select(
            &[same_vendor_a, same_vendor_b, other_vendor],
            Some("aa-vendor-fast"),
            None,
            root(),
            None,
        )
        .expect("routes");
        assert_eq!(routing.reviewer.id(), "zz-other");
    }

    fn reviewing_profile(id: &str) -> Profile {
        Profile::parse(&format!(
            r#"
            id = "{id}"
            command = "true"
            args = ["{{{{prompt}}}}", "--write"]
            review_args = ["{{{{prompt}}}}", "--read-only"]
            "#
        ))
        .expect("valid")
    }

    #[test]
    fn an_unpicked_reviewer_is_one_with_a_review_invocation_first() {
        // `a` sorts first and would have been the reviewer, but it has no
        // review invocation, so it would review with the one that can write.
        let profiles = [profile("a"), reviewing_profile("b"), profile("writer")];
        let routing = select(&profiles, Some("writer"), None, root(), None).expect("routes");
        assert_eq!(routing.reviewer.id(), "b");
    }

    #[test]
    fn posture_outranks_a_different_binary_when_choosing_a_reviewer() {
        // `other` is a different binary from the author and has no review
        // invocation; `same` shares the author's binary and can only read.
        // Whether a verdict can be trusted comes before how good it is.
        let author = command_profile("author", "true");
        let other = command_profile("other", "sh");
        let same = reviewing_profile("same");
        let routing =
            select(&[author, other, same], Some("author"), None, root(), None).expect("routes");
        assert_eq!(routing.reviewer.id(), "same");
    }

    #[test]
    fn posture_is_not_asked_of_an_author() {
        // Only a reviewer needs to be unable to write. Picking an author by
        // whether it can review would push the one read-only profile into the
        // wrong seat.
        let profiles = [profile("a"), reviewing_profile("b")];
        let routing = select(&profiles, None, None, root(), None).expect("routes");
        assert_eq!(routing.author.id(), "a");
        assert_eq!(routing.reviewer.id(), "b");
    }

    #[test]
    fn a_writable_reviewer_is_still_used_when_it_is_the_only_one() {
        // An ordering, not a refusal: the tree check in the orchestrator is what
        // refuses a reviewer that actually writes.
        let profiles = [profile("a"), profile("b")];
        let routing = select(&profiles, Some("a"), None, root(), None).expect("routes");
        assert_eq!(routing.reviewer.id(), "b");
    }

    #[test]
    fn a_named_writable_reviewer_is_honoured() {
        let profiles = [profile("a"), reviewing_profile("b"), profile("c")];
        let routing = select(&profiles, Some("b"), Some("c"), root(), None).expect("routes");
        assert_eq!(routing.reviewer.id(), "c");
    }

    #[test]
    fn one_vendor_on_two_profiles_is_still_a_usable_pair() {
        // The escape hatch has to actually work: with nothing else installed,
        // two profiles of the same binary review each other rather than the run
        // refusing outright.
        let profiles = [
            command_profile("vendor-fast", "true"),
            command_profile("vendor-slow", "true"),
        ];
        let routing = select(&profiles, None, None, root(), None).expect("routes");
        assert_eq!(routing.author.id(), "vendor-fast");
        assert_eq!(routing.reviewer.id(), "vendor-slow");
    }

    #[test]
    fn an_unknown_id_is_reported_by_name() {
        let err =
            select(&[profile("a")], Some("nope"), None, root(), None).expect_err("must refuse");
        assert!(err.to_string().contains("nope"));
    }
}
