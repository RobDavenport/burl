//! Event logging and git operations for clean command.

use super::types::CleanupResult;
use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::locks::acquire_workflow_lock;
use serde_json::json;

/// Log the clean event and commit workflow state.
pub fn log_clean_event(
    ctx: &WorkflowContext,
    config: &Config,
    result: &CleanupResult,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock for the critical section
    let _workflow_lock = acquire_workflow_lock(ctx, "clean")?;

    // Append the clean event
    let event = Event::new(EventAction::Clean).with_details(json!({
        "removed_count": result.removed_count,
        "skipped_count": result.skipped_count,
    }));

    append_event(ctx, &event)?;

    // Commit if auto-commit is enabled
    if config.workflow_auto_commit {
        commit_clean(ctx, result)?;

        // Push if auto-push is enabled
        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the clean operation to the workflow branch.
fn commit_clean(ctx: &WorkflowContext, result: &CleanupResult) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage clean changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("burl clean: removed {} worktree(s)", result.removed_count);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit clean: {}", e)))?;

    Ok(())
}

/// Push the workflow branch to the remote.
fn push_workflow_branch(ctx: &WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}
