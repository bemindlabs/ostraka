//! `ostraka check` — validate configuration before anything runs.

use crate::workspace::Workspace;
use ostraka_adapter::Profile;

type Outcome = Result<bool, Box<dyn std::error::Error>>;

pub fn run(workspace: &Workspace, fix: bool, json: bool) -> Outcome {
    let mut problems: Vec<String> = Vec::new();

    if !workspace.declared() {
        return Err(format!(
            "{} is not an Ostraka workspace — `ostraka init` writes one",
            workspace.root.display()
        )
        .into());
    }

    let mut profiles = 0usize;
    let adapters_dir = workspace.adapters();
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
    } else {
        problems.push(format!("no adapter profiles in {}", adapters_dir.display()));
    }

    // Every repository is checked, and each against the configuration that
    // will actually run it: a repository that brought its own answer is judged
    // by that one, not by the workspace's.
    let repositories = workspace.repositories();
    if repositories.is_empty() {
        problems.push(format!(
            "no repositories in {} — clone what you want worked on into it, \
             or `ostraka tui` starts one",
            workspace.repositories_dir().display()
        ));
    }
    let mut rows = Vec::new();
    let mut checks = 0usize;
    for repo in &repositories {
        let mut said: Vec<String> = Vec::new();
        match workspace.config_for(repo) {
            Ok(config) => {
                checks = checks.max(config.gate.checks.len());
                if let Err(e) = config.validate() {
                    said.push(e.to_string());
                }
            }
            Err(e) => said.push(e.to_string()),
        }
        // Everything downstream stands on `git worktree add`. A configuration
        // that is perfect in a directory git has never heard of is one that
        // cannot run, and finding that out from a run is finding it out late.
        if !ostraka_runtime::worktree::is_repository(&repo.path) {
            said.push(format!(
                "{} is not a git repository — a run isolates its work in a worktree, \
                 so `git init` or clone into it",
                repo.path.display()
            ));
        }
        problems.extend(said.iter().map(|s| format!("{}: {s}", repo.name)));
        rows.push(serde_json::json!({
            "name": repo.name,
            "config": workspace.config_source(repo).display().to_string(),
            "problems": said,
        }));
    }

    if json {
        let report = serde_json::json!({
            "workspace": workspace.root.display().to_string(),
            "checks": checks,
            "profiles": profiles,
            "repositories": rows,
            "problems": problems,
            "ok": problems.is_empty(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if problems.is_empty() {
        println!(
            "ok — {profiles} adapter profile(s), {} repositor{}",
            repositories.len(),
            if repositories.len() == 1 { "y" } else { "ies" }
        );
        for repo in &repositories {
            let source = workspace.config_source(repo);
            let source = source.strip_prefix(&workspace.root).unwrap_or(&source);
            println!("  {:<20} {}", repo.name, source.display());
        }
    } else {
        for p in &problems {
            println!("problem: {p}");
        }
    }

    // `--fix` walks the same steps the browser takes, from the same module.

    // Only where somebody is there to answer: a walk that met the commit

    // step unattended would either hang on a question nobody answers or

    // commit a working directory on its own, and both are worse than the

    // error it was asked to remove.

    // Whenever `--fix` is set and something has a way out — not when `problems`
    // is non-empty. The two are not the same set: `check` does not report a
    // repository with no commits, and that is precisely a case `Remedy` exists
    // to fix, so gating on `problems` skipped the remedy most worth running.
    let fixable = workspace
        .repositories()
        .iter()
        .any(|repo| crate::remedy::Remedy::diagnose(&repo.path).is_some());
    if fix && fixable {
        if json {
            return Err("--fix asks questions, which --json has nowhere to put".into());
        }

        if !crate::offer::at_a_terminal() {
            return Err("--fix needs a terminal to ask on; run it where you can answer".into());
        }

        for repo in workspace.repositories() {
            let Some(remedy) = crate::remedy::Remedy::diagnose(&repo.path) else {
                continue;
            };

            println!();

            crate::fix::walk(
                &repo.path,
                remedy,
                &mut std::io::stdin().lock(),
                &mut std::io::stdout(),
            )?;
        }

        // The whole check again, not the remedy. Asking only whether the
        // remedy is satisfied would report success with an adapter profile
        // still unparseable — an exit status that lies about the thing the
        // command is named for.
        return run(workspace, false, json);
    }

    Ok(problems.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A workspace with one repository in it and nothing wrong but git.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ostraka-check-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(".ostraka/adapters")).expect("scratch");
        std::fs::create_dir_all(dir.join("repositories/work")).expect("repository");
        std::fs::write(
            dir.join(".ostraka/ostraka.toml"),
            "[gate]\nchecks = [{ name = \"t\", cmd = \"true\", required = true }]\n",
        )
        .expect("config");
        std::fs::write(
            dir.join(".ostraka/adapters/one.toml"),
            "id = \"one\"\ncommand = \"true\"\nargs = [\"{{prompt}}\"]\n",
        )
        .expect("profile");
        dir
    }

    #[test]
    fn a_repository_git_does_not_know_is_a_problem_however_good_the_config_is() {
        // It validates, every profile parses, and nothing will ever run in it.
        let dir = scratch("no-git");
        let workspace = Workspace::at(&dir);
        assert!(
            !run(&workspace, false, true).expect("checks"),
            "a repository that cannot run anything passed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_same_workspace_with_a_real_repository_in_it_is_fine() {
        let dir = scratch("git");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.join("repositories/work"))
                .status()
                .expect("git runs")
                .success()
        );
        assert!(run(&Workspace::at(&dir), false, true).expect("checks"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workspace_with_nothing_cloned_into_it_says_so() {
        let dir = scratch("empty");
        std::fs::remove_dir_all(dir.join("repositories/work")).expect("remove");
        assert!(!run(&Workspace::at(&dir), false, true).expect("checks"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
