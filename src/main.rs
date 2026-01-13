//! Burl: Minimal file-based workflow orchestrator for agentic coding pipelines.
//!
//! This is the main entry point for the `burl` CLI. It parses arguments,
//! dispatches to the appropriate command handler, and handles errors with
//! proper exit codes.

mod cli;
mod commands;
pub mod config;
pub mod context;
pub mod diff;
pub mod error;
pub mod events;
pub mod exit_codes;
pub mod fs;
pub mod git;
pub mod git_worktree;
pub mod locks;
pub mod task;
pub mod validate;
pub mod workflow;

use cli::Cli;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse_args();

    match commands::dispatch(cli.command) {
        Ok(()) => ExitCode::from(exit_codes::SUCCESS as u8),
        Err(err) => {
            // Print user-actionable error message to stderr
            eprintln!("Error: {}", err);

            // Return appropriate exit code
            ExitCode::from(err.exit_code() as u8)
        }
    }
}
