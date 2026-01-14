//! Git operations for the init command.
//!
//! Handles committing the workflow structure and pushing to the remote.

use crate::config::Config;
use crate::context::{WorkflowContext, DEFAULT_WORKFLOW_BRANCH};
use crate::error::{BurlError, Result};
use crate::git::run_git;

/// Commit the workflow structure to the workflow branch.
pub(super) fn commit_workflow_structure(
    ctx: &WorkflowContext,
    is_new_worktree: bool,
) -> Result<()> {
    // Check if there are any changes to commit
    let status = run_git(&ctx.workflow_worktree, &["status", "--porcelain"])?;

    if status.stdout.is_empty() {
        // Nothing to commit
        return Ok(());
    }

    // Stage all workflow files
    run_git(&ctx.workflow_worktree, &["add", ".workflow/"])?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;

    if staged.stdout.is_empty() {
        // Nothing was staged (maybe only untracked files in locks/)
        return Ok(());
    }

    // Create commit message
    let commit_msg = if is_new_worktree {
        "Initialize burl workflow structure\n\nCreated:\n- .workflow/READY/\n- .workflow/DOING/\n- .workflow/QA/\n- .workflow/DONE/\n- .workflow/BLOCKED/\n- .workflow/events/\n- .workflow/config.yaml\n- .workflow/.gitignore"
    } else {
        "Update burl workflow structure"
    };

    run_git(&ctx.workflow_worktree, &["commit", "-m", commit_msg]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to commit workflow structure: {}\n\n\
             You may need to configure git user.name and user.email:\n\
             git config user.name \"Your Name\"\n\
             git config user.email \"you@example.com\"",
            e
        ))
    })?;

    Ok(())
}

/// Push the workflow branch to the remote.
pub(super) fn push_workflow_branch(ctx: &WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", "-u", &config.remote, DEFAULT_WORKFLOW_BRANCH],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to push workflow branch: {}\n\n\
             You can push manually with:\n\
             git -C {} push -u {} {}",
            e,
            ctx.workflow_worktree.display(),
            config.remote,
            DEFAULT_WORKFLOW_BRANCH
        ))
    })?;

    Ok(())
}
