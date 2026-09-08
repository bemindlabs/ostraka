//! `ostraka check` — validate configuration before anything runs.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
use std::path::Path;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(project: &Path, json: bool) -> Outcome {
    let config_path = project.join("ostraka.toml");
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;

    let config = Config::parse(&text)?;
    let mut problems: Vec<String> = Vec::new();
    if let Err(e) = config.validate() {
        problems.push(e.to_string());
    }
    // Everything downstream stands on `git worktree add`. A configuration that
    // is perfect in a directory git has never heard of is a configuration that
    // cannot run, and finding that out from a run is finding it out late.
    if !ostraka_runtime::worktree::is_repository(project) {
        problems.push(format!(
            "{} is not a git repository — a run isolates its work in a worktree, \
             so `git init` here first",
            project.display()
        ));
    }

    let mut profiles = 0usize;
    let adapters_dir = project.join("adapters");
    if adapters_dir.is_dir() {
        for entry in std::fs::read_dir(&adapters_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            profiles += 1;
            let text = std::fs::read_to_string(&path)?;
            if let Err(e) = Profile::parse(&text) {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
    }

    if json {
        let report = serde_json::json!({
            "checks": config.gate.checks.len(),
            "profiles": profiles,
            "problems": problems,
            "ok": problems.is_empty(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if problems.is_empty() {
        println!(
            "ok — {} gate check(s), {profiles} adapter profile(s)",
            config.gate.checks.len()
        );
    } else {
        for p in &problems {
            println!("problem: {p}");
        }
    }

    Ok(problems.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-check-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(
            dir.join("ostraka.toml"),
            "[gate]\nchecks = [{ name = \"t\", cmd = \"true\", required = true }]\n",
        )
        .expect("config");
        dir
    }

    #[test]
    fn a_perfect_config_in_a_directory_git_does_not_know_is_still_a_problem() {
        // It validates, every profile parses, and nothing will ever run in it.
        let dir = scratch("no-git");
        assert!(
            !run(&dir, true).expect("checks"),
            "a directory that cannot run anything passed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_same_config_inside_a_repository_is_fine() {
        let dir = scratch("git");
        let out = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .expect("git runs");
        assert!(out.success());
        assert!(run(&dir, true).expect("checks"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
