//! `@` in the box: a file, a directory or an agent, named without leaving it.
//!
//! **A path stays in the task, an agent does not.** `@src/main.rs` is part of
//! what is being asked, so it reaches the agent as written, and the agent reads
//! the file from its worktree like any other. `@codex` is not part of the task.
//! It says who should do it, so it is taken out of the text and chooses the
//! profile for that one run. A second agent named chooses the reviewer. Routing
//! still refuses a pair that is one profile: naming is not the same as being
//! allowed to.
//!
//! **Paths come from git.** They are what `git ls-files` lists, tracked or not
//! yet added, and nothing it ignores. Offering `target/` or `node_modules/`
//! would offer thousands of entries nobody means. The list is taken once per
//! repository, not once per keystroke.
//!
//! **A name that is both wins as an agent.** `@codex` is the profile. The
//! directory is `@codex/`, and the menu offers both.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Agent,
    Dir,
    File,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Agent => "agent",
            Kind::Dir => "directory",
            Kind::File => "file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What follows the `@`. A directory ends with `/`.
    pub text: String,
    pub kind: Kind,
}

/// What can be named in one repository: the agents here, and its paths.
#[derive(Debug, Clone, Default)]
pub struct Mentionable {
    /// Whether anything has been read yet.
    pub loaded: bool,
    /// The repository `paths` were read from, so a pane that moves to another
    /// one reads them again rather than offering the last one's files.
    pub repository: Option<std::path::PathBuf>,
    pub agents: Vec<String>,
    /// Every file, and every directory above one, directories ending in `/`.
    pub paths: Vec<String>,
}

/// How many candidates a menu is given. More is a list nobody reads.
const LIMIT: usize = 50;

/// The `@word` the cursor sits at the end of: where its `@` is, and the word
/// after it.
///
/// Only at the start of a word. An address in the middle of a sentence — a
/// name at a host — is not somebody reaching for a file.
pub fn token(prompt: &str, at: usize) -> Option<(usize, &str)> {
    let at = at.min(prompt.len());
    if !prompt.is_char_boundary(at) {
        return None;
    }
    // At the end of the word, or the menu would open under a cursor that has
    // been walked back into the middle of one.
    if prompt[at..]
        .chars()
        .next()
        .is_some_and(|c| !c.is_whitespace())
    {
        return None;
    }
    let before = &prompt[..at];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = before[start..].strip_prefix('@')?;
    if word.contains('@') {
        return None;
    }
    Some((start, word))
}

/// What `@query` could be completed to.
///
/// Agents whose id starts with it come first. Then the entries one level below
/// the directory the query has reached, so `@src/` lists what is in `src`
/// rather than every file under it. Where nothing at that level matches, any
/// path containing the query is offered instead, which is how `@main` finds
/// `src/main.rs`.
pub fn candidates(query: &str, found: &Mentionable) -> Vec<Candidate> {
    let lower = query.to_lowercase();
    let mut out: Vec<Candidate> = found
        .agents
        .iter()
        .filter(|id| !query.contains('/') && id.to_lowercase().starts_with(&lower))
        .map(|id| Candidate {
            text: id.clone(),
            kind: Kind::Agent,
        })
        .collect();

    let (parent, rest) = match query.rfind('/') {
        Some(i) => (&query[..=i], &query[i + 1..]),
        None => ("", query),
    };
    let rest = rest.to_lowercase();
    let level: Vec<&String> = found
        .paths
        .iter()
        .filter(|path| {
            let Some(below) = path.strip_prefix(parent) else {
                return false;
            };
            let name = below.strip_suffix('/').unwrap_or(below);
            !name.is_empty() && !name.contains('/') && name.to_lowercase().starts_with(&rest)
        })
        .collect();
    let paths: Vec<&String> = if level.is_empty() && !query.is_empty() {
        found
            .paths
            .iter()
            .filter(|path| path.to_lowercase().contains(&lower))
            .collect()
    } else {
        level
    };

    let mut paths: Vec<Candidate> = paths
        .into_iter()
        .map(|path| Candidate {
            text: path.clone(),
            kind: if path.ends_with('/') {
                Kind::Dir
            } else {
                Kind::File
            },
        })
        .collect();
    paths.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.text.cmp(&b.text)));
    out.extend(paths);
    out.truncate(LIMIT);
    out
}

/// The prompt with the `@word` ending at `at` replaced by a candidate, and
/// where the cursor goes after it.
///
/// A file or an agent is finished, so a space follows it. A directory is where
/// somebody is still going, so the cursor stays right after its `/` and the
/// menu opens again one level down.
pub fn complete(prompt: &str, at: usize, candidate: &Candidate) -> Option<(String, usize)> {
    let (start, _) = token(prompt, at)?;
    let tail = if candidate.kind == Kind::Dir { "" } else { " " };
    let inserted = format!("@{}{tail}", candidate.text);
    let mut rest = &prompt[at..];
    if !tail.is_empty() && rest.starts_with(' ') {
        rest = &rest[1..];
    }
    let completed = format!("{}{inserted}{rest}", &prompt[..start]);
    Some((completed, start + inserted.len()))
}

