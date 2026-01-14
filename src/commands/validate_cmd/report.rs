//! QA Report writing and git operations.

use crate::config::Config;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::locks::acquire_workflow_lock;
use crate::task::TaskFile;
use serde_json::json;

/// Write QA Report entry to task file and append validate event.
pub fn write_qa_report_and_event(
    ctx: &crate::context::WorkflowContext,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    task_id: &str,
    passed: bool,
    summary: &str,
    config: &Config,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock for state mutation
    let _workflow_lock = acquire_workflow_lock(ctx, "validate")?;

    // Append to QA Report in task file
    task_file.append_to_qa_report(summary);

    // Atomically write updated task file
    task_file.save(task_path)?;

    // Append validate event
    let event = Event::new(EventAction::Validate)
        .with_task(task_id)
        .with_details(json!({
            "passed": passed,
            "title": task_file.frontmatter.title
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_validate(ctx, task_id, passed)?;

        // Push if auto-push enabled
        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the validation result to the workflow branch.
fn commit_validate(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    passed: bool,
) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage validate changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let status = if passed { "passed" } else { "failed" };
    let commit_msg = format!("Validate task {}: {}", task_id, status);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit validate: {}", e)))?;

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
