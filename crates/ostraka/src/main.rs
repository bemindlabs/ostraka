//! The `ostraka` command.

mod adapters;
mod banner;
mod bench;
mod check;
mod discover;
mod drain;
mod fix;
mod init;
mod init_cmd;
mod offer;
mod promote;
mod prune;
mod remedy;
mod replay;
mod run;
mod runs;
mod task_cmd;
mod tasks;
mod tui;
mod workspace;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "ostraka",
    version,
    about = "Run agent fleets you can actually review."
)]
struct Cli {
    /// Workspace directory. Defaults to the current directory.
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,

    /// Which repository under `repositories/` to work in. Only needed where
    /// the workspace holds more than one.
    #[arg(long, global = true)]
    repository: Option<String>,

    /// Emit machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Absent when the binary is run by name and nothing else, which is how
    /// somebody asks a program what it is. clap's answer to a missing required
    /// subcommand is a usage error on stderr and a non-zero exit; that is a
    /// true sentence about the parser and the wrong one to be met by.
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Write a task down. It waits until a run takes it.
    Add {
        /// What the agent should do.
        prompt: String,
        /// Which repository it belongs in.
        #[arg(long)]
        repository: Option<String>,
        /// The profile that should write it.
        #[arg(long)]
        adapter: Option<String>,
    },
    /// Put a claimed task back, for a run that never reported.
    Release {
        /// The task id, as `ostraka tasks` shows it.
        id: String,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Validate the project config and every adapter profile.
    Check {
        /// Walk the steps out of what it found, one at a time, asking first.
        #[arg(long)]
        fix: bool,
    },

    /// Write the files a project needs to be run by Ostraka.
    Init {
        /// Rewrite files that already exist. Off by default: what is there is
        /// somebody's, and a setup command should not be how they lose it.
        #[arg(long)]
        force: bool,
    },

    /// List adapter profiles and whether each one can run here.
    Adapters,

    /// Run the same tasks through every candidate and tabulate what the gate
    /// said.
    ///
    /// Declared in `.ostraka/bench.toml`: the tasks, the candidates —  a
    /// profile and the models to try it on — and the reviewer profiles held
    /// constant across them. Every cell is a full run, so a matrix costs what
    /// its size says it does; `--dry-run` prints that size without spending it.
    Bench {
        /// Print the matrix and stop. Nothing is run and no vendor is called.
        #[arg(long)]
        dry_run: bool,
    },

    /// Run one task: isolate, execute, gate, review, record.
    Run {
        /// What the agent should do. Omitted with `--next`, which takes it
        /// from the list.
        prompt: Option<String>,

        /// Identity accountable for the change.
        #[arg(long, default_value = run::AUTHOR)]
        author: String,

        /// Identity that reviews it. Must differ from the author.
        #[arg(long, default_value = run::REVIEWER)]
        reviewer: String,

        /// Adapter profile that writes the change.
        #[arg(long)]
        adapter: Option<String>,

        /// Adapter profile that reviews it. Must differ from `--adapter`.
        #[arg(long)]
        review_adapter: Option<String>,

        /// Git ref the worktree branches from.
        #[arg(long, default_value = run::BASE_REF)]
        base_ref: String,

        /// Continue a finished run: branch from what it left, not from HEAD.
        /// Takes a run id. Only an approved run can be continued.
        #[arg(long, conflicts_with = "base_ref", value_name = "RUN_ID")]
        from: Option<String>,

        /// Take the oldest task from the list instead of being given one.
        #[arg(long, conflicts_with = "prompt")]
        next: bool,

        /// Model hint passed through to the adapter.
        #[arg(long)]
        model: Option<String>,
    },

    /// Write a shell completion script to stdout.
    ///
    /// Generated from this parser, so it names the commands this binary
    /// actually has. Install it the way your shell wants:
    ///
    ///   ostraka completion bash > ~/.local/share/bash-completion/completions/ostraka
    ///   ostraka completion zsh  > "${fpath[1]}/_ostraka"
    ///   ostraka completion fish > ~/.config/fish/completions/ostraka.fish
    Completion {
        /// Which shell to write for.
        shell: clap_complete::Shell,
    },

    /// Work written down before somebody is free to do it.
    Task {
        #[command(subcommand)]
        what: TaskCommand,
    },

    /// Everything on the task list, in every state.
    Tasks,

    /// Take the task list down, several at a time.
    Drain {
        /// How many runs at once.
        #[arg(long, default_value_t = drain::WORKERS)]
        workers: usize,

        /// Identity accountable for the changes.
        #[arg(long, default_value = run::AUTHOR)]
        author: String,

        /// Identity that reviews them. Must differ from the author.
        #[arg(long, default_value = run::REVIEWER)]
        reviewer: String,

        /// Adapter profile that writes, where a task has not named one.
        #[arg(long)]
        adapter: Option<String>,

        /// Adapter profile that reviews. Must differ from `--adapter`.
        #[arg(long)]
        review_adapter: Option<String>,
    },

    /// List every run this project has recorded.
    Runs,

    /// Browse runs in the terminal.
    Tui,

    /// Give an approved run a branch of its own. Merges nothing.
    Promote {
        /// Run id, as printed by `run`.
        run_id: String,

        /// Branch to create. Defaults to `promoted/<run-id>`.
        #[arg(long)]
        branch: Option<String>,
    },

    /// Remove worktrees left by finished runs. Branches are untouched.
    Prune {
        /// Actually remove them. Without this it only reports what it would do.
        #[arg(long)]
        apply: bool,
    },

    /// Read a finished run back from its record.
    Replay {
        /// Run id, as printed by `run`.
        run_id: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let Some(command) = &cli.command else {
        banner::print();
        return ExitCode::SUCCESS;
    };

    // Before a workspace is looked for. Completions are about the command line,
    // not about a project, and a shell asking for them in someone's home
    // directory should not be told that their home directory is not a
    // workspace.
    // Matched on a reference. It compiles either way today — `Shell` is `Copy`,
    // so binding it takes a copy and `cli` is not moved — but that is a fact
    // about a dependency's derives, and the day one field of this variant stops
    // being `Copy` the error lands here rather than in the change that caused
    // it. Locked once, because a completion script is one write.
    if let Commands::Completion { shell } = command {
        write_completion(*shell, &mut std::io::stdout().lock());
        return ExitCode::SUCCESS;
    }

    let here = cli.workspace.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace = workspace::Workspace::at(&here);

    let result = match command {
        Commands::Check { fix } => check::run(&workspace, *fix, cli.json),
        Commands::Init { force } => init_cmd::run(&here, *force, cli.json),
        Commands::Adapters => adapters::run(&workspace, cli.json),
        Commands::Bench { dry_run } => bench::run(&workspace, *dry_run, cli.json),
        Commands::Replay { run_id } => replay::run(&workspace, run_id, cli.json),
        Commands::Task { what } => match what {
            TaskCommand::Add {
                prompt,
                repository,
                adapter,
            } => task_cmd::add(
                &workspace,
                prompt,
                repository.as_deref().or(cli.repository.as_deref()),
                adapter.as_deref(),
                cli.json,
            ),
            TaskCommand::Release { id } => task_cmd::release(&workspace, id, cli.json),
        },
        Commands::Tasks => task_cmd::list(&workspace, cli.json),
        Commands::Drain {
            workers,
            author,
            reviewer,
            adapter,
            review_adapter,
        } => {
            let mut args = run::Args::for_task(String::new());
            args.repository = cli.repository.clone();
            args.author = author.clone();
            args.reviewer = reviewer.clone();
            args.adapter = adapter.clone();
            args.review_adapter = review_adapter.clone();
            drain::run(&workspace, args, *workers, cli.json)
        }
        Commands::Runs => runs::run(&workspace, cli.json),
        Commands::Completion { .. } => unreachable!("handled before a workspace is resolved"),
        Commands::Prune { apply } => prune::run(&workspace, *apply, cli.json),
        Commands::Tui => tui::run(&workspace),
        Commands::Promote { run_id, branch } => {
            promote::run(&workspace, run_id, branch.as_deref(), cli.json)
        }
        Commands::Run {
            prompt,
            next,
            author,
            reviewer,
            adapter,
            review_adapter,
            base_ref,
            from,
            model,
        } => {
            let mut args = run::Args {
                prompt: prompt.clone().unwrap_or_default(),
                repository: cli.repository.clone(),
                author: author.clone(),
                reviewer: reviewer.clone(),
                adapter: adapter.clone(),
                review_adapter: review_adapter.clone(),
                base_ref: base_ref.clone(),
                from: from.clone(),
                model: model.clone(),
            };
            if *next {
                task_cmd::run_next(&workspace, args, cli.json)
            } else if args.prompt.is_empty() {
                Err("say what the agent should do, or `--next` to take it from the list".into())
            } else {
                args.repository = args.repository.take();
                run::run(&workspace, &args, cli.json)
            }
        }
    };

    match result {
        // A refused run is a correct outcome reported correctly, but the exit
        // code has to distinguish it: CI treats a non-zero exit as "not ready".
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Writes the completion script for one shell.
///
/// Taken from `Cli::command()` rather than written out, so it describes the
/// commands and flags this binary has — including the ones added after the
/// script was installed, once it is regenerated.
fn write_completion(shell: clap_complete::Shell, out: &mut impl std::io::Write) {
    let mut command = <Cli as clap::CommandFactory>::command();
    clap_complete::generate(shell, &mut command, "ostraka", out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ostraka").chain(args.iter().copied()))
    }

    #[test]
    fn a_completion_script_is_written_for_every_shell_clap_knows() {
        // Generated from the parser, so the assertion worth making is that the
        // parser is what reached the script: a command added later appears
        // without anybody editing anything, and one renamed stops appearing.
        // Asked of clap rather than listed here. `Shell` is `#[non_exhaustive]`
        // — upstream adds one without it being a breaking change — and a list
        // written out is the second description of something this whole change
        // exists to argue against keeping.
        let shells = <clap_complete::Shell as clap::ValueEnum>::value_variants();
        assert!(!shells.is_empty(), "clap knows no shells");
        for shell in shells.iter().copied() {
            let mut out: Vec<u8> = Vec::new();
            write_completion(shell, &mut out);
            let text = String::from_utf8(out).expect("utf-8");
            assert!(!text.is_empty(), "{shell} wrote nothing");
            assert!(text.contains("ostraka"), "{shell} did not name the binary");
            for command in ["run", "promote", "prune", "adapters", "replay"] {
                assert!(
                    text.contains(command),
                    "{shell} did not carry {command}:\n{text}"
                );
            }
            // And the flags, which are the half a hand-written script forgets.
            // Asked by name rather than by spelling: fish writes a long option
            // as `-l from` and the rest write `--from`, so looking for the
            // dashes tests which shell this is and not whether the option
            // arrived.
            assert!(
                text.contains("--from") || text.contains("-l from"),
                "{shell} did not carry the --from option:\n{text}"
            );
        }
    }

    #[test]
    fn from_and_base_ref_are_alternatives_and_only_one_of_them_may_be_given() {
        // The pair carries a real hazard: `--base-ref` has a default, and an
        // argument declared `conflicts_with` a defaulted one would reject every
        // invocation if clap counted the default as having been supplied. It
        // does not — but "it does not" is a fact about a dependency, and the
        // failure if it changed is that `--from` stops working entirely rather
        // than misbehaving somewhere visible. So it is pinned here.
        let cli = parse(&["run", "a task", "--from", "t1"]).expect("--from alone parses");
        let Some(Commands::Run { from, base_ref, .. }) = &cli.command else {
            panic!("not the run command")
        };
        assert_eq!(from.as_deref(), Some("t1"));
        assert_eq!(base_ref, run::BASE_REF, "the default still applies");

        // And naming both is refused, because one says which run and the other
        // says which ref: a run given both would have to ignore one of them.
        // `Cli` is not `Debug`, so the error is taken by hand.
        let err = match parse(&["run", "a task", "--from", "t1", "--base-ref", "other"]) {
            Ok(_) => panic!("--from and --base-ref were accepted together"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);

        // Neither is required, and the default is what a run gets.
        let cli = parse(&["run", "a task"]).expect("a bare run parses");
        let Some(Commands::Run { from, base_ref, .. }) = &cli.command else {
            panic!("not the run command")
        };
        assert_eq!(from.as_deref(), None);
        assert_eq!(base_ref, run::BASE_REF);
    }
}
