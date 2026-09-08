//! A workspace: what Ostraka owns, and the repositories it works on.
//!
//! ```text
//! <workspace>/
//!   .ostraka/
//!     ostraka.toml      how runs are made here
//!     adapters/*.toml   the vendor profiles this machine has
//!     runs/             what happened                     (not committed)
//!     worktrees/        where agents work                 (not committed)
//!   repositories/<name> the repositories being worked on
//!   notes/              what was worked out along the way
//! ```
//!
//! One directory holds everything the runtime owns, so a repository cloned in
//! here is left as its owner left it: no config appears at its root, no
//! worktrees are made inside it, and removing the workspace removes every
//! trace of Ostraka having been used.
//!
//! **A repository may bring its own `ostraka.toml`.** How a project is verified
//! is a property of that project — a workspace holding a Rust repository and a
//! Node one cannot have one gate between them — so a repository's own file
//! wins entirely where there is one, and the workspace's is the answer where
//! there is not.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
use std::path::{Path, PathBuf};

type Loaded<T> = Result<T, Box<dyn std::error::Error>>;

/// One repository the workspace works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The directory name under `repositories/`, which is what a record says
    /// and what `--repository` takes.
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    /// Resolved, and that is not cosmetic.
    ///
    /// The command line defaults to `.`, and a relative worktree path built
    /// from it means two different directories: `git worktree add` resolves it
    /// against the repository it is run in, and everything here resolves it
    /// against the workspace. The agent then works in a checkout git has never
    /// heard of, and the first thing to notice is `git status` failing two
    /// minutes later. Found by running it.
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        }
    }

    /// Everything the runtime owns.
    pub fn ostraka(&self) -> PathBuf {
        self.root.join(".ostraka")
    }

    pub fn config_path(&self) -> PathBuf {
        self.ostraka().join("ostraka.toml")
    }

    pub fn adapters(&self) -> PathBuf {
        self.ostraka().join("adapters")
    }

    /// Where run records go. `RunLog` puts `runs/` under this.
    pub fn records(&self) -> PathBuf {
        self.ostraka()
    }

    pub fn repositories_dir(&self) -> PathBuf {
        self.root.join("repositories")
    }

    pub fn notes(&self) -> PathBuf {
        self.root.join("notes")
    }

    /// The skills directory: procedures this workspace provides to every agent
    /// it runs, whoever the vendor is.
    ///
    /// The counterpart to `notes/` and the opposite direction of travel. Notes
    /// are what runs worked out and wrote down; skills are what people wrote
    /// down for runs to follow. Both belong to the workspace and neither is
    /// part of the change a reviewer judges — and each is linked into a
    /// worktree only where the workspace has one, which is why the pair of
    /// `_if_present` accessors exists and why an author is told about whichever
    /// of them actually arrived.
    ///
    /// A workspace directory rather than a vendor one on purpose. Every CLI
    /// here has some private notion of skills or plugins, kept in a home
    /// directory that isolation deliberately relocates — so a run that depended
    /// on those would give a different answer on a different machine, which is
    /// the thing isolation exists to prevent. A skill the workspace owns is one
    /// every vendor gets and every machine reproduces.
    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// The skills directory, if it is there. Absent is not an error.
    pub fn skills_if_present(&self) -> Option<PathBuf> {
        let skills = self.skills();
        skills.is_dir().then_some(skills)
    }

    /// The notes directory, if it is there. Absent is not an error: a
    /// workspace without one is a workspace nobody has taken notes in.
    pub fn notes_if_present(&self) -> Option<PathBuf> {
        let notes = self.notes();
        notes.is_dir().then_some(notes)
    }

    /// True once this is a workspace rather than a directory.
    pub fn declared(&self) -> bool {
        self.config_path().is_file()
    }

    /// Every repository, by name, in the order a person would read them.
    pub fn repositories(&self) -> Vec<Repository> {
        let Ok(entries) = std::fs::read_dir(self.repositories_dir()) else {
            return Vec::new();
        };
        let mut found: Vec<Repository> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter_map(|path| {
                let name = path.file_name()?.to_string_lossy().into_owned();
                Some(Repository { name, path })
            })
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found
    }

    /// The repository a command means.
    ///
    /// Naming one is only necessary where there is a choice. A workspace with
    /// one repository in it is the common case and should not need saying.
    pub fn repository(&self, named: Option<&str>) -> Loaded<Repository> {
        let found = self.repositories();
        if let Some(name) = named {
            return found.into_iter().find(|r| r.name == name).ok_or_else(|| {
                format!(
                    "no repository {name:?} in {}",
                    self.repositories_dir().display()
                )
                .into()
            });
        }
        match found.len() {
            0 => Err(format!(
                "no repositories in {} — clone what you want worked on into it, \
                 or `ostraka tui` starts one",
                self.repositories_dir().display()
            )
            .into()),
            1 => Ok(found.into_iter().next().expect("one")),
            _ => Err(format!(
                "this workspace has {} repositories; name one with --repository: {}",
                found.len(),
                found
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into()),
        }
    }

    /// Starts a repository here, for work that is not cloned from anywhere.
    ///
    /// A workspace is often opened before there is anything in it, and not
    /// every piece of work begins as somebody else's repository. This is the
    /// half of "nothing to work on yet" that the browser can do something
    /// about: cloning needs a URL only the operator knows, and starting one
    /// needs a name.
    ///
    /// Only `git init`. The commit a worktree needs to branch from is left to
    /// the guided fix, which already asks before it commits and already
    /// reports git's own words when there is no author configured.
    pub fn start_repository(&self, name: &str) -> Loaded<Repository> {
        let name = name.trim();
        // A name is one path segment. Anything else reaches out of the
        // directory it is supposed to be creating something in, and a browser
        // that made a repository two levels up because somebody typed a slash
        // would be a browser nobody should leave open.
        if name.is_empty() {
            return Err("a repository needs a name".into());
        }
        if name.starts_with('.') || name.contains(std::path::is_separator) || name.contains("..") {
            return Err(format!("{name:?} is not a name a directory can have").into());
        }

        let path = self.repositories_dir().join(name);
        if path.exists() {
            return Err(format!("{name} is already here").into());
        }
        std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;

        let out = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&path)
            .output()
            .map_err(|e| format!("git init: {e}"))?;
        if !out.status.success() {
            // Cleaned up rather than left as a directory that looks like a
            // repository and is not.
            let _ = std::fs::remove_dir(&path);
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )
            .into());
        }

        Ok(Repository {
            name: name.to_string(),
            path,
        })
    }

    /// How this repository is run: its own answer where it has one.
    pub fn config_for(&self, repo: &Repository) -> Loaded<Config> {
        let theirs = repo.path.join("ostraka.toml");
        let path = if theirs.is_file() {
            theirs
        } else {
            self.config_path()
        };
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Config::parse(&text)?)
    }

    /// Which file answered for this repository, for a screen that says so.
    pub fn config_source(&self, repo: &Repository) -> PathBuf {
        let theirs = repo.path.join("ostraka.toml");
        if theirs.is_file() {
            theirs
        } else {
            self.config_path()
        }
    }

    /// The workspace's own configuration, for the things that are not a
    /// repository's business: the policy, and the default gate.
    pub fn config(&self) -> Loaded<Config> {
        let path = self.config_path();
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Config::parse(&text)?)
    }

    /// Where worktrees are made, resolved against the workspace.
    pub fn worktrees(&self, config: &Config) -> PathBuf {
        self.root.join(&config.worktree.base)
    }

    /// Reads every `*.toml` in `.ostraka/adapters/`, in sorted order.
    pub fn profiles(&self) -> Loaded<Vec<Profile>> {
        let dir = self.adapters();
        if !dir.is_dir() {
            return Err(format!("no adapters directory at {}", dir.display()).into());
        }
        let mut paths: Vec<_> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();

        let mut profiles = Vec::with_capacity(paths.len());
        for path in paths {
            let text = std::fs::read_to_string(&path)?;
            profiles.push(Profile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?);
        }
        Ok(profiles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ostraka-ws-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn workspace(name: &str, repos: &[&str]) -> (PathBuf, Workspace) {
        let dir = scratch(name);
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("ostraka");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"t\", cmd = \"true\", required = true }]\n",
        )
        .expect("config");
        for repo in repos {
            std::fs::create_dir_all(dir.join("repositories").join(repo)).expect("repo");
        }
        let ws = Workspace::at(&dir);
        (dir, ws)
    }

    #[test]
    fn a_workspace_resolves_where_it_is() {
        // A relative root means `git worktree add` and everything here
        // disagree about where a worktree went, and the agent works in a
        // checkout git has never heard of.
        let (dir, _) = workspace("relative", &["only"]);
        let there = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("chdir");
        let here = Workspace::at(Path::new("."));
        std::env::set_current_dir(there).expect("chdir back");

        assert!(here.root.is_absolute(), "{:?}", here.root);
        let config = here.config().expect("parses");
        assert!(here.worktrees(&config).is_absolute());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_repository_does_not_have_to_be_named() {
        let (dir, ws) = workspace("one", &["only"]);
        assert_eq!(ws.repository(None).expect("found").name, "only");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn several_repositories_have_to_be_named_and_the_error_lists_them() {
        // Picking one would be picking, and the operator is the only one who
        // knows which.
        let (dir, ws) = workspace("several", &["alpha", "beta"]);
        let complaint = ws.repository(None).expect_err("ambiguous").to_string();
        assert!(complaint.contains("--repository"), "{complaint}");
        assert!(complaint.contains("alpha, beta"), "{complaint}");
        assert_eq!(ws.repository(Some("beta")).expect("named").name, "beta");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workspace_with_nothing_cloned_into_it_says_what_to_do() {
        let (dir, ws) = workspace("empty", &[]);
        let complaint = ws.repository(None).expect_err("none").to_string();
        assert!(
            complaint.contains("clone what you want worked on"),
            "{complaint}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repository_can_be_started_here_rather_than_cloned() {
        // Not every piece of work begins as somebody else's repository, and
        // this is the half of "nothing to work on yet" the browser can do.
        let (dir, ws) = workspace("start", &[]);
        let made = ws.start_repository("fresh").expect("starts one");

        assert_eq!(made.name, "fresh");
        assert!(made.path.join(".git").is_dir(), "git never heard of it");
        assert_eq!(ws.repositories(), vec![made]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_repository_is_left_needing_the_commit_a_worktree_branches_from() {
        // Left to the guided fix, which asks before it commits and reports
        // git's own words when there is no author configured.
        let (dir, ws) = workspace("half", &[]);
        let made = ws.start_repository("fresh").expect("starts one");
        assert!(!ostraka_runtime::worktree::has_a_commit(&made.path));
        assert!(ostraka_runtime::worktree::is_repository(&made.path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_name_that_is_not_a_name_is_refused_rather_than_obeyed() {
        // A browser that made a repository two levels up because somebody
        // typed a slash is a browser nobody should leave open.
        let (dir, ws) = workspace("names", &[]);
        for bad in ["", "   ", "../escape", "a/b", ".hidden"] {
            assert!(ws.start_repository(bad).is_err(), "{bad:?} was accepted");
        }
        assert!(!dir.parent().expect("a parent").join("escape").exists());

        // And a name already taken is not quietly reused.
        ws.start_repository("taken").expect("starts one");
        assert!(ws.start_repository("taken").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repository_that_brought_its_own_config_is_run_by_it() {
        // A workspace holding a Rust repository and a Node one cannot have one
        // gate between them, so the repository's answer wins where it has one.
        let (dir, ws) = workspace("own", &["theirs", "ours"]);
        std::fs::write(
            dir.join("repositories/theirs/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"theirs\", cmd = \"true\", required = true }]\n",
        )
        .expect("their config");

        let theirs = ws.repository(Some("theirs")).expect("found");
        let ours = ws.repository(Some("ours")).expect("found");
        assert_eq!(
            ws.config_for(&theirs).expect("parses").gate.checks[0].name,
            "theirs"
        );
        assert_eq!(
            ws.config_for(&ours).expect("parses").gate.checks[0].name,
            "t"
        );
        assert!(
            ws.config_source(&theirs)
                .starts_with(dir.join("repositories"))
        );
        assert_eq!(ws.config_source(&ours), ws.config_path());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn worktrees_are_the_workspaces_rather_than_the_repositorys() {
        // A checkout of somebody's repository is not somewhere to leave copies
        // of it.
        let (dir, ws) = workspace("worktrees", &["only"]);
        let config = ws.config().expect("parses");
        assert_eq!(ws.worktrees(&config), dir.join(".ostraka/worktrees"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
