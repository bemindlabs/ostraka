//! What a workspace kept, against what this version would have written.
//!
//! `init` never overwrites: what is there is somebody's, and a setup command
//! should not be how they lose it. The cost of that — reported as issue #70 —
//! is that after an upgrade there was no way to learn that a kept file is
//! missing keys the current version ships. `check` said `ok`, `init` said
//! `kept`, and the only method anybody found was to `init` into a scratch
//! directory and diff every file by hand.
//!
//! So the files this version would write are compared with the ones on disk.
//! Nothing here fails a check or blocks a run: a file that differs is usually
//! a file somebody edited on purpose, and this is a report, not a verdict.
//!
//! Two kinds of file are compared, and not in the same way.
//!
//! An adapter profile is shipped verbatim, so a value that disagrees with the
//! stock one is a fact worth knowing — `codex`'s `streams_json` was corrected
//! to `false` in #27 and a workspace set up before that went on declaring
//! `true`. `ostraka.toml` is *generated* from what was detected, so its values
//! are nobody's stock: a hand-written gate is the normal state and reporting
//! it as drift every time would be noise that teaches operators to skip the
//! report. Only the keys it would have are compared there.

use crate::init::{self, Kind, TEMPLATES};
use crate::workspace::Workspace;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One thing a kept file does not say, or says differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// A whole table this version writes and the file does not have.
    ///
    /// The only kind `--upgrade` acts on, and the reason it can: the stock
    /// text of a table the file has no header for is appended without a line
    /// of what is already there being read, reordered or rewritten.
    MissingTable { name: String, text: String },
    /// A key missing from a table the file does have. Reported, never written:
    /// putting it in means editing around what is there, and a file somebody
    /// hand-measured values into is not one to rewrite on their behalf.
    MissingKey(String),
    /// In both, and not the same. Never written either — this is the shape a
    /// deliberate local change takes, and it is also the shape a correction
    /// upstream takes, and nothing here can tell those apart.
    Differs {
        key: String,
        found: String,
        stock: String,
    },
    /// Every key agrees and the text does not: comments, ordering, spacing.
    ///
    /// Worth one line, because #34 corrected a profile's comment and a
    /// workspace that already had that profile never saw the correction. A
    /// comment is where a profile says what it measured and what it could not,
    /// so a stale one is a wrong answer to a question somebody will ask.
    Prose,
}

impl Finding {
    /// One line, in the order somebody reads it: what, then where.
    pub fn describe(&self) -> String {
        match self {
            Self::MissingTable { name, .. } => format!("missing [{name}]"),
            Self::MissingKey(key) => format!("missing {key}"),
            Self::Differs { key, found, stock } => {
                format!("{key} is {found}, stock says {stock}")
            }
            Self::Prose => "every key agrees; comments or ordering differ".to_string(),
        }
    }
}

/// One kept file and what it does not say.
#[derive(Debug, Clone)]
pub struct Drift {
    pub path: PathBuf,
    /// How the file is named to an operator: relative to the workspace.
    pub relative: String,
    pub findings: Vec<Finding>,
}

impl Drift {
    /// Whether `--upgrade` has anything to add here.
    pub fn upgradable(&self) -> bool {
        self.findings
            .iter()
            .any(|f| matches!(f, Finding::MissingTable { .. }))
    }
}

/// Every kept file that differs from the stock this version would write.
///
/// A profile with no shipped counterpart is not surveyed. `adapters/` is where
/// somebody puts a profile of their own, and a file this version never writes
/// has no stock to differ from.
pub fn survey(workspace: &Workspace) -> Vec<Drift> {
    let mut drifts = Vec::new();

    let config = workspace.config_path();
    if let Ok(found) = std::fs::read_to_string(&config) {
        let stock = init::config_for(Kind::of_repositories(&workspace.repositories_dir()));
        let findings = compare(&found, &stock, Values::Generated);
        if !findings.is_empty() {
            drifts.push(drift(workspace, config, findings));
        }
    }

    for (name, stock) in TEMPLATES {
        let path = workspace.adapters().join(name);
        let Ok(found) = std::fs::read_to_string(&path) else {
            continue;
        };
        let findings = compare(&found, stock, Values::Shipped);
        if !findings.is_empty() {
            drifts.push(drift(workspace, path, findings));
        }
    }

    drifts
}

