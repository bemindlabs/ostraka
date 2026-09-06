//! The `ostraka` command.

mod adapters;
mod check;
mod project;
mod promote;
mod replay;
mod run;

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
    /// Project directory. Defaults to the current directory.
    #[arg(long, global = true)]
    project: Option<PathBuf>,

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

    /// List adapter profiles and whether each one can run here.
    Adapters,

    /// Run one task: isolate, execute, gate, review, record.
    Run {
        /// What the agent should do.
        prompt: String,

        /// Identity accountable for the change.
        #[arg(long, default_value = "author")]
        author: String,

        /// Identity that reviews it. Must differ from the author.
        #[arg(long, default_value = "reviewer")]
        reviewer: String,

        /// Adapter profile that writes the change.
        #[arg(long)]
        adapter: Option<String>,

        /// Adapter profile that reviews it. Must differ from `--adapter`.
        #[arg(long)]
        review_adapter: Option<String>,

        /// Git ref the worktree branches from.
        #[arg(long, default_value = "HEAD")]
        base_ref: String,

        /// Model hint passed through to the adapter.
        #[arg(long)]
        model: Option<String>,
    },

    /// Give an approved run a branch of its own. Merges nothing.
    Promote {
        /// Run id, as printed by `run`.
        run_id: String,

        /// Branch to create. Defaults to `promoted/<run-id>`.
        #[arg(long)]
        branch: Option<String>,
    },

    /// Read a finished run back from its record.
    Replay {
        /// Run id, as printed by `run`.
        run_id: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let project = cli.project.clone().unwrap_or_else(|| PathBuf::from("."));

    let result = match &cli.command {
        Commands::Check => check::run(&project, cli.json),
        Commands::Adapters => adapters::run(&project, cli.json),
        Commands::Replay { run_id } => replay::run(&project, run_id, cli.json),
        Commands::Promote { run_id, branch } => {
            promote::run(&project, run_id, branch.as_deref(), cli.json)
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
            &project,
            &run::Args {
                prompt: prompt.clone(),
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
