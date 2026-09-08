//! The `ostraka` command.

mod adapters;
mod check;
mod init;
mod init_cmd;
mod promote;
mod prune;
mod replay;
mod run;
mod runs;
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

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate the project config and every adapter profile.
    Check,

    /// Write the files a project needs to be run by Ostraka.
    Init {
        /// Rewrite files that already exist. Off by default: what is there is
        /// somebody's, and a setup command should not be how they lose it.
        #[arg(long)]
        force: bool,
    },

    /// List adapter profiles and whether each one can run here.
    Adapters,

    /// Run one task: isolate, execute, gate, review, record.
    Run {
        /// What the agent should do.
        prompt: String,

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

        /// Model hint passed through to the adapter.
        #[arg(long)]
        model: Option<String>,
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
    let here = cli.workspace.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace = workspace::Workspace::at(&here);

    let result = match &cli.command {
        Commands::Check => check::run(&workspace, cli.json),
        Commands::Init { force } => init_cmd::run(&here, *force, cli.json),
        Commands::Adapters => adapters::run(&workspace, cli.json),
        Commands::Replay { run_id } => replay::run(&workspace, run_id, cli.json),
        Commands::Runs => runs::run(&workspace, cli.json),
        Commands::Prune { apply } => prune::run(&workspace, *apply, cli.json),
        Commands::Tui => tui::run(&workspace),
        Commands::Promote { run_id, branch } => {
            promote::run(&workspace, run_id, branch.as_deref(), cli.json)
        }
        Commands::Run {
            prompt,
            author,
            reviewer,
            adapter,
            review_adapter,
            base_ref,
            model,
        } => run::run(
            &workspace,
            &run::Args {
                prompt: prompt.clone(),
                repository: cli.repository.clone(),
                author: author.clone(),
                reviewer: reviewer.clone(),
                adapter: adapter.clone(),
                review_adapter: review_adapter.clone(),
                base_ref: base_ref.clone(),
                model: model.clone(),
            },
            cli.json,
        ),
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
