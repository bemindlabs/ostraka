//! What a vendor process is allowed to inherit.
//!
//! Isolation relocated configuration directories and suppressed instruction
//! files, and left the environment alone. An adapter profile that ran `env`
//! showed what that meant: an authoring agent inherited its launcher's whole
//! environment, and when the launcher was itself a coding CLI, that included the
//! parent session's id, its IPC socket and its messaging token — a live channel
//! out of a process that is supposed to be a subprocess writing to a worktree.
//!
//! So the environment is built rather than inherited. Everything is dropped
//! except an operating-system baseline, whatever the profile names, and what
//! isolation sets.
//!
//! **No vendor variable appears here.** A CLI that authenticates by environment
//! variable names that variable in its own profile, which is the same rule that
//! keeps product names out of this crate: the runtime favors no vendor, so the
//! runtime cannot know which variable belongs to which one.

use std::collections::BTreeMap;

/// Variables every process needs regardless of what it is.
///
/// Deliberately small and deliberately generic: where a program lives, who it
/// is, how it talks to a terminal, and how it reaches the network. Nothing here
/// identifies a vendor, a session or a machine's owner beyond what a shell would
/// tell any program.
const BASELINE: &[&str] = &[
    // Where things are.
    "HOME",
    "PATH",
    "TMPDIR",
    "SHELL",
    // Who is running.
    "USER",
    "LOGNAME",
    // How output should look.
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    // How text should be interpreted. Dropping these changes how a program
    // reads a file, which is a behaviour change disguised as tidiness.
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    // How the network is reached. Not reproducible between machines, and a
    // subprocess that cannot reach its own API is not isolated, it is broken.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    // Windows cannot start a process without these.
    "SystemRoot",
    "SystemDrive",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ComSpec",
    "PATHEXT",
    "WINDIR",
    "TEMP",
    "TMP",
    "NUMBER_OF_PROCESSORS",
];

/// The environment a vendor process is started with.
///
/// Layered, later winning: the baseline, then the names the profile asks to
/// inherit, then the values the profile sets outright, then isolation — which
/// is last because it is the structural guarantee and nothing above it should
/// be able to undo it. A profile that sets a variable isolation also sets is
/// refused at parse time rather than silently overruled.
pub fn build(
    inherit: &[String],
    set: &BTreeMap<String, String>,
    isolation: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for name in BASELINE
        .iter()
        .map(|s| (*s).to_string())
        .chain(inherit.iter().cloned())
    {
        // Only what is actually set. Passing an empty value is a different
        // request from not passing one, and some tools read it as a request.
        if let Some(value) = std::env::var_os(&name) {
            env.insert(name, value.to_string_lossy().into_owned());
        }
    }
    env.extend(set.clone());
    env.extend(isolation);
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_is_generic_and_names_no_vendor() {
        // The rule this file exists under. A vendor's variable belongs in that
        // vendor's profile, where adding one is a file rather than a release.
        //
        // Counted first: every assertion here lives inside the loop, so an
        // empty baseline would pass this while meaning the environment is not
        // built at all.
        assert!(!BASELINE.is_empty(), "nothing is passed to a vendor");
        for name in BASELINE {
            let lower = name.to_lowercase();
            for vendor in [
                "anthropic",
                "claude",
                "openai",
                "codex",
                "copilot",
                "github",
                "google",
                "gemini",
                "ollama",
            ] {
                assert!(
                    !lower.contains(vendor),
                    "{name} names a vendor; it belongs in a profile"
                );
            }
        }
    }

    #[test]
    fn a_variable_nobody_asked_for_does_not_reach_the_vendor() {
        let env = build(&[], &BTreeMap::new(), BTreeMap::new());
        assert!(
            !env.contains_key("CLAUDE_CODE_MESSAGING_TOKEN"),
            "the launcher's own session reached the child"
        );
        assert!(!env.contains_key("CARGO_PKG_NAME"));
    }

    #[test]
    fn the_baseline_still_gets_through() {
        let env = build(&[], &BTreeMap::new(), BTreeMap::new());
        // PATH is set in every environment a test can run in; without it the
        // vendor cannot find its own helper binaries.
        assert!(env.contains_key("PATH"), "PATH was dropped");
    }

    #[test]
    fn a_profile_can_ask_for_what_its_vendor_needs() {
        // This is the escape hatch, and it is per profile on purpose: the
        // runtime cannot know which variable belongs to which vendor.
        let name = "OSTRAKA_TEST_INHERITED";
        // SAFETY: a name no other test uses, set and read in the same test.
        unsafe { std::env::set_var(name, "yes") };
        let env = build(&[name.to_string()], &BTreeMap::new(), BTreeMap::new());
        assert_eq!(env.get(name).map(String::as_str), Some("yes"));
        unsafe { std::env::remove_var(name) };
    }

    #[test]
    fn a_name_that_is_not_set_here_is_not_passed_as_empty() {
        let env = build(
            &["OSTRAKA_DEFINITELY_UNSET".to_string()],
            &BTreeMap::new(),
            BTreeMap::new(),
        );
        assert!(!env.contains_key("OSTRAKA_DEFINITELY_UNSET"));
    }

    #[test]
    fn the_profile_overrides_what_it_inherits_and_isolation_overrides_both() {
        let set = BTreeMap::from([("PATH".to_string(), "/only/this".to_string())]);
        let isolation = BTreeMap::from([("VENDOR_HOME".to_string(), "/run/home".to_string())]);
        let env = build(&[], &set, isolation);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/only/this"));
        assert_eq!(
            env.get("VENDOR_HOME").map(String::as_str),
            Some("/run/home")
        );
    }
}
