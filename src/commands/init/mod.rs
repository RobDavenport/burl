//! Implementation of the `burl init` command.
//!
//! This module bootstraps (or reattaches) the canonical workflow worktree
//! and creates the on-branch workflow state directory structure.
//!
//! # What `burl init` does
//!
//! 1. Creates or attaches the workflow worktree (default: `.burl/` on branch `burl`)
//! 2. Creates workflow state directories: READY/, DOING/, QA/, DONE/, BLOCKED/, events/
//! 3. Creates `config.yaml` template (if missing)
//! 4. Creates `.gitignore` with `locks/` entry
//! 5. Ensures `locks/` directory exists locally (untracked)
//! 6. Creates `.worktrees/` directory at repo root (local, untracked)
//! 7. Optionally adds `.burl/` and `.worktrees/` to `.git/info/exclude`
//! 8. Commits the scaffolding to the workflow branch (if `workflow_auto_commit` is true)

mod git_ops;
mod scaffolding;
mod validation;
mod worktree;

#[cfg(test)]
mod tests;

use crate::config::Config;
use crate::context::{resolve_context, DEFAULT_WORKFLOW_BRANCH};
use crate::error::Result;
use crate::events::{append_event, Event, EventAction};
use crate::locks;
use serde_json::json;

use git_ops::*;
use scaffolding::*;
use validation::*;
use worktree::*;

/// Status buckets that will be created under `.workflow/`.
const BUCKETS: &[&str] = &["READY", "DOING", "QA", "DONE", "BLOCKED"];

/// Execute the `burl init` command.
///
/// This command is **idempotent**: running it multiple times will not error
/// and will not cause destructive changes to existing workflow state.
pub fn cmd_init() -> Result<()> {
    let ctx = resolve_context()?;

    // Check if .burl exists but is not a valid git worktree
    validate_existing_workflow_dir(&ctx)?;

    // Create or attach the workflow worktree
    let worktree_created = ensure_workflow_worktree(&ctx)?;

    // Acquire workflow lock for the scaffolding phase
    // This prevents concurrent init operations
    let _lock_guard = locks::acquire_workflow_lock(&ctx, "init")?;

    // Create the workflow state directory structure
    create_workflow_structure(&ctx)?;

    // Create the .worktrees directory at repo root (untracked)
    create_worktrees_dir(&ctx)?;

    // Add .burl/ and .worktrees/ to .git/info/exclude
    add_to_git_exclude(&ctx)?;

    // Load config to check if auto-commit is enabled
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Append init event before committing (while holding workflow.lock)
    let event = Event::new(EventAction::Init).with_details(json!({
        "workflow_branch": DEFAULT_WORKFLOW_BRANCH,
        "workflow_worktree": ctx.workflow_worktree.display().to_string(),
        "auto_commit": config.workflow_auto_commit,
        "auto_push": config.workflow_auto_push
    }));
    append_event(&ctx, &event)?;

    // Commit the workflow structure if auto-commit is enabled
    if config.workflow_auto_commit {
        commit_workflow_structure(&ctx, worktree_created)?;

        // Push if auto-push is enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // Print success message
    println!("Initialized burl workflow.");
    println!();
    println!("Workflow worktree: {}", ctx.workflow_worktree.display());
    println!("Workflow branch:   {}", DEFAULT_WORKFLOW_BRANCH);
    println!();
    println!("Created directories:");
    for bucket in BUCKETS {
        println!("  .burl/.workflow/{}/", bucket);
    }
    println!("  .burl/.workflow/events/");
    println!("  .burl/.workflow/locks/  (untracked)");
    println!("  .worktrees/             (untracked)");
    println!();
    println!("You can now add tasks with `burl add \"task title\"`.");

    Ok(())
}
