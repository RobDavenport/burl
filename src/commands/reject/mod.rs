//! Implementation of the `burl reject` command.
//!
//! This module implements the QA -> READY (or BLOCKED) transition:
//! - Verify task is in QA
//! - Verify --reason is provided and non-empty
//! - Increment qa_attempts
//! - Append reason to QA Report with timestamp and actor
//! - Apply attempt policy (move to BLOCKED if max attempts exceeded)
//! - Preserve branch and worktree (no cleanup)
//! - Append reject event and commit workflow branch
//!
//! # Transaction Steps
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Verify task is in QA
//! 3. Verify --reason is non-empty
//! 4. Acquire `workflow.lock` for workflow-state mutation
//! 5. Increment qa_attempts
//! 6. Append reason to QA Report with timestamp and actor
//! 7. Check attempt policy: if qa_attempts >= qa_max_attempts, move to BLOCKED
//! 8. Optional: boost priority on retry if configured
//! 9. Move QA -> READY (or BLOCKED)
//! 10. Clear submitted_at for rework
//! 11. Append reject event and commit workflow branch
//! 12. If workflow_auto_push, push the workflow branch
//! 13. Release locks

mod git_ops;
#[cfg(test)]
mod tests;

use crate::cli::RejectArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, validate_task_id};
use chrono::Utc;
use serde_json::json;

use git_ops::{commit_reject, push_workflow_branch};

/// Get the actor string for event metadata and QA Report.
fn get_actor_string() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!("{}@{}", user, host)
}

/// Execute the `burl reject` command.
///
/// Rejects a task in QA by incrementing qa_attempts, appending the rejection reason,
/// and moving the task to READY (or BLOCKED if max attempts exceeded).
///
/// # Exit Codes
///
/// - 0: Success
/// - 1: User error (task not in QA, empty reason, invalid config)
/// - 4: Lock contention
pub fn cmd_reject(args: RejectArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // ========================================================================
    // Phase 1: Validate arguments
    // ========================================================================

    // Verify --reason is non-empty
    let reason = args.reason.trim();
    if reason.is_empty() {
        return Err(BurlError::UserError(
            "rejection reason cannot be empty.\n\n\
             Usage: burl reject TASK-ID --reason \"detailed reason for rejection\"\n\n\
             The reason should explain what needs to be fixed so the task can be reworked."
                .to_string(),
        ));
    }

    // ========================================================================
    // Phase 2: Task Resolution and Validation
    // ========================================================================

    // Build task index
    let index = TaskIndex::build(&ctx)?;

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
             Only tasks in QA can be rejected.\n\
             Use `burl status` to see tasks in each bucket.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 3: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "reject")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 4: Workflow state mutation (requires workflow lock)
    // ========================================================================

    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(&ctx, "reject")?;

    // Increment qa_attempts
    task_file.increment_qa_attempts();
    let qa_attempts = task_file.frontmatter.qa_attempts;

    // Get actor for QA Report
    let actor = get_actor_string();
    let now = Utc::now();

    // Append rejection reason to QA Report with timestamp and actor
    let rejection_entry = format!(
        "### Rejection: {}\n\n\
         **Actor:** {}\n\
         **Attempt:** {}\n\
         **Reason:** {}\n",
        now.format("%Y-%m-%d %H:%M:%S UTC"),
        actor,
        qa_attempts,
        reason
    );
    task_file.append_to_qa_report(&rejection_entry);

    // Determine destination bucket based on attempt policy
    let (destination_bucket, blocked_reason) = if qa_attempts >= config.qa_max_attempts {
        // Max attempts reached - move to BLOCKED
        let blocked_reason = format!(
            "max QA attempts reached ({}/{})",
            qa_attempts, config.qa_max_attempts
        );
        ("BLOCKED", Some(blocked_reason))
    } else {
        // Still have attempts left - move to READY
        // Apply priority boost if configured
        if config.auto_priority_boost_on_retry && task_file.frontmatter.priority != "high" {
            task_file.frontmatter.priority = "high".to_string();
        }
        ("READY", None)
    };

    // Clear submitted_at for rework
    task_file.frontmatter.submitted_at = None;

    // Save the updated task file
    task_file.save(&task_info.path)?;

    // Move task QA -> destination bucket
    let filename = task_info
        .path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let destination_path = ctx.bucket_path(destination_bucket).join(filename);

    crate::fs::move_file(&task_info.path, &destination_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from QA to {}: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            destination_bucket,
            e,
            task_info.path.display(),
            destination_path.display()
        ))
    })?;

    // Append reject event
    let event = Event::new(EventAction::Reject)
        .with_task(&task_id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "reason": reason,
            "qa_attempts": qa_attempts,
            "max_attempts": config.qa_max_attempts,
            "destination": destination_bucket,
            "blocked_reason": blocked_reason,
            "priority_boosted": config.auto_priority_boost_on_retry && destination_bucket == "READY" && task_file.frontmatter.priority == "high",
        }));
    append_event(&ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_reject(&ctx, &task_id, reason, destination_bucket)?;

        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // ========================================================================
    // Phase 5: Print results
    // ========================================================================

    println!();
    println!("Rejected task: {}", task_id);
    println!("  Title:       {}", task_file.frontmatter.title);
    println!("  Reason:      {}", reason);
    println!("  From:        QA");
    println!("  To:          {}", destination_bucket);
    println!("  QA Attempts: {}/{}", qa_attempts, config.qa_max_attempts);

    if destination_bucket == "BLOCKED" {
        println!();
        println!(
            "This task has exceeded the maximum QA attempts ({}).",
            config.qa_max_attempts
        );
        println!("It has been moved to BLOCKED and requires manual intervention.");
    } else {
        if config.auto_priority_boost_on_retry {
            println!(
                "  Priority:    {} (boosted)",
                task_file.frontmatter.priority
            );
        }
        println!();
        println!("The task branch and worktree have been preserved for rework.");
        if let Some(worktree) = &task_file.frontmatter.worktree {
            println!("  Worktree: {}", worktree);
        }
    }

    Ok(())
}