/// Appends the missing tables, and nothing else.
///
/// Returns what it added to each file. A file whose only findings are a
/// missing key, a differing value or prose is left exactly as it was, and says
/// so by not appearing.
pub fn upgrade(drifts: &[Drift]) -> std::io::Result<Vec<(PathBuf, Vec<String>)>> {
    let mut added = Vec::new();
    for drift in drifts {
        let tables: Vec<(&String, &String)> = drift
            .findings
            .iter()
            .filter_map(|f| match f {
                Finding::MissingTable { name, text } => Some((name, text)),
                _ => None,
            })
            .collect();
        if tables.is_empty() {
            continue;
        }
        let mut text = std::fs::read_to_string(&drift.path)?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        for (_, table) in &tables {
            text.push('\n');
            text.push_str(table);
        }
        std::fs::write(&drift.path, &text)?;
        added.push((
            drift.path.clone(),
            tables.iter().map(|(name, _)| (*name).clone()).collect(),
        ));
    }
    Ok(added)
}

fn drift(workspace: &Workspace, path: PathBuf, findings: Vec<Finding>) -> Drift {
    let relative = path
        .strip_prefix(&workspace.root)
        .unwrap_or(&path)
        .display()
        .to_string();
    Drift {
        path,
        relative,
        findings,
    }
}

/// Whether the stock file's *values* mean anything here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Values {
    /// Shipped verbatim: a value that disagrees is worth reporting.
    Shipped,
    /// Generated from what was detected: only the keys are comparable.
    Generated,
}

/// What one kept file does not say, against the stock one.
///
/// A file that does not parse produces nothing. `check` already reports that,
/// in its own words and as a problem rather than as a note, and a second
/// account of the same broken file helps nobody.
fn compare(found: &str, stock: &str, values: Values) -> Vec<Finding> {
    let (Ok(found_doc), Ok(stock_doc)) = (
        toml::from_str::<toml::Value>(found),
        toml::from_str::<toml::Value>(stock),
    ) else {
        return Vec::new();
    };

    let found_keys = flatten(&found_doc);
    let stock_keys = flatten(&stock_doc);
    let headers = tables(stock);

    let mut findings = Vec::new();
    let mut named: Vec<String> = Vec::new();
    for (key, stock_value) in &stock_keys {
        // Where a workspace keeps its repositories is the operator's answer to
        // a question `init` does not ask, so its absence is not drift.
        if key.starts_with("workspace.") {
            continue;
        }
        match found_keys.get(key) {
            Some(found_value) => {
                if values == Values::Shipped && found_value != stock_value {
                    findings.push(Finding::Differs {
                        key: key.clone(),
                        found: found_value.clone(),
                        stock: stock_value.clone(),
                    });
                }
            }
            None => match missing_table(key, &headers, &found_keys) {
                Some(name) => {
                    if !named.contains(&name) {
                        if let Some(text) = table_text(stock, &name) {
                            findings.push(Finding::MissingTable {
                                name: name.clone(),
                                text,
                            });
                        }
                        named.push(name);
                    }
                }
                None => findings.push(Finding::MissingKey(key.clone())),
            },
        }
    }

    // Said last, and only when there is nothing else to say: a file with a
    // missing table has drifted in a way that names itself, and adding "and
    // the comments differ too" to that tells nobody anything.
    if findings.is_empty() && values == Values::Shipped && found != stock {
        findings.push(Finding::Prose);
    }
    findings
}

