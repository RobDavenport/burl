//! Worktree management for the init command.
//!
//! Handles creating or attaching the workflow worktree,
//! checking for existing branches, and getting the current branch.

use crate::context::{DEFAULT_WORKFLOW_BRANCH, WorkflowContext};
use crate::error::{BurlError, Result};
use crate::git::run_git;

/// Ensure the workflow worktree exists and is properly set up.
/// Returns true if a new worktree was created, false if it already existed.
pub(super) fn ensure_workflow_worktree(ctx: &WorkflowContext) -> Result<bool> {
    let workflow_path = &ctx.workflow_worktree;

    // If worktree already exists and is valid, we're done
    if workflow_path.exists() {
        return Ok(false);
    }

    // Check if the workflow branch already exists
    let branch_exists = check_branch_exists(ctx, DEFAULT_WORKFLOW_BRANCH)?;

    if branch_exists {
        // Branch exists - attach worktree to existing branch
        run_git(
            &ctx.repo_root,
            &[
                "worktree",
                "add",
                workflow_path.to_str().unwrap(),
                DEFAULT_WORKFLOW_BRANCH,
            ],
        )
        .map_err(|e| {
            BurlError::GitError(format!(
                "failed to create worktree at '{}': {}\n\n\
                 Try manually: git worktree add {} {}",
                workflow_path.display(),
                e,
                workflow_path.display(),
                DEFAULT_WORKFLOW_BRANCH
            ))
        })?;
    } else {
        // Branch doesn't exist - create new branch and worktree
        // Get the current branch name to use as the base
        let base_branch = get_current_branch(ctx)?;

        run_git(
            &ctx.repo_root,
            &[
                "worktree",
                "add",
                "-b",
                DEFAULT_WORKFLOW_BRANCH,
                workflow_path.to_str().unwrap(),
                &base_branch,
            ],
        )
        .map_err(|e| {
            BurlError::GitError(format!(
                "failed to create worktree with new branch at '{}': {}\n\n\
                 Try manually: git worktree add -b {} {} {}",
                workflow_path.display(),
                e,
                DEFAULT_WORKFLOW_BRANCH,
                workflow_path.display(),
                base_branch
            ))
        })?;
    }

    Ok(true)
}

/// Check if a branch exists in the repository.
pub(super) fn check_branch_exists(ctx: &WorkflowContext, branch: &str) -> Result<bool> {
    let result = run_git(
        &ctx.repo_root,
        &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
    );

    Ok(result.is_ok())
}

/// Get the current branch name.
pub(super) fn get_current_branch(ctx: &WorkflowContext) -> Result<String> {
    let output = run_git(&ctx.repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = output.stdout.trim();

    // Handle detached HEAD
    if branch == "HEAD" {
        // Fallback to using HEAD directly
        Ok("HEAD".to_string())
    } else {
        Ok(branch.to_string())
    }
}
