//! `ostraka init` — make a directory into a project Ostraka can run in.
//!
//! Two files and a listing: `ostraka.toml` says how this project is verified,
//! `adapters/` says which CLIs may run, and `.gitignore` keeps the working
//! evidence out of history. Writing them by hand is what everyone did until
//! now, including this project's own scratch repositories.
//!
//! Nothing is overwritten. A plan says what exists and what is missing, and
//! applying it writes only the missing part — so running it twice is safe, and
//! running it in a half-configured project completes it rather than resetting
//! it.
//!
//! The adapter templates are the profiles this repository ships, embedded so a
//! binary installed by `curl` carries them. They are copies of `adapters/*.toml`
//! and `check-hygiene.sh` fails if the two drift.

use std::path::{Path, PathBuf};

/// What kind of project this is, as far as the files on disk say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rust,
    Node,
    Python,
    /// Nothing recognisable. The gate written is one that refuses, because a
    /// gate that passes everything is worse than one that fails loudly.
    Unknown,
}

impl Kind {
    /// What kind of project a workspace is for.
    ///
    /// Read from the repository it holds rather than from the workspace, which
    /// has no source in it. A workspace with nothing cloned into it yet, or
    /// with several that disagree, gets the gate that fails on purpose — which
    /// is the right answer to "I cannot tell".
    pub fn of_workspace(root: &Path) -> Self {
        let repositories = root.join("repositories");
        let mut kinds: Vec<Kind> = std::fs::read_dir(&repositories)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .map(|p| Self::detect(&p))
            .collect();
        kinds.dedup();
        match kinds.as_slice() {
            [one] => *one,
            _ => Self::Unknown,
        }
    }

    pub fn detect(project: &Path) -> Self {
        if project.join("Cargo.toml").is_file() {
            Self::Rust
        } else if project.join("package.json").is_file() {
            Self::Node
        } else if project.join("pyproject.toml").is_file()
            || project.join("setup.py").is_file()
            || project.join("requirements.txt").is_file()
        {
            Self::Python
        } else {
            Self::Unknown
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Rust => "a Rust project",
            Self::Node => "a Node project",
            Self::Python => "a Python project",
            Self::Unknown => "no recognisable project",
        }
    }
}

/// What will happen to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    /// Only the lines that are missing, added to what is already there.
    Append,
    AlreadyThere,
}

/// What a file is to the project, which decides who has to care it is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Without this there is no workspace. `.ostraka/ostraka.toml`.
    Config,
    /// A directory the layout needs, with nothing in it yet.
    Place,
    /// One vendor profile. A project needs at least one; which one is a choice.
    Profile,
    /// Keeps a run's working evidence out of history. Its absence is untidy,
    /// not broken.
    Ignore,
}

#[derive(Debug, Clone)]
pub struct Planned {
    pub path: PathBuf,
    pub contents: String,
    pub action: Action,
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub project: PathBuf,
    pub kind: Kind,
    pub files: Vec<Planned>,
}

impl Plan {
    /// True when there is nothing left to do. What `init` asks.
    pub fn complete(&self) -> bool {
        self.files.iter().all(|f| f.action == Action::AlreadyThere)
    }

    /// True when this directory can be run as it stands.
    ///
    /// Deliberately weaker than `complete`, and it is the question the browser
    /// asks. A project with a config and a profile runs, whether or not its
    /// `.gitignore` has picked up the two lines `init` also offers — and
    /// answering "this directory is not an Ostraka project yet" across a
    /// screen that has runs to show gets it plainly wrong. This repository's
    /// own `.gitignore` names four paths under `.ostraka/` rather than the
    /// directory, which is how that was found.
    pub fn runnable(&self) -> bool {
        self.files
            .iter()
            .any(|f| f.role == Role::Config && f.action == Action::AlreadyThere)
            && has_a_profile(&self.project)
    }
}

/// The profiles this repository ships, as `init` will write them.
///
/// Kept here rather than read from disk because the binary is installed on its
/// own; a profile someone has to fetch separately is a profile they will not
/// have.
pub const TEMPLATES: [(&str, &str); 6] = [
    ("agy.toml", include_str!("../templates/agy.toml")),
    (
        "claude-code.toml",
        include_str!("../templates/claude-code.toml"),
    ),
    ("codex.toml", include_str!("../templates/codex.toml")),
    (
        "copilot-cli.toml",
        include_str!("../templates/copilot-cli.toml"),
    ),
    ("grok.toml", include_str!("../templates/grok.toml")),
    ("kimi-cli.toml", include_str!("../templates/kimi-cli.toml")),
];

