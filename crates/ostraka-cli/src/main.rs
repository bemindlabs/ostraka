//! The `ostraka` command.

mod adapters;
mod check;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let project = cli.project.clone().unwrap_or_else(|| PathBuf::from("."));

    let result = match cli.command {
        Commands::Check => check::run(&project, cli.json),
        Commands::Adapters => adapters::run(&project, cli.json),
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
