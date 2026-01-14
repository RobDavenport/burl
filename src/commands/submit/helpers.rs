//! Helper functions for the submit command.

use crate::config::Config;
use crate::error::{BurlError, Result};
use crate::git::run_git;

/// Count commits since base_sha in the worktree.
pub(super) fn count_commits_since(worktree: &std::path::Path, base_sha: &str) -> Result<u32> {
    let range = format!("{}..HEAD", base_sha);
    let output = run_git(worktree, &["rev-list", "--count", &range])?;

    output.stdout.trim().parse::<u32>().map_err(|e| {
        BurlError::GitError(format!(
            "failed to parse commit count '{}': {}",
            output.stdout.trim(),
            e
        ))
    })
}

/// Push the task branch to the remote.
pub(super) fn push_task_branch(
    worktree: &std::path::Path,
    remote: &str,
    branch: &str,
) -> Result<()> {
    run_git(worktree, &["push", remote, branch]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to push task branch '{}' to '{}': {}\n\n\
             Fix any issues and try again, or set `push_task_branch_on_submit: false` in config.",
            branch, remote, e
        ))
    })?;
    Ok(())
}

/// Commit the submit to the workflow branch.
pub(super) fn commit_submit(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    title: &str,
) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage submit changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("Submit task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit submit: {}", e)))?;

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
