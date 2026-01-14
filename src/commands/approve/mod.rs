//! Implementation of the `burl approve` command.
//!
//! This module implements the QA -> DONE transition with rebase, validation, and merge:
//! - Fetch and rebase onto origin/main
//! - Run validation against rebased base (origin/main..HEAD)
//! - Fast-forward merge into local main
//! - Cleanup worktree and branch
//!
//! # Transaction Steps (rebase_ff_only strategy)
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Verify task is in QA with valid worktree/branch
//! 3. Fetch origin/main
//! 4. Rebase task branch onto origin/main (conflict -> reject, move QA -> READY)
//! 5. Run validation against rebased base (origin/main..HEAD)
//! 6. Merge into local main using --ff-only (fail -> reject)
//! 7. Optionally push main if `push_main_on_approve: true`
//! 8. Cleanup worktree and branch (best-effort)
//! 9. Acquire `workflow.lock` for workflow-state mutation
//! 10. Set completed_at, move QA -> DONE
//! 11. Append approve event and commit workflow branch
//! 12. Release locks

mod git_ops;
mod strategies;
mod validation;

#[cfg(test)]
mod tests;

use crate::cli::ApproveArgs;
use crate::config::{Config, MergeStrategy};
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::locks::acquire_task_lock;
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, validate_task_id};

use strategies::{approve_ff_only, approve_rebase_ff_only};

/// Execute the `burl approve` command.
///
/// Approves a task in QA by rebasing, validating, merging to main, and moving to DONE.
///
/// # Exit Codes
///
/// - 0: Success
/// - 1: User error (task not in QA, missing state, invalid config)
/// - 2: Validation failure (scope/stub/build-test violations)
/// - 3: Git error (rebase conflict, non-FF merge)
/// - 4: Lock contention
pub fn cmd_approve(args: ApproveArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;

    // ========================================================================
    // Phase 1: Task Resolution and Validation
    // ========================================================================

    let task_id = validate_task_id(&args.task_id)?;

    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Use `burl status` to see available tasks.",
            task_id
        ))
    })?;

    // Verify task is in QA bucket
    if task_info.bucket != "QA" {
        return Err(BurlError::UserError(format!(
            "task '{}' is not in QA (currently in {}).\n\n\
             Only tasks in QA can be approved.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "approve")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 3: Verify task has required git state
    // ========================================================================

    let refs = crate::task_git::require_task_git_refs(
        &ctx,
        &task_id,
        task_file.frontmatter.branch.as_deref(),
        task_file.frontmatter.worktree.as_deref(),
    )?;

    let expected_branch = refs.branch;
    let worktree_path = refs.worktree_path;

    if !worktree_path.exists() {
        return Err(BurlError::UserError(format!(
            "task worktree does not exist at '{}'.\n\n\
             Run `burl doctor` to diagnose and repair this inconsistency.",
            worktree_path.display()
        )));
    }

    let current_branch = crate::git_worktree::get_current_branch(&worktree_path)?;
    if current_branch != expected_branch {
        return Err(BurlError::UserError(format!(
            "task worktree is on branch '{}', but task expects branch '{}'.\n\n\
             Remediation: checkout the correct branch with:\n\
             cd {} && git checkout {}",
            current_branch,
            expected_branch,
            worktree_path.display(),
            expected_branch
        )));
    }

    // ========================================================================
    // Phase 4: Strategy-based git operations
    // ========================================================================

    match config.merge_strategy {
        MergeStrategy::RebaseFfOnly => approve_rebase_ff_only(
            &ctx,
            &config,
            &task_info.id,
            &task_info.path,
            &mut task_file,
            &worktree_path,
            &expected_branch,
        ),
        MergeStrategy::FfOnly => approve_ff_only(
            &ctx,
            &config,
            &task_info.id,
            &task_info.path,
            &mut task_file,
            &worktree_path,
            &expected_branch,
        ),
        MergeStrategy::Manual => Err(BurlError::UserError(
            "merge_strategy 'manual' is not implemented in V1.\n\n\
             Use 'rebase_ff_only' (default) or 'ff_only' instead, or perform the merge manually."
                .to_string(),
        )),
    }
}