/// Writes one shipped profile into a workspace's `adapters/`.
///
/// The same bytes `init` would have written, from the same list, so a profile
/// added later is the profile that would have been there from the start. The
/// directory is made if it is missing, because the case this exists for is a
/// workspace that has no `adapters/` at all.
///
/// An existing file is left alone: whoever edited it meant to, and overwriting
/// somebody's `[env]` block to fix a routing failure would be a poor trade.
pub fn write_profile(workspace: &crate::workspace::Workspace, id: &str) -> std::io::Result<()> {
    let Some((_, contents)) = TEMPLATES
        .iter()
        .find(|(name, _)| name.strip_suffix(".toml") == Some(id))
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no profile named {id:?} is shipped"),
        ));
    };
    let dir = workspace.adapters();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.toml"));
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents)
}

/// Lines that keep a run's working evidence out of history.
/// Lines that keep a run's working evidence out of history.
///
/// The subdirectories rather than `/.ostraka/` itself, because the
/// configuration and the adapter profiles live in there too and those are
/// meant to be committed — a workspace's agreement about how runs are made is
/// not working evidence.
const IGNORED: [&str; 5] = [
    "/.ostraka/runs/",
    "/.ostraka/worktrees/",
    "/.ostraka/cache/",
    "/.ostraka/secrets/",
    "/.ostraka/vendor-home/",
];

pub fn plan(project: &Path) -> Plan {
    let kind = Kind::of_workspace(project);
    let mut files = vec![planned(
        project,
        ".ostraka/ostraka.toml",
        config_for(kind),
        Role::Config,
    )];

    for (name, contents) in TEMPLATES {
        files.push(planned(
            project,
            &format!(".ostraka/adapters/{name}"),
            contents.to_string(),
            Role::Profile,
        ));
    }
    // The two directories the layout is about. A repository is cloned into the
    // first; the second is linked into every worktree, so what an agent works
    // out along the way survives the run that worked it out.
    //
    // `skills/` is not among them, and that is deliberate. Notes are written by
    // runs, so a run needs somewhere to write before it has anything to say;
    // skills are written by people, and an empty directory waiting for one is
    // clutter that also costs the setup screen a line it was using to explain
    // the gate. A workspace grows one when somebody has a skill to put in it.
    for place in ["repositories", "notes"] {
        files.push(planned(project, place, String::new(), Role::Place));
    }
    files.push(gitignore(project));

    Plan {
        project: project.to_path_buf(),
        kind,
        files,
    }
}

/// Whether `adapters/` holds a profile of any name.
///
/// Asked of the directory rather than of the plan, and the difference is the
/// whole point: a plan lists the profiles `init` would write, so a
/// project that brought its own under other names has every planned profile
/// missing while being perfectly able to run. Reading that as "not set up yet"
/// put the opening screen over a working project — and then a stray keystroke
/// on that screen wrote profiles into it that nobody had asked for.
fn has_a_profile(project: &Path) -> bool {
    std::fs::read_dir(project.join(".ostraka/adapters"))
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|kind| kind == "toml"))
        })
        .unwrap_or(false)
}

fn planned(project: &Path, relative: &str, contents: String, role: Role) -> Planned {
    let path = project.join(relative);
    let action = if path.exists() {
        Action::AlreadyThere
    } else {
        Action::Create
    };
    Planned {
        path,
        contents,
        action,
        role,
    }
}

/// The ignore entries, added to whatever is already in the file.
///
/// Appended rather than written: a project's `.gitignore` is its own, and
/// replacing it to add two lines would be a rude way to set up a tool.
fn gitignore(project: &Path) -> Planned {
    let path = project.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let missing: Vec<&str> = IGNORED
        .iter()
        .copied()
        .filter(|line| !existing.lines().any(|l| l.trim() == *line))
        .collect();

    if missing.is_empty() && path.exists() {
        return Planned {
            path,
            contents: String::new(),
            action: Action::AlreadyThere,
            role: Role::Ignore,
        };
    }

    let block = format!(
        "\n# Ostraka: worktrees an agent works in, and the record of each run.\n\
         # The audit trail that has to survive is in the commits, not here.\n{}\n",
        missing.join("\n")
    );
    Planned {
        path,
        contents: block,
        action: if existing.is_empty() {
            Action::Create
        } else {
            Action::Append
        },
        role: Role::Ignore,
    }
}

