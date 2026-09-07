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

#[derive(Debug, Clone)]
pub struct Planned {
    pub path: PathBuf,
    pub contents: String,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub project: PathBuf,
    pub kind: Kind,
    pub files: Vec<Planned>,
}

impl Plan {
    /// True when there is nothing left to do.
    pub fn complete(&self) -> bool {
        self.files.iter().all(|f| f.action == Action::AlreadyThere)
    }
}

/// The three profiles this repository ships, as `init` will write them.
///
/// Kept here rather than read from disk because the binary is installed on its
/// own; a profile someone has to fetch separately is a profile they will not
/// have.
pub const TEMPLATES: [(&str, &str); 3] = [
    (
        "claude-code.toml",
        include_str!("../templates/claude-code.toml"),
    ),
    ("codex.toml", include_str!("../templates/codex.toml")),
    (
        "copilot-cli.toml",
        include_str!("../templates/copilot-cli.toml"),
    ),
];

/// Lines that keep a run's working evidence out of history.
const IGNORED: [&str; 2] = ["/.ostraka/", "/worktrees/"];

pub fn plan(project: &Path) -> Plan {
    let kind = Kind::detect(project);
    let mut files = vec![planned(project, "ostraka.toml", config_for(kind))];

    for (name, contents) in TEMPLATES {
        files.push(planned(
            project,
            &format!("adapters/{name}"),
            contents.to_string(),
        ));
    }
    files.push(gitignore(project));

    Plan {
        project: project.to_path_buf(),
        kind,
        files,
    }
}

fn planned(project: &Path, relative: &str, contents: String) -> Planned {
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
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").expect("write");
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
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").expect("write");
        let written = apply(&plan(&dir), false).expect("applies");
        assert_eq!(written.len(), 5, "{written:?}");
        assert!(dir.join("ostraka.toml").is_file());
        assert!(dir.join("adapters/codex.toml").is_file());
        assert!(dir.join(".gitignore").is_file());

        let text = std::fs::read_to_string(dir.join("ostraka.toml")).expect("reads");
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
        std::fs::write(dir.join("ostraka.toml"), "# mine\n").expect("write");
        let written = apply(&plan(&dir), false).expect("applies");
        assert!(!written.iter().any(|p| p.ends_with("ostraka.toml")));
        assert_eq!(
            std::fs::read_to_string(dir.join("ostraka.toml")).expect("reads"),
            "# mine\n"
        );
        assert!(dir.join("adapters/codex.toml").is_file());
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
        std::fs::write(dir.join(".gitignore"), "/.ostraka/\n/worktrees/\n").expect("write");
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
