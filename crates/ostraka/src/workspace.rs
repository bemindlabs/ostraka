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
//! **The repositories can live somewhere else.** `repositories/` is only the
//! default. `[workspace] repositories` in `.ostraka/ostraka.toml` names another
//! directory — relative to the workspace, absolute, or under `~/` — and
//! `--repositories` overrides that for one command. Somebody who already keeps
//! their code in one place should not have to move it, or symlink it, to let a
//! workspace work on it: nothing is ever written into a repository, so where it
//! lives is the operator's business.
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
    /// The directory name inside the repositories directory, which is what a
    /// record says and what `--repository` takes.
    pub name: String,
    pub path: PathBuf,
}

/// Where the repositories directory was decided, so a screen can say so.
///
/// A path somebody set in a file a week ago is a path they have forgotten
/// setting, and "no repositories in /home/them/code" is only a useful sentence
/// alongside "because `.ostraka/ostraka.toml` says so".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoriesFrom {
    /// Nobody chose: `repositories/` inside the workspace.
    Default,
    /// `[workspace] repositories` in `.ostraka/ostraka.toml`.
    Config,
    /// `--repositories` on the command line.
    Flag,
}

impl RepositoriesFrom {
    pub fn describe(self) -> &'static str {
        match self {
            RepositoriesFrom::Default => "the default",
            RepositoriesFrom::Config => "[workspace] repositories in .ostraka/ostraka.toml",
            RepositoriesFrom::Flag => "--repositories",
        }
    }
}

