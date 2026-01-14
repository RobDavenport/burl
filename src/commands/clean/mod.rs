//! Implementation of the `burl clean` command.
//!
//! Cleans up workflow artifacts safely:
//! - Worktrees for completed tasks (in DONE bucket)
//! - Orphan worktrees (directories in `.worktrees/` not referenced by any task)
//!
//! # Safety
//!
//! - Default behavior is dry-run (prints what would be removed)
//! - Requires `--yes` to actually perform deletions
//! - Only operates on directories under the configured worktrees root (`.worktrees/`)
//! - Never deletes branches by default
//! - Never allows path traversal (`..` in paths)
//!
//! # Logging
//!
//! Appends a `clean` event with summary after deletions.
//! If `workflow_auto_commit: true`, commits the workflow branch.
//! If `workflow_auto_push: true`, pushes the workflow branch.

mod display;
mod execution;
mod logging;
mod planning;
mod types;

#[cfg(test)]
mod tests;

use crate::cli::CleanArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::Result;

use display::print_cleanup_plan;
use execution::execute_cleanup;
use logging::log_clean_event;
use planning::build_cleanup_plan;

/// Execute the `burl clean` command.
///
/// Cleans up completed and orphan worktrees with safety checks.
///
/// # Behavior
///
/// - Without `--yes`: dry-run mode, prints what would be removed
/// - With `--yes`: performs actual deletions
/// - `--completed`: only clean completed task worktrees
/// - `--orphans`: only clean orphan worktrees
/// - Neither flag: clean both completed and orphans
pub fn cmd_clean(args: CleanArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build the cleanup plan
    let plan = build_cleanup_plan(&ctx, &args)?;

    // Check if there's anything to clean
    let total_count = plan.completed_worktrees.len()
        + plan.orphan_worktrees.len()
        + plan.orphan_directories.len();

    if total_count == 0 {
        println!("No cleanup candidates found.");
        return Ok(());
    }

    // Print the plan
    print_cleanup_plan(&plan, &ctx.repo_root);

    // If dry-run (no --yes), just exit
    if !args.yes {
        println!();
        println!("Dry-run mode: no changes made.");
        println!("Run with --yes to perform the cleanup.");
        return Ok(());
    }

    // Perform the cleanup
    let result = execute_cleanup(&ctx, &plan)?;

    // Log the clean event if anything was removed
    if result.removed_count > 0 {
        log_clean_event(&ctx, &config, &result)?;
    }

    // Print summary
    println!();
    println!("Cleanup complete:");
    println!("  Removed: {} item(s)", result.removed_count);
    if result.skipped_count > 0 {
        println!("  Skipped: {} item(s)", result.skipped_count);
        for (path, reason) in &result.skipped {
            println!("    - {}: {}", path.display(), reason);
        }
    }

    Ok(())
}
