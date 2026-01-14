//! Validation logic for the init command.
//!
//! Ensures that if `.burl` already exists, it's a valid git worktree
//! and is checked out to the correct workflow branch.

use crate::context::{WorkflowContext, DEFAULT_WORKFLOW_BRANCH};
use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::fs;

/// Validate that if `.burl` exists, it's a valid git worktree.
pub(super) fn validate_existing_workflow_dir(ctx: &WorkflowContext) -> Result<()> {
    let workflow_path = &ctx.workflow_worktree;

    if !workflow_path.exists() {
        return Ok(());
    }

    // Check if it's a git worktree by looking for the .git file/directory
    let git_path = workflow_path.join(".git");

    if !git_path.exists() {
        return Err(BurlError::UserError(format!(
            "directory '{}' exists but is not a git worktree.\n\n\
             To fix this, either:\n\
             1. Delete or rename '{}' and run `burl init` again\n\
             2. Or manually set up the worktree with:\n\
                git worktree add {} {}",
            workflow_path.display(),
            workflow_path.display(),
            workflow_path.display(),
            DEFAULT_WORKFLOW_BRANCH
        )));
    }

    // If .git exists, check if it's a valid worktree or directory for this repo
    if git_path.is_file() {
        // It's a worktree - check if it points to the same repo
        let git_content = fs::read_to_string(&git_path).map_err(|e| {
            BurlError::UserError(format!("failed to read '{}': {}", git_path.display(), e))
        })?;

        if !git_content.starts_with("gitdir:") {
            return Err(BurlError::UserError(format!(
                "directory '{}' has an invalid .git file.\n\n\
                 To fix this, delete or rename '{}' and run `burl init` again.",
                workflow_path.display(),
                workflow_path.display()
            )));
        }

        // Verify the worktree is checked out to the workflow branch
        verify_worktree_branch(ctx)?;
    } else if git_path.is_dir() {
        // It's a full git repo, not a worktree - this shouldn't happen
        return Err(BurlError::UserError(format!(
            "directory '{}' contains a full git repository, not a worktree.\n\n\
             The workflow directory should be a git worktree, not a separate repository.\n\
             To fix this, delete or rename '{}' and run `burl init` again.",
            workflow_path.display(),
            workflow_path.display()
        )));
    }

    Ok(())
}

/// Verify that the existing workflow worktree is checked out to the workflow branch.
fn verify_worktree_branch(ctx: &WorkflowContext) -> Result<()> {
    let output = run_git(
        &ctx.workflow_worktree,
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )?;
    let current_branch = output.stdout.trim();

    if current_branch != DEFAULT_WORKFLOW_BRANCH {
        return Err(BurlError::UserError(format!(
            "workflow worktree is checked out to '{}', expected '{}'.\n\n\
             To fix this, either:\n\
             1. Checkout the correct branch: git -C {} checkout {}\n\
             2. Or delete '{}' and run `burl init` again.",
            current_branch,
            DEFAULT_WORKFLOW_BRANCH,
            ctx.workflow_worktree.display(),
            DEFAULT_WORKFLOW_BRANCH,
            ctx.workflow_worktree.display()
        )));
    }

    Ok(())
}