/// The part of `.ostraka/ostraka.toml` only this binary reads.
///
/// Parsed here rather than added to `ostraka_core::config::Config`. Where a
/// workspace keeps its repositories is a question about this workspace, not
/// about how any repository is verified — and a repository's own
/// `ostraka.toml`, which wins for the gate, has no business deciding it. It
/// also keeps the published `Config` untouched: that parser ignores a table it
/// does not know, so the same file serves both.
#[derive(Debug, Default, serde::Deserialize)]
struct Layout {
    #[serde(default)]
    workspace: LayoutTable,
    #[serde(default)]
    routing: RoutingTable,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LayoutTable {
    #[serde(default)]
    repositories: Option<String>,
}

/// `[routing]`: who this workspace would rather have review, in order.
///
/// A workspace's preference, for the same reason as `[workspace]`: which of the
/// profiles on this machine should judge changes is not a question about how a
/// repository is verified, and the published `Config` stays untouched.
#[derive(Debug, Default, serde::Deserialize)]
struct RoutingTable {
    #[serde(default)]
    reviewers: Vec<String>,
}

/// A path somebody wrote down, made into one this process can use.
///
/// `~` and `~/…` are the home directory, because that is what everyone who
/// types them means and a TOML string does not expand anything by itself.
/// `~user` is not expanded — it is a directory literally called that. Relative
/// paths are read against `base`, which is the workspace for a file and the
/// current directory for a flag, since those are where each one was written.
fn resolve(given: &str, base: &Path, home: Option<&Path>) -> PathBuf {
    let path = match (given.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(given),
    };
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    // Canonical where the directory exists. Where it does not — which is the
    // case an error message is about to name — `..` and `.` are folded by hand,
    // so the operator reads `/home/them/nowhere` rather than
    // `/home/them/ws/../nowhere` and does not have to work out which one was
    // meant.
    path.canonicalize().unwrap_or_else(|_| clean(&path))
}

/// `a/./b/../c` as `a/c`, without touching the filesystem.
///
/// Lexical, so it can be wrong about a symlinked parent — which is acceptable
/// only because it is used for a path that does not exist, where there is no
/// link to be wrong about.
fn clean(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // Up out of a directory: drop it.
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // Nothing is above the root, so there is nowhere to go.
                Some(Component::RootDir | Component::Prefix(_)) => {}
                // Relative and already climbing, or nothing yet: keep it.
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    out
}

/// A directory given on the command line, as this process should use it.
///
/// Shared by `Workspace::open` and `init` so there is one answer. The shell
/// expands a bare `~/code`, but not `--repositories=~/code` or a quoted
/// `"~/code"` — and `init` used to take those literally, planning a directory
/// called `~` and writing it into the config. Raised in review.
pub(crate) fn resolve_flag(given: &Path) -> std::io::Result<PathBuf> {
    let here = std::env::current_dir()?;
    Ok(resolve(&given.to_string_lossy(), &here, home().as_deref()))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    /// Where the repositories being worked on are, resolved.
    pub repositories: PathBuf,
    /// And who decided that.
    pub repositories_from: RepositoriesFrom,
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
    ///
    /// Reads `[workspace] repositories` if the file says it, and quietly takes
    /// the default if the file is missing or does not parse. That leniency is
    /// for the callers that are planning rather than running — `init`, and the
    /// setup screen deciding what to offer. Everything that runs goes through
    /// [`Workspace::open`], which says so instead.
    pub fn at(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let declared = std::fs::read_to_string(root.join(".ostraka").join("ostraka.toml"))
            .ok()
            .and_then(|text| toml::from_str::<Layout>(&text).ok())
            .and_then(|layout| layout.workspace.repositories)
            .filter(|given| !given.trim().is_empty());
        let (repositories, repositories_from) = match declared {
            Some(given) => (
                resolve(&given, &root, home().as_deref()),
                RepositoriesFrom::Config,
            ),
            None => (root.join("repositories"), RepositoriesFrom::Default),
        };
        Self {
            root,
            repositories,
            repositories_from,
        }
    }

    /// The workspace as a command will run in it, with every choice checked.
    ///
    /// `flag` is `--repositories`, and wins over the file. A directory somebody
    /// *chose* that is not there is an error naming it and naming who chose it,
    /// rather than a workspace that looks empty — "no repositories" sends
    /// someone to clone into a directory that was never the one they meant. The
    /// default is allowed to be missing, because `init` and the browser both
    /// know how to make it.
    pub fn open(root: &Path, flag: Option<&Path>) -> Loaded<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let file = root.join(".ostraka").join("ostraka.toml");
        let declared = match std::fs::read_to_string(&file) {
            Ok(text) => {
                toml::from_str::<Layout>(&text)
                    .map_err(|e| format!("{}: {e}", file.display()))?
                    .workspace
                    .repositories
            }
            // Absent is a workspace nobody configured, which is allowed.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // Anything else is a file that is there and could not be read — and
            // quietly taking the default then would run in `repositories/` a
            // workspace whose config says somewhere else. Raised in review.
            Err(e) => return Err(format!("{}: {e}", file.display()).into()),
        };

        let (repositories, repositories_from) = if let Some(flag) = flag {
            (resolve_flag(flag)?, RepositoriesFrom::Flag)
        } else if let Some(given) = declared {
            if given.trim().is_empty() {
                return Err(format!(
                    "{}: [workspace] repositories is empty — remove it for the default, \
                     or name a directory",
                    file.display()
                )
                .into());
            }
            (
                resolve(&given, &root, home().as_deref()),
                RepositoriesFrom::Config,
            )
        } else {
            return Ok(Self {
                repositories: root.join("repositories"),
                root,
                repositories_from: RepositoriesFrom::Default,
            });
        };

        // The workspace itself would list `.ostraka` and `notes` as
        // repositories and offer to run agents in them.
        if repositories == root {
            return Err(format!(
                "the repositories directory cannot be the workspace itself ({}) — set by {}",
                root.display(),
                repositories_from.describe()
            )
            .into());
        }
        if repositories.exists() && !repositories.is_dir() {
            return Err(format!(
                "the repositories directory {} is a file, not a directory — set by {}",
                repositories.display(),
                repositories_from.describe()
            )
            .into());
        }
        if !repositories.is_dir() {
            return Err(format!(
                "the repositories directory {} does not exist — set by {}; create it, or \
                 point it at one that does",
                repositories.display(),
                repositories_from.describe()
            )
            .into());
        }
        Ok(Self {
            root,
            repositories,
            repositories_from,
        })
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
        self.repositories.clone()
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

    /// Why a run that would be gated cannot start here, if anything.
    ///
    /// `init` writes a check that fails on purpose where it could not tell how
    /// a project is verified. That is the right thing to write — a generated
    /// gate that passed everything would be worse — and the wrong thing to
    /// find out from a refusal: the run is authored, a vendor is paid, the
    /// gate then fails by design, and nothing on the way there said the gate
    /// was a placeholder. It reads as "everything is rejected", which is how
    /// it was reported.
    ///
    /// Asked of one repository, the one a run would be made in. A task naming
    /// another is not covered, and that is the honest limit of a check made
    /// before anything is claimed: this refuses what is certainly wrong rather
    /// than guessing at what might be.
    ///
    /// Only for runs. A question is not gated, so a workspace with no gate yet
    /// is still one somebody can use — which is what makes refusing the run
    /// affordable.
    pub fn placeholder_gate(&self, repository: Option<&str>) -> Option<String> {
        let repo = self.repository(repository).ok()?;
        let config = self.config_for(&repo).ok()?;
        if !config
            .gate
            .checks
            .iter()
            .any(|check| check.name == crate::init::PLACEHOLDER_CHECK)
        {
            return None;
        }
        let source = self.config_source(&repo);
        let named = source.strip_prefix(&self.root).unwrap_or(&source);
        Some(format!(
            "the gate here is still the placeholder `{}`, which fails on purpose \u{2014} say how \
             this project is verified in {}, or ask instead: a question is not gated",
            crate::init::PLACEHOLDER_CHECK,
            named.display()
        ))
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

    /// The reviewers this workspace prefers, in order, for a run nobody named
    /// a reviewer for. Read when asked: it is a file somebody edits.
    ///
    /// `[routing] reviewers` where the workspace says, and it always wins —
    /// which profiles judge changes here is the operator's call. Where it says
    /// nothing, the profiles that can review read-only and are handed the
    /// repository's own rules, in id order. Empty where there are none of
    /// those, which leaves the choice to routing, so a workspace whose only
    /// other profile falls short still gets a pair.
    ///
    /// The default is what changed. Where a workspace said nothing, routing
    /// used to decide on its own, and it ordered by whether a profile could
    /// review read-only and then by id — so `agy`, which reviews read-only and
    /// is not handed `AGENTS.md`, became the reviewer wherever its id sorted
    /// first, and judged changes without the rules they were written under.
    /// Only workspaces `init` had written a list into were spared that.
    pub fn reviewers(&self) -> Vec<String> {
        let listed = std::fs::read_to_string(self.config_path())
            .ok()
            .and_then(|text| toml::from_str::<Layout>(&text).ok())
            .map(|layout| layout.routing.reviewers)
            .unwrap_or_default();
        if !listed.is_empty() {
            return listed;
        }
        self.reviews_with_the_rules()
    }

    /// Profiles that review read-only and do not say they go without the
    /// repository's instructions.
    ///
    /// Read from the files rather than from `Profile`, the same way `[models]`
    /// is: whether a CLI is handed `AGENTS.md` is a fact about the vendor, a
    /// profile is where vendor facts are written down, and a field for it on
    /// the published type is a breaking change. The published parser ignores a
    /// table it does not know, so one file serves both.
    fn reviews_with_the_rules(&self) -> Vec<String> {
        #[derive(serde::Deserialize)]
        struct Side {
            id: String,
            #[serde(default)]
            review_args: Vec<String>,
            #[serde(default)]
            instructions: InstructionsTable,
        }
        #[derive(Default, serde::Deserialize)]
        struct InstructionsTable {
            repository: Option<bool>,
        }

        let Ok(entries) = std::fs::read_dir(self.adapters()) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .filter_map(|text| toml::from_str::<Side>(&text).ok())
            .filter(|side| !side.review_args.is_empty())
            .filter(|side| side.instructions.repository != Some(false))
            .map(|side| side.id)
            .collect();
        ids.sort();
        ids
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

    fn declare(dir: &Path, table: &str) {
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            format!(
                "[gate]\nchecks = [{{ name = \"t\", cmd = \"true\", required = true }}]\n\n{table}"
            ),
        )
        .expect("config");
    }

    #[test]
    fn the_repositories_directory_is_repositories_unless_somebody_says_otherwise() {
        let (dir, ws) = workspace("default-layout", &["only"]);
        assert_eq!(ws.repositories_dir(), ws.root.join("repositories"));
        assert_eq!(ws.repositories_from, RepositoriesFrom::Default);
        let opened = Workspace::open(&dir, None).expect("opens");
        assert_eq!(opened.repositories_dir(), ws.root.join("repositories"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workspace_can_keep_its_repositories_somewhere_else() {
        let (dir, _) = workspace("custom-layout", &[]);
        std::fs::create_dir_all(dir.join("code/alpha")).expect("repo");
        declare(&dir, "[workspace]\nrepositories = \"code\"\n");

        let ws = Workspace::open(&dir, None).expect("opens");
        assert_eq!(ws.repositories_from, RepositoriesFrom::Config);
        assert_eq!(ws.repositories_dir(), ws.root.join("code"));
        assert_eq!(ws.repository(None).expect("found").name, "alpha");

        // Starting one puts it where the workspace keeps them, not in a
        // `repositories/` nobody asked for.
        let made = ws.start_repository("fresh").expect("starts");
        assert!(
            made.path.starts_with(ws.root.join("code")),
            "{:?}",
            made.path
        );
        assert!(!dir.join("repositories/fresh").exists());

        // The lenient constructor agrees, and it is the one init plans with.
        assert_eq!(Workspace::at(&dir).repositories_dir(), ws.root.join("code"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_absolute_directory_outside_the_workspace_is_allowed() {
        // Nothing is written into a repository, so it can live anywhere —
        // including where somebody already keeps all their code.
        let (dir, _) = workspace("absolute-layout", &[]);
        let elsewhere = scratch("elsewhere");
        std::fs::create_dir_all(elsewhere.join("theirs")).expect("repo");
        declare(
            &dir,
            &format!(
                "[workspace]\nrepositories = \"{}\"\n",
                elsewhere.display().to_string().replace('\\', "\\\\")
            ),
        );
        let ws = Workspace::open(&dir, None).expect("opens");
        assert_eq!(ws.repository(None).expect("found").name, "theirs");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn a_tilde_means_the_home_directory_and_nothing_else() {
        let base = Path::new("/workspace");
        let home = Path::new("/home/someone");
        assert_eq!(
            resolve("~/code", base, Some(home)),
            PathBuf::from("/home/someone/code")
        );
        assert_eq!(
            resolve("~", base, Some(home)),
            PathBuf::from("/home/someone")
        );
        assert_eq!(
            resolve("code", base, Some(home)),
            PathBuf::from("/workspace/code")
        );
        // `~other` is a directory literally called that.
        assert_eq!(
            resolve("~other/x", base, Some(home)),
            PathBuf::from("/workspace/~other/x")
        );
    }

    #[test]
    fn a_path_that_does_not_exist_is_named_without_its_detours() {
        // The error about a missing directory is the one place this path is
        // read by a person, and `ws/../nowhere` makes them work out where that is.
        let base = Path::new("/nonexistent-ostraka/ws");
        assert_eq!(
            resolve("../nowhere", base, None),
            PathBuf::from("/nonexistent-ostraka/nowhere")
        );
        assert_eq!(
            resolve("a/./b/../c", base, None),
            PathBuf::from("/nonexistent-ostraka/ws/a/c")
        );
        // Nothing climbs above the root.
        assert_eq!(resolve("/../../x", base, None), PathBuf::from("/x"));
    }

    #[test]
    fn a_config_that_is_there_but_cannot_be_read_is_an_error_not_a_default() {
        // A directory where the file belongs is the portable way to make a read
        // fail with something other than "not found".
        let (dir, _) = workspace("unreadable-config", &[]);
        let file = dir.join(".ostraka/ostraka.toml");
        std::fs::remove_file(&file).expect("remove");
        std::fs::create_dir_all(&file).expect("a directory where the file goes");
        let err = Workspace::open(&dir, None)
            .expect_err("must refuse")
            .to_string();
        assert!(err.contains("ostraka.toml"), "{err}");

        // Whereas no config at all is simply a workspace nobody configured.
        std::fs::remove_dir_all(&file).expect("remove");
        assert_eq!(
            Workspace::open(&dir, None)
                .expect("opens")
                .repositories_from,
            RepositoriesFrom::Default
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_flag_wins_over_the_file() {
        let (dir, _) = workspace("flag-layout", &[]);
        std::fs::create_dir_all(dir.join("code/a")).expect("repo");
        std::fs::create_dir_all(dir.join("other/b")).expect("repo");
        declare(&dir, "[workspace]\nrepositories = \"code\"\n");
        let ws = Workspace::open(&dir, Some(&dir.join("other"))).expect("opens");
        assert_eq!(ws.repositories_from, RepositoriesFrom::Flag);
        assert_eq!(ws.repository(None).expect("found").name, "b");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_chosen_directory_that_is_not_there_is_named_rather_than_found_empty() {
        let (dir, _) = workspace("missing-layout", &[]);
        declare(&dir, "[workspace]\nrepositories = \"nowhere\"\n");
        let err = Workspace::open(&dir, None)
            .expect_err("must refuse")
            .to_string();
        assert!(err.contains("nowhere"), "{err}");
        assert!(err.contains("[workspace] repositories"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_that_would_list_the_workspace_itself_is_refused() {
        let (dir, _) = workspace("self-layout", &[]);
        for given in ["", "."] {
            declare(&dir, &format!("[workspace]\nrepositories = \"{given}\"\n"));
            assert!(
                Workspace::open(&dir, None).is_err(),
                "{given:?} was accepted"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_file_is_an_error_to_run_in_and_the_default_to_plan_with() {
        let (dir, _) = workspace("broken-layout", &[]);
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[workspace\nrepositories = ",
        )
        .expect("write");
        let err = Workspace::open(&dir, None)
            .expect_err("must refuse")
            .to_string();
        assert!(err.contains("ostraka.toml"), "{err}");
        assert_eq!(
            Workspace::at(&dir).repositories_from,
            RepositoriesFrom::Default
        );
        std::fs::remove_dir_all(&dir).ok();
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

    /// `init`'s placeholder gate is recognised, and a real one is not mistaken
    /// for it. Both the browser and the command line ask this one question.
    #[test]
    fn a_placeholder_gate_is_named_and_a_real_one_is_left_alone() {
        let dir = scratch("placeholder");
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        let config = dir.join(".ostraka/ostraka.toml");
        std::fs::create_dir_all(config.parent().expect("parent")).expect("ostraka");

        std::fs::write(
            &config,
            format!(
                "[gate]\nchecks = [{{ name = \"{}\", cmd = \"exit 1\", required = true }}]\n",
                crate::init::PLACEHOLDER_CHECK
            ),
        )
        .expect("config");
        let workspace = Workspace::at(&dir);
        let said = workspace
            .placeholder_gate(None)
            .expect("the placeholder was not recognised");
        assert!(said.contains(crate::init::PLACEHOLDER_CHECK), "{said}");
        // It says where to fix it, and that asking still works.
        assert!(said.contains("ostraka.toml"), "{said}");
        assert!(said.contains("ask instead"), "{said}");

        std::fs::write(
            &config,
            "[gate]\nchecks = [{ name = \"test\", cmd = \"true\", required = true }]\n",
        )
        .expect("config");
        assert_eq!(Workspace::at(&dir).placeholder_gate(None), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Where a workspace names no reviewers, the default leaves out a profile
    /// that says it is not handed the repository's instructions — and where it
    /// does name them, what it names is used as written.
    #[test]
    fn a_reviewer_that_never_sees_the_rules_is_not_the_default() {
        let dir = scratch("reviewers");
        let adapters = dir.join(".ostraka/adapters");
        std::fs::create_dir_all(&adapters).expect("adapters");
        let profile = |id: &str, extra: &str| {
            std::fs::write(
                adapters.join(format!("{id}.toml")),
                format!("id = \"{id}\"\ncommand = \"true\"\nargs = [\"{{{{prompt}}}}\"]\n{extra}"),
            )
            .expect("profile");
        };
        // Sorts first, reviews read-only, and says it never sees AGENTS.md.
        profile(
            "agy",
            "review_args = [\"{{prompt}}\"]\n\n[instructions]\nrepository = false\n",
        );
        profile("codex", "review_args = [\"{{prompt}}\"]\n");
        // Cannot review read-only at all, so it is no default either.
        profile("writer", "");
        std::fs::write(dir.join(".ostraka/ostraka.toml"), "[gate]\nchecks = []\n").expect("config");

        assert_eq!(Workspace::at(&dir).reviewers(), vec!["codex".to_string()]);

        // The workspace's own list wins, agy included, because it is a choice.
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = []\n\n[routing]\nreviewers = [\"agy\", \"codex\"]\n",
        )
        .expect("config");
        assert_eq!(
            Workspace::at(&dir).reviewers(),
            vec!["agy".to_string(), "codex".to_string()]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// With nothing else that can review read-only and see the rules, the
    /// default is empty and routing decides as before, so the workspace still
    /// gets a pair rather than no reviewer at all.
    #[test]
    fn with_no_better_reviewer_the_choice_is_left_to_routing() {
        let dir = scratch("reviewers-none");
        let adapters = dir.join(".ostraka/adapters");
        std::fs::create_dir_all(&adapters).expect("adapters");
        std::fs::write(
            adapters.join("agy.toml"),
            "id = \"agy\"\ncommand = \"true\"\nargs = [\"{{prompt}}\"]\n\
             review_args = [\"{{prompt}}\"]\n\n[instructions]\nrepository = false\n",
        )
        .expect("profile");
        assert!(Workspace::at(&dir).reviewers().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The shipped profile says it, and the published parser still reads it.
    #[test]
    fn the_shipped_agy_profile_declares_it_and_still_parses() {
        let (_, text) = crate::init::TEMPLATES
            .iter()
            .find(|(name, _)| *name == "agy.toml")
            .expect("agy is shipped");
        assert!(
            text.contains("[instructions]\nrepository = false"),
            "{text}"
        );
        ostraka_adapter::Profile::parse(text).expect("the published parser ignores the table");
    }
}
