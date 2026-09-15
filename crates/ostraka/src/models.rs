//! What models each profile can run, as its own CLI says.
//!
//! **A profile says how to ask, not what the answer is.** A `[models]` table in
//! a profile names the command that lists that CLI's models and how to read its
//! output. It can also name a fixed list for a CLI that has no such command.
//! Model ids live in configuration and in a vendor's answer, never in this
//! code, which is the rule against model identifiers written into Rust applied
//! to a picker.
//!
//! ```toml
//! [models]
//! args = ["models", "openrouter"]  # run as `<command> models openrouter`
//! prefix = "openrouter/"           # keep lines that start with it, minus it
//! separator = "\t"                 # or: keep lines holding it, up to it
//! known = ["opus", "sonnet"]       # or: a fixed list, for a CLI with no command
//! ```
//!
//! The listing runs with the profile's own `[env]`, because for some profiles
//! the environment is where the models are declared, and a model the profile
//! does not declare is one it cannot run. It reads the table itself rather than
//! adding a field to the published `Profile`, whose parser ignores a table it
//! does not know.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a listing may take before it is given up on. A CLI that fetches
/// its list over the network gets a few seconds, not a stalled screen.
const LISTING: Duration = Duration::from_secs(20);

/// One model a profile can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub profile: String,
    /// `None` is the profile's own default.
    pub model: Option<String>,
}

/// What one profile's `[models]` table said, and what asking it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub profile: String,
    pub models: Vec<String>,
    /// Why the list is short or empty, where there is a reason worth saying.
    pub note: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Table {
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    separator: Option<String>,
    #[serde(default)]
    known: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ProfileFile {
    id: String,
    command: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    models: Option<Table>,
}

/// Every profile in `adapters` that says how to list its models, listed.
///
/// A profile with no `[models]` table is left out: the picker offers what can
/// be offered, and the agents dialog is where every profile is shown.
pub fn catalogs(adapters: &Path) -> Vec<Catalog> {
    let Ok(entries) = std::fs::read_dir(adapters) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| catalog(&text))
        .collect()
}

/// One profile's catalog, from the text of its file. `None` where the file
/// does not parse or says nothing about models.
pub fn catalog(text: &str) -> Option<Catalog> {
    let file: ProfileFile = toml::from_str(text).ok()?;
    let table = file.models?;
    let mut models = table.known.clone();
    let mut note = None;
    if let Some(args) = &table.args {
        match list(&file.command, args, &file.env) {
            Ok(output) => {
                let found = read(&output, table.prefix.as_deref(), table.separator.as_deref());
                if found.is_empty() {
                    note = Some(format!(
                        "`{} {}` listed nothing this profile can run",
                        file.command,
                        args.join(" ")
                    ));
                }
                models.extend(found);
            }
            Err(why) => note = Some(why),
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    models.retain(|m| seen.insert(m.clone()));
    Some(Catalog {
        profile: file.id,
        models,
        note,
    })
}

/// The picker's rows: each profile's own default first, then its models.
pub fn rows(catalogs: &[Catalog]) -> Vec<Model> {
    catalogs
        .iter()
        .flat_map(|c| {
            std::iter::once(Model {
                profile: c.profile.clone(),
                model: None,
            })
            .chain(c.models.iter().map(|m| Model {
                profile: c.profile.clone(),
                model: Some(m.clone()),
            }))
        })
        .collect()
}

/// Model ids out of a listing: lines holding the separator up to it, or lines
/// starting with the prefix without it, or every non-empty line.
fn read(output: &str, prefix: Option<&str>, separator: Option<&str>) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let line = match separator {
                Some(sep) => line.split_once(sep)?.0.trim(),
                None => line,
            };
            match prefix {
                Some(prefix) => line.strip_prefix(prefix).map(str::to_string),
                None => Some(line.to_string()),
            }
        })
        .filter(|m| !m.is_empty())
        .collect()
}

/// Runs a listing, with the profile's environment laid over this one's, and
/// gives up on it after [`LISTING`].
fn list(command: &str, args: &[String], env: &BTreeMap<String, String>) -> Result<String, String> {
    let mut child = Command::new(command)
        .args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("`{command}` could not be run: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= LISTING => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`{command} {}` took longer than {}s",
                    args.join(" "),
                    LISTING.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("`{command}` could not be read: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{command} {}` exited with {}",
            args.join(" "),
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_keeps_the_lines_that_carry_it_and_drops_it() {
        let listed = "opencode 1.18\nollama/qwen3.8:27b\n\nollama/gemma3-tools:27b\n";
        assert_eq!(
            read(listed, Some("ollama/"), None),
            ["qwen3.8:27b", "gemma3-tools:27b"]
        );
    }

    #[test]
    fn a_separator_keeps_the_lines_that_hold_it_up_to_it() {
        let listed = "Fetching available models...\nfast-high\tFast (High)\nslow\tSlow\n";
        assert_eq!(read(listed, None, Some("\t")), ["fast-high", "slow"]);
    }

    #[test]
    fn a_fixed_list_needs_no_command_and_a_profile_without_a_table_is_left_out() {
        let fixed = catalog(
            "id = \"fixed\"\ncommand = \"nothing-to-run\"\nargs = [\"{{prompt}}\"]\n\
             [models]\nknown = [\"opus\", \"sonnet\", \"opus\"]\n",
        )
        .expect("a catalog");
        assert_eq!(fixed.models, ["opus", "sonnet"], "the duplicate was kept");
        assert_eq!(fixed.note, None);

        assert_eq!(
            catalog("id = \"bare\"\ncommand = \"x\"\nargs = [\"{{prompt}}\"]\n"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_listing_runs_with_the_profile_environment_and_says_why_when_it_fails() {
        let dir = std::env::temp_dir().join(format!("ostraka-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let script = dir.join("lister.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = models ] || exit 3\necho \"p/$DECLARED\"\necho p/other\n",
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let listed = catalog(&format!(
            "id = \"lister\"\ncommand = \"{}\"\nargs = [\"{{{{prompt}}}}\"]\n\
             [env]\nDECLARED = \"from-env\"\n\
             [models]\nargs = [\"models\"]\nprefix = \"p/\"\n",
            script.display()
        ))
        .expect("a catalog");
        assert_eq!(listed.models, ["from-env", "other"]);

        let failing = catalog(&format!(
            "id = \"lister\"\ncommand = \"{}\"\nargs = [\"{{{{prompt}}}}\"]\n\
             [models]\nargs = [\"nope\"]\n",
            script.display()
        ))
        .expect("a catalog");
        assert!(failing.models.is_empty());
        assert!(
            failing
                .note
                .as_deref()
                .is_some_and(|n| n.contains("exited")),
            "{failing:?}"
        );

        let rows = rows(&[listed]);
        assert_eq!(rows[0].model, None, "the profile's own default comes first");
        assert_eq!(rows.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