/// Every key in a document, dotted, with its value rendered for a person.
///
/// Arrays are leaves. `gate.checks` is the whole gate as one value, which is
/// the level somebody reasons about it at — a report naming
/// `gate.checks[2].cmd` describes a diff rather than a project.
fn flatten(document: &toml::Value) -> BTreeMap<String, String> {
    fn walk(value: &toml::Value, prefix: &str, out: &mut BTreeMap<String, String>) {
        match value {
            toml::Value::Table(table) => {
                for (key, value) in table {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(value, &path, out);
                }
            }
            other => {
                out.insert(prefix.to_string(), render(other));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(document, "", &mut out);
    out
}

/// A value as it is worth putting in a one-line report.
fn render(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => format!("{text:?}"),
        toml::Value::Array(items) => format!("{} item(s)", items.len()),
        other => other.to_string(),
    }
}

/// The table headers written in a file, in the order they appear.
fn tables(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .filter(|name| !name.starts_with('['))
                .map(str::to_string)
        })
        .collect()
}

/// The table a missing key is missing with, if the whole table is absent.
///
/// The deepest header that is a prefix of the key and that the found document
/// has nothing under. `gate.review` rather than `gate` for a file that has a
/// gate and no review table, so `--upgrade` appends the smallest thing that
/// answers.
fn missing_table(
    key: &str,
    headers: &[String],
    found: &BTreeMap<String, String>,
) -> Option<String> {
    headers
        .iter()
        .filter(|name| key.starts_with(&format!("{name}.")))
        .filter(|name| {
            let under = format!("{name}.");
            !found.keys().any(|k| k.starts_with(&under))
        })
        .max_by_key(|name| name.len())
        .cloned()
}

/// One table's stock text: its header, the comment block introducing it, and
/// everything under it up to the next header.
///
/// The comments come along because they are the reason the table says what it
/// says. A `[routing]` block appended without the paragraph above it arrives as
/// four vendor names and no account of what they are for.
fn table_text(stock: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = stock.lines().collect();
    let header = format!("[{name}]");
    let at = lines.iter().position(|line| line.trim() == header)?;

    let mut start = at;
    while start > 0 && lines[start - 1].trim_start().starts_with('#') {
        start -= 1;
    }

    let mut end = at + 1;
    while end < lines.len() && !lines[end].trim_start().starts_with('[') {
        end += 1;
    }
    // A comment block at the bottom introduces the next table, not this one.
    while end > at + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut trailing = end;
    while trailing > at + 1 && lines[trailing - 1].trim_start().starts_with('#') {
        trailing -= 1;
    }
    if trailing > at + 1 {
        end = trailing;
        while end > at + 1 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
    }

    let mut text = lines[start..end].join("\n");
    text.push('\n');
    Some(text)
}

/// The version whose stock this is, for the sentence that reports drift.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The report, as `check` and `init` both print it.
pub fn report(drifts: &[Drift]) -> String {
    let mut out = format!(
        "\ndrift — {} kept file(s) differ from what ostraka {} would write\n",
        drifts.len(),
        version()
    );
    for drift in drifts {
        out.push_str(&format!("  {}\n", drift.relative));
        for finding in &drift.findings {
            out.push_str(&format!("    {}\n", finding.describe()));
        }
    }
    if drifts.iter().any(Drift::upgradable) {
        out.push_str(
            "\n`ostraka init --upgrade` appends the missing tables. A missing key, a\n\
             differing value and a comment are reported and never written: that is the\n\
             shape a deliberate local change takes as well as the shape a correction\n\
             upstream takes, and nothing here can tell those apart.\n",
        );
    }
    out
}

