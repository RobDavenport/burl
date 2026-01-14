//! Git operations for the reject command.
//!
//! This module contains git-related helpers for committing and pushing
//! the reject workflow state changes.

use crate::config::Config;
use crate::error::{BurlError, Result};
use crate::git::run_git;

/// Commit the rejection to the workflow branch.
pub(super) fn commit_reject(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    reason: &str,
    destination: &str,
) -> Result<()> {
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage reject changes: {}", e)))?;

    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Truncate reason for commit message
    let short_reason = if reason.len() > 50 {
        format!("{}...", &reason[..47])
    } else {
        reason.to_string()
    };

    let commit_msg = format!(
        "Reject task {} -> {}: {}",
        task_id, destination, short_reason
    );

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit reject: {}", e)))?;

    Ok(())
}

/// Push the workflow branch to the remote.
pub(super) fn push_workflow_branch(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}
