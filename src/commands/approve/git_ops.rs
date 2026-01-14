//! Git operations for the approve command.
//!
//! This module handles git operations like merge, push, and workflow state updates.

use crate::config::Config;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::git_worktree::cleanup_task_worktree;
use crate::locks::acquire_workflow_lock;
use crate::task::TaskFile;
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;

/// Merge the task branch into local main using --ff-only.
pub fn merge_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    branch: &str,
) -> Result<()> {
    // Checkout main in the repo root
    run_git(&ctx.repo_root, &["checkout", &config.main_branch]).map_err(|e| {
        BurlError::GitError(format!("failed to checkout {}: {}", config.main_branch, e))
    })?;

    // Attempt fast-forward merge
    let merge_result = run_git(&ctx.repo_root, &["merge", "--ff-only", branch]);

    if let Err(e) = merge_result {
        // Reject the task with non-FF merge reason
        reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("non-FF merge required: {}", e),
        )
    } else {
        Ok(())
    }
}

/// Push main to remote.
pub fn push_main(ctx: &crate::context::WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.repo_root,
        &["push", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to push {} to {}: {}\n\n\
             The merge was successful locally. You can push manually with:\n\
             git push {} {}",
            config.main_branch, config.remote, e, config.remote, config.main_branch
        ))
    })?;
    Ok(())
}

/// Cleanup worktree and branch (best-effort).
///
/// Returns true if cleanup failed, false if successful.
pub fn cleanup_worktree(
    ctx: &crate::context::WorkflowContext,
    branch: &str,
    worktree_path: &PathBuf,
) -> Result<bool> {
    let cleanup_failed = match crate::git::has_worktree_changes(worktree_path) {
        Ok(true) | Err(_) => {
            eprintln!(
                "Warning: task worktree has uncommitted changes; skipping cleanup to avoid data loss: {}",
                worktree_path.display()
            );
            true
        }
        Ok(false) => {
            let cleanup_result = cleanup_task_worktree(
                &ctx.repo_root,
                branch,
                Some(worktree_path.as_path()),
                true, // Force removal since changes are now merged
            );

            if let Err(e) = &cleanup_result {
                eprintln!("Warning: cleanup failed (will proceed anyway): {}", e);
            }

            cleanup_result.is_err()
        }
    };

    Ok(cleanup_failed)
}

/// Reject a task by moving it from QA to READY with a reason.
pub fn reject_task(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    reason: &str,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(ctx, "approve-reject")?;

    // Append rejection reason to QA Report
    let rejection_entry = format!(
        "### Rejection: {}\n\n**Reason:** {}\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        reason
    );
    task_file.append_to_qa_report(&rejection_entry);

    // Increment qa_attempts
    task_file.increment_qa_attempts();

    // Check if max attempts reached
    if task_file.frontmatter.qa_attempts >= config.qa_max_attempts {
        // Boost priority if configured
        if config.auto_priority_boost_on_retry && task_file.frontmatter.priority != "high" {
            task_file.frontmatter.priority = "high".to_string();
        }
    }

    // Clear submitted_at for re-work
    task_file.frontmatter.submitted_at = None;

    // Save the updated task file
    task_file.save(task_path)?;

    // Move task QA -> READY
    let filename = task_path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let ready_path = ctx.bucket_path("READY").join(filename);

    crate::fs::move_file(task_path, &ready_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from QA to READY: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_path.display(),
            ready_path.display()
        ))
    })?;

    // Append reject event
    let event = Event::new(EventAction::Reject)
        .with_task(task_id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "reason": reason,
            "qa_attempts": task_file.frontmatter.qa_attempts,
            "triggered_by": "approve"
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_reject(ctx, task_id, reason)?;

        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    println!();
    println!("Rejected task: {}", task_id);
    println!("  Title:       {}", task_file.frontmatter.title);
    println!("  Reason:      {}", reason);
    println!("  From:        QA");
    println!("  To:          READY");
    println!("  QA Attempts: {}", task_file.frontmatter.qa_attempts);
    println!();
    println!("The task branch and worktree have been preserved for rework.");

    // Return an error to signal that approval failed
    Err(BurlError::GitError(format!(
        "approval rejected: {}",
        reason
    )))
}

/// Complete the approval by updating workflow state.
pub fn complete_approval(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    cleanup_failed: bool,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(ctx, "approve")?;

    // Set completed_at
    let now = Utc::now();
    task_file.set_completed(now);

    // Append success to QA Report
    let success_entry = format!(
        "### Approved: {}\n\n**Result:** Merged to main\n",
        now.format("%Y-%m-%d %H:%M:%S UTC")
    );
    task_file.append_to_qa_report(&success_entry);

    // Save the updated task file
    task_file.save(task_path)?;

    // Move task QA -> DONE
    let filename = task_path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let done_path = ctx.bucket_path("DONE").join(filename);

    crate::fs::move_file(task_path, &done_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from QA to DONE: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_path.display(),
            done_path.display()
        ))
    })?;

    // Append approve event
    let event = Event::new(EventAction::Approve)
        .with_task(task_id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "branch": task_file.frontmatter.branch,
            "cleanup_failed": cleanup_failed,
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_approve(ctx, task_id, &task_file.frontmatter.title)?;

        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the approval to the workflow branch.
fn commit_approve(ctx: &crate::context::WorkflowContext, task_id: &str, title: &str) -> Result<()> {
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage approve changes: {}", e)))?;

    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    let commit_msg = format!("Approve task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit approve: {}", e)))?;

    Ok(())
}

/// Commit the rejection to the workflow branch.
fn commit_reject(ctx: &crate::context::WorkflowContext, task_id: &str, reason: &str) -> Result<()> {
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

    let commit_msg = format!("Reject task {} (approve): {}", task_id, short_reason);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit reject: {}", e)))?;

    Ok(())
}

/// Push the workflow branch to the remote.
fn push_workflow_branch(ctx: &crate::context::WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}