/// The drift a file has, as JSON for `--json` callers.
pub fn as_json(drifts: &[Drift]) -> serde_json::Value {
    serde_json::Value::Array(
        drifts
            .iter()
            .map(|drift| {
                serde_json::json!({
                    "path": drift.path.display().to_string(),
                    "relative": drift.relative,
                    "findings": drift
                        .findings
                        .iter()
                        .map(|f| f.describe())
                        .collect::<Vec<_>>(),
                    "upgradable": drift.upgradable(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "\
id = \"codex\"
command = \"codex\"

# Why this table is here.
[capabilities]
headless = true
streams_json = false

# Introducing the models table.
[models]
args = [\"models\"]
";

    #[test]
    fn a_table_this_version_ships_and_the_file_lacks_is_named_with_its_text() {
        let found = "id = \"codex\"\ncommand = \"codex\"\n\n[capabilities]\nheadless = true\nstreams_json = false\n";
        let findings = compare(found, STOCK, Values::Shipped);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let Finding::MissingTable { name, text } = &findings[0] else {
            panic!("{findings:?}");
        };
        assert_eq!(name, "models");
        // The comment introducing it comes along; the one above the table
        // before it does not.
        assert_eq!(
            text,
            "# Introducing the models table.\n[models]\nargs = [\"models\"]\n"
        );
    }

    #[test]
    fn a_value_corrected_upstream_is_reported_and_not_written() {
        let found = STOCK.replace("streams_json = false", "streams_json = true");
        let findings = compare(&found, STOCK, Values::Shipped);
        assert_eq!(
            findings,
            vec![Finding::Differs {
                key: "capabilities.streams_json".to_string(),
                found: "true".to_string(),
                stock: "false".to_string(),
            }]
        );
        assert!(
            !Drift {
                path: PathBuf::new(),
                relative: String::new(),
                findings,
            }
            .upgradable()
        );
    }

    /// #34 corrected a profile's comment. A workspace that already had the
    /// profile kept the stale one, and nothing said so.
    #[test]
    fn a_profile_that_differs_only_in_its_comments_says_so() {
        let found = STOCK.replace("# Why this table is here.", "# What it used to say.");
        assert_eq!(
            compare(&found, STOCK, Values::Shipped),
            vec![Finding::Prose]
        );
    }

    #[test]
    fn a_file_that_matches_the_stock_it_shipped_with_has_nothing_to_report() {
        assert!(compare(STOCK, STOCK, Values::Shipped).is_empty());
    }

    /// A generated config is compared by its keys only: a gate somebody wrote
    /// is the normal state, and reporting it every time is how a report gets
    /// skipped.
    #[test]
    fn a_hand_written_gate_is_not_drift_and_a_missing_routing_table_is() {
        let stock = init::config_for(Kind::Rust);
        let found = stock
            .replace("cargo test --workspace", "cargo nextest run")
            .replace(
                "reviewers = [\"claude-code\", \"codex\", \"copilot-cli\", \"grok\"]\n",
                "",
            )
            .replace("[routing]\n", "");
        let findings = compare(&found, &stock, Values::Generated);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            matches!(&findings[0], Finding::MissingTable { name, .. } if name == "routing"),
            "{findings:?}"
        );
    }

    /// The whole of #70's first finding: a workspace set up under an earlier
    /// version, upgraded, and told what it is missing.
    #[test]
    fn upgrading_appends_the_missing_table_and_leaves_the_rest_alone() {
        let dir = std::env::temp_dir().join(format!("ostraka-drift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("codex.toml");
        let before = "id = \"codex\"\ncommand = \"codex\"\n\n[capabilities]\nheadless = true\nstreams_json = false\n";
        std::fs::write(&path, before).expect("write");

        let findings = compare(before, STOCK, Values::Shipped);
        let drift = Drift {
            path: path.clone(),
            relative: "codex.toml".to_string(),
            findings,
        };
        let added = upgrade(std::slice::from_ref(&drift)).expect("upgrades");
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].1, vec!["models".to_string()]);

        let after = std::fs::read_to_string(&path).expect("reads");
        assert!(
            after.starts_with(before),
            "what was there was rewritten:\n{after}"
        );
        assert!(after.contains("[models]"), "{after}");
        // And nothing is missing from it any more. It still differs in its
        // comments, because the fixture's did — which is the one finding
        // `--upgrade` is not for.
        assert_eq!(
            compare(&after, STOCK, Values::Shipped),
            vec![Finding::Prose]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