/// The agents a task names, taken out of it: the task as an agent should read
/// it, the author named first and the reviewer named second.
///
/// Only a word that is exactly `@` and a profile id counts. Anything else —
/// `@src/`, an id with a typo, an address — is text, and stays in the task.
pub fn agents_named(prompt: &str, agents: &[String]) -> (String, Option<String>, Option<String>) {
    let mut named: Vec<String> = Vec::new();
    let lines: Vec<String> = prompt
        .split('\n')
        .map(|line| {
            let mut kept: Vec<&str> = Vec::new();
            for word in line.split(' ') {
                let id = word
                    .strip_prefix('@')
                    .filter(|id| agents.iter().any(|a| a == id));
                match id {
                    Some(id) if named.len() < 2 => named.push(id.to_string()),
                    _ => kept.push(word),
                }
            }
            kept.join(" ").trim_end().to_string()
        })
        .collect();
    let mut named = named.into_iter();
    (
        lines.join("\n").trim().to_string(),
        named.next(),
        named.next(),
    )
}

/// Every path git would show in `repo`, and every directory above one.
pub fn paths(repo: &Path) -> Vec<String> {
    let Ok(out) = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(repo)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut all = BTreeSet::new();
    for file in String::from_utf8_lossy(&out.stdout).split('\0') {
        if file.is_empty() {
            continue;
        }
        let mut dir = String::new();
        let parts: Vec<&str> = file.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            dir.push_str(part);
            dir.push('/');
            all.insert(dir.clone());
        }
        all.insert(file.to_string());
    }
    all.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found() -> Mentionable {
        Mentionable {
            loaded: true,
            repository: None,
            agents: vec!["codex".into(), "claude-code".into()],
            paths: vec![
                "README.md".into(),
                "codex/".into(),
                "codex/notes.md".into(),
                "src/".into(),
                "src/main.rs".into(),
                "src/tui/".into(),
                "src/tui/view.rs".into(),
            ],
        }
    }

    fn texts(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn a_mention_starts_a_word_and_ends_at_the_cursor() {
        assert_eq!(token("fix @src/ma", 11), Some((4, "src/ma")));
        assert_eq!(token("@", 1), Some((0, "")));
        // An address is not a mention.
        assert_eq!(token("mail me@example.org", 19), None);
        // Nor is a word the cursor has been walked back into.
        assert_eq!(token("fix @src here", 6), None);
        assert_eq!(token("fix @src here", 8), Some((4, "src")));
    }

    #[test]
    fn a_directory_lists_what_is_in_it_and_not_everything_below_it() {
        assert_eq!(
            texts(&candidates("src/", &found())),
            ["src/tui/", "src/main.rs"]
        );
        assert_eq!(texts(&candidates("src/t", &found())), ["src/tui/"]);
    }

    #[test]
    fn agents_come_first_and_a_name_that_is_both_is_offered_both_ways() {
        let both = candidates("co", &found());
        assert_eq!(both[0].kind, Kind::Agent);
        assert_eq!(texts(&both), ["codex", "codex/"]);
        // Past a slash nobody is naming an agent.
        assert!(
            candidates("codex/", &found())
                .iter()
                .all(|c| c.kind != Kind::Agent)
        );
    }

    #[test]
    fn nothing_at_the_level_falls_back_to_anywhere_in_the_path() {
        assert_eq!(texts(&candidates("view", &found())), ["src/tui/view.rs"]);
    }

    #[test]
    fn completing_a_file_finishes_the_word_and_a_directory_keeps_going() {
        let file = Candidate {
            text: "src/main.rs".into(),
            kind: Kind::File,
        };
        assert_eq!(
            complete("fix @src/ma now", 11, &file),
            Some(("fix @src/main.rs now".to_string(), 17))
        );
        let dir = Candidate {
            text: "src/".into(),
            kind: Kind::Dir,
        };
        assert_eq!(
            complete("fix @s", 6, &dir),
            Some(("fix @src/".to_string(), 9))
        );
    }

    #[test]
    fn named_agents_leave_the_task_and_paths_stay_in_it() {
        let agents = vec!["codex".to_string(), "claude-code".to_string()];
        let (task, author, reviewer) =
            agents_named("@codex fix @src/main.rs @claude-code", &agents);
        assert_eq!(task, "fix @src/main.rs");
        assert_eq!(author.as_deref(), Some("codex"));
        assert_eq!(reviewer.as_deref(), Some("claude-code"));

        let (task, author, _) = agents_named("mail me@example.org about @nobody", &agents);
        assert_eq!(task, "mail me@example.org about @nobody");
        assert_eq!(author, None);
    }

    #[test]
    fn the_paths_are_what_git_would_show() {
        let dir = std::env::temp_dir().join(format!("ostraka-mention-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("src")).expect("dir");
        std::fs::create_dir_all(dir.join("target")).expect("dir");
        std::fs::write(dir.join("src/main.rs"), "").expect("file");
        std::fs::write(dir.join("target/junk"), "").expect("file");
        std::fs::write(dir.join(".gitignore"), "/target/\n").expect("file");
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git")
        };
        git(&["init", "-q"]);

        let listed = paths(&dir);
        assert!(listed.contains(&"src/".to_string()), "{listed:?}");
        assert!(listed.contains(&"src/main.rs".to_string()), "{listed:?}");
        assert!(
            !listed.iter().any(|p| p.starts_with("target")),
            "{listed:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