/// The gate for a detected project.
///
/// Never empty. An unrecognised project gets a check that fails with an
/// instruction, because a gate with nothing in it would approve anything a
/// reviewer waved through, and silence is the wrong way to learn that.
fn config_for(kind: Kind) -> String {
    let checks = match kind {
        Kind::Rust => vec![
            ("format", "cargo fmt --all -- --check"),
            ("lint", "cargo clippy --all-targets -- -D warnings"),
            ("test", "cargo test --workspace"),
            ("build", "cargo build --workspace"),
        ],
        Kind::Node => vec![("test", "npm test")],
        Kind::Python => vec![("test", "python -m pytest")],
        Kind::Unknown => vec![(
            "declare-your-checks",
            "echo 'ostraka: edit ostraka.toml and declare how this project is verified' >&2; \
             exit 1",
        )],
    };

    let mut out = String::from(
        "# How this project is verified.\n\
         #\n\
         # These commands run inside the worktree an agent worked in, before any\n\
         # reviewer is asked anything. A change that fails one of them never\n\
         # reaches review.\n\n[gate]\nchecks = [\n",
    );
    for (name, cmd) in &checks {
        out.push_str(&format!(
            "    {{ name = \"{name}\", cmd = \"{}\", required = true }},\n",
            cmd.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    out.push_str("]\n\n");
    if kind == Kind::Unknown {
        out.push_str(
            "# The check above fails on purpose. Ostraka could not tell what kind of\n\
             # project this is, and a gate that declares nothing would approve\n\
             # anything a reviewer waved through. Replace it with the commands you\n\
             # would want run before trusting a change you did not write.\n\n",
        );
    }
    out.push_str(
        "# A check that hangs otherwise hangs every run waiting on the gate.\n\
         timeout_secs = 1800\n\n\
         [policy]\n\
         # Wall-clock ceiling for one agent, after which it is stopped. An agent\n\
         # waiting on a stalled connection, or on a prompt nobody will answer,\n\
         # otherwise waits forever and so does the fleet.\n\
         timeout_secs = 900\n\n\
         # Paths an agent may modify. Enforced at the gate, not by the sandbox,\n\
         # and read from git rather than from the agent's own account of itself.\n\
         # allowed_paths = [\"src/\"]\n\
         # enforce_paths = true\n\n\
         [gate.review]\n\
         # Whoever wrote a change cannot be the one who approves it.\n\
         must_differ_from_author = true\n\n\
         [worktree]\n\
         base = \"worktrees\"\n",
    );
    out.push_str(link_for(kind));
    out
}

/// What a worktree needs linked into it for this kind of project.
///
/// A worktree is a fresh checkout, so anything git ignores is absent — and for
/// most ecosystems that is exactly the directory the toolchain needs. Detecting
/// a Node project and writing `npm test` without this hands somebody a gate
/// that cannot run in the environment Ostraka itself builds.
fn link_for(kind: Kind) -> &'static str {
    match kind {
        Kind::Node => concat!(
            "\n",
            "# A worktree is a fresh checkout, so gitignored directories are not in it.\n",
            "# Linked rather than installed per worktree: `npm ci` in each one costs\n",
            "# hundreds of megabytes, and linking is instant and free.\n",
            "link = [\"node_modules\"]\n",
        ),
        Kind::Python => concat!(
            "\n",
            "# A worktree is a fresh checkout, so gitignored directories are not in it.\n",
            "# Uncomment whichever your toolchain needs.\n",
            "# link = [\".venv\"]\n",
        ),
        Kind::Rust | Kind::Unknown => concat!(
            "\n",
            "# A worktree is a fresh checkout, so anything git ignores is absent from it.\n",
            "# List what your toolchain needs; it is linked, not copied.\n",
            "# link = [\"node_modules\"]\n",
            "\n",
            "# Or run a command in the worktree before the agent starts.\n",
            "# setup = \"make deps\"\n",
        ),
    }
}

/// Writes the missing part of a plan. Returns what it wrote.
pub fn apply(plan: &Plan, force: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for file in &plan.files {
        let doing = if force && file.action == Action::AlreadyThere {
            Action::Create
        } else {
            file.action
        };
        match doing {
            Action::AlreadyThere => continue,
            // A place is a directory with nothing in it: `repositories/` for
            // what is worked on, `notes/` for what is worked out.
            Action::Create if file.role == Role::Place => {
                std::fs::create_dir_all(&file.path)?;
            }
            Action::Create => {
                if let Some(parent) = file.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&file.path, &file.contents)?;
            }
            Action::Append => {
                use std::io::Write;
                let mut handle = std::fs::OpenOptions::new().append(true).open(&file.path)?;
                handle.write_all(file.contents.as_bytes())?;
            }
        }
        written.push(file.path.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ostraka-init-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch dir");
        path
    }

    #[test]
    fn a_rust_project_gets_the_checks_a_rust_project_needs() {
        let dir = scratch();
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        std::fs::write(dir.join("repositories/work/Cargo.toml"), "[package]\n").expect("write");
        let plan = plan(&dir);
        assert_eq!(plan.kind, Kind::Rust);
        let config = &plan.files[0].contents;
        assert!(config.contains("cargo clippy"), "{config}");
        assert!(config.contains("must_differ_from_author = true"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unrecognised_project_gets_a_gate_that_refuses() {
        // A gate declaring nothing approves whatever a reviewer waves through.
        // Failing loudly is the only safe thing to generate.
        let dir = scratch();
        let plan = plan(&dir);
        assert_eq!(plan.kind, Kind::Unknown);
        let config = &plan.files[0].contents;
        assert!(config.contains("exit 1"), "{config}");
        assert!(config.contains("fails on purpose"), "{config}");

        // And it must still be a valid, non-empty gate.
        let parsed = ostraka_core::config::Config::parse(config).expect("parses");
        parsed.validate().expect("a generated config is usable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_node_project_gets_a_gate_that_can_actually_run() {
        // The reported defect: init detected Node and wrote `npm test`, which
        // cannot execute in a worktree because node_modules is gitignored and
        // therefore absent. Detecting the ecosystem and then handing over an
        // unrunnable gate is worse than not detecting it.
        let config = ostraka_core::config::Config::parse(&config_for(Kind::Node)).expect("parses");
        assert_eq!(config.worktree.link, ["node_modules"]);
    }

    #[test]
    fn a_project_whose_gitignore_covers_the_ground_differently_is_still_runnable() {
        // This repository's own `.gitignore` names four paths under `.ostraka/`
        // rather than the directory, written before `init` existed. `init` is
        // right that the line it offers is not there, and the browser was
        // wrong to read that as a directory nobody had set up yet.
        let dir = scratch();
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        std::fs::write(dir.join("repositories/work/Cargo.toml"), "[package]\n").expect("write");
        apply(&plan(&dir), false).expect("applies");
        std::fs::write(
            dir.join(".gitignore"),
            "/target\n/worktrees/\n/.ostraka/runs/\n/.ostraka/vendor-home/\n",
        )
        .expect("rewrite");

        let plan = plan(&dir);
        assert!(!plan.complete(), "init should still offer the missing line");
        assert!(plan.runnable(), "a configured project read as unconfigured");
    }

    #[test]
    fn a_directory_with_nothing_in_it_is_neither_complete_nor_runnable() {
        let dir = scratch();
        let plan = plan(&dir);
        assert!(!plan.complete());
        assert!(!plan.runnable());
    }

    #[test]
    fn a_project_that_brought_its_own_adapter_profiles_is_runnable() {
        // The plan wants its own profiles by name. A project with one profile
        // under a name of its own has none of them, and runs perfectly well —
        // reading that as "not set up yet" is how the opening screen ended up
        // over a working project, with a stray key on it writing three
        // profiles nobody had asked for.
        let dir = scratch();
        std::fs::create_dir_all(dir.join(".ostraka")).expect("ostraka");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "# mine\n").expect("write");
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("adapters");
        std::fs::write(dir.join(".ostraka/adapters/mine.toml"), "id = \"mine\"\n").expect("write");

        let plan = plan(&dir);
        assert!(plan.runnable(), "{:?}", plan.files);
        assert!(!plan.complete(), "init should still offer its own");
    }

    #[test]
    fn an_empty_adapters_directory_is_not_a_profile() {
        let dir = scratch();
        std::fs::create_dir_all(dir.join(".ostraka")).expect("ostraka");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "# mine\n").expect("write");
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("adapters");
        assert!(!plan(&dir).runnable());
    }

    #[test]
    fn a_config_with_no_profile_beside_it_is_not_runnable_yet() {
        // Half-configured is the case the opening screen exists for: there is
        // a config, and nothing it can invoke.
        let dir = scratch();
        std::fs::create_dir_all(dir.join(".ostraka")).expect("ostraka");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "# mine\n").expect("write");
        assert!(!plan(&dir).runnable());
    }

    #[test]
    fn a_generated_config_carries_both_ceilings() {
        // The point of writing them is that somebody finds out they exist.
        // `policy.timeout_secs` spent this project's whole life declared and
        // unenforced; a default nobody can see is the next version of that.
        for kind in [Kind::Rust, Kind::Node, Kind::Python, Kind::Unknown] {
            let config = ostraka_core::config::Config::parse(&config_for(kind)).expect("parses");
            assert!(
                config.policy.timeout_secs.is_some(),
                "{kind:?} has no agent ceiling"
            );
            assert!(
                config.gate.timeout_secs.is_some(),
                "{kind:?} has no gate ceiling"
            );
        }
    }

    #[test]
    fn every_generated_config_parses_and_validates() {
        for kind in [Kind::Rust, Kind::Node, Kind::Python, Kind::Unknown] {
            let text = config_for(kind);
            let parsed = ostraka_core::config::Config::parse(&text)
                .unwrap_or_else(|e| panic!("{kind:?} did not parse: {e}\n{text}"));
            parsed
                .validate()
                .unwrap_or_else(|e| panic!("{kind:?} did not validate: {e}"));
        }
    }

    #[test]
    fn the_embedded_profiles_are_the_ones_this_project_ships() {
        for (name, contents) in TEMPLATES {
            let profile = ostraka_adapter::Profile::parse(contents)
                .unwrap_or_else(|e| panic!("{name} is not a usable profile: {e}"));
            assert!(!profile.id.is_empty());
        }
    }

    #[test]
    fn applying_writes_the_files_and_they_are_readable_afterwards() {
        let dir = scratch();
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        std::fs::write(dir.join("repositories/work/Cargo.toml"), "[package]\n").expect("write");
        let written = apply(&plan(&dir), false).expect("applies");
        // The config, one profile per shipped template, notes/ and .gitignore.
        // The fixture made repositories/ to put a Cargo.toml in, and a place
        // that is already there is left alone, so it is not among them.
        assert_eq!(written.len(), TEMPLATES.len() + 3, "{written:?}");
        assert!(dir.join(".ostraka/ostraka.toml").is_file());
        assert!(dir.join(".ostraka/adapters/codex.toml").is_file());
        assert!(dir.join(".gitignore").is_file());
        assert!(dir.join("notes").is_dir(), "notes were not made");
        assert!(dir.join("repositories").is_dir());

        let text = std::fs::read_to_string(dir.join(".ostraka/ostraka.toml")).expect("reads");
        ostraka_core::config::Config::parse(&text)
            .expect("parses")
            .validate()
            .expect("validates");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_it_twice_changes_nothing_the_second_time() {
        let dir = scratch();
        apply(&plan(&dir), false).expect("applies");
        let second = plan(&dir);
        assert!(second.complete(), "{:?}", second.files);
        assert!(apply(&second, false).expect("applies").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_half_configured_project_is_completed_rather_than_reset() {
        // Someone's hand-written ostraka.toml is theirs. Init fills the gaps.
        let dir = scratch();
        std::fs::create_dir_all(dir.join(".ostraka")).expect("ostraka");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "# mine\n").expect("write");
        let written = apply(&plan(&dir), false).expect("applies");
        assert!(!written.iter().any(|p| p.ends_with("ostraka.toml")));
        assert_eq!(
            std::fs::read_to_string(dir.join(".ostraka/ostraka.toml")).expect("reads"),
            "# mine\n"
        );
        assert!(dir.join(".ostraka/adapters/codex.toml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_gitignore_is_appended_to_not_replaced() {
        let dir = scratch();
        std::fs::write(dir.join(".gitignore"), "/target\nnode_modules/\n").expect("write");
        apply(&plan(&dir), false).expect("applies");
        let text = std::fs::read_to_string(dir.join(".gitignore")).expect("reads");
        assert!(text.starts_with("/target\nnode_modules/\n"), "{text}");
        assert!(text.contains("/.ostraka/"), "{text}");
        assert!(text.contains("/worktrees/"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gitignore_that_already_covers_it_is_left_alone() {
        let dir = scratch();
        std::fs::write(dir.join(".gitignore"), "/.ostraka/runs/\n/.ostraka/worktrees/\n/.ostraka/cache/\n/.ostraka/secrets/\n/.ostraka/vendor-home/\n").expect("write");
        let plan = plan(&dir);
        let ignore = plan
            .files
            .iter()
            .find(|f| f.path.ends_with(".gitignore"))
            .expect("planned");
        assert_eq!(ignore.action, Action::AlreadyThere);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
