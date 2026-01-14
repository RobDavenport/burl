//! Implementation of the `burl submit` command.
//!
//! This module implements the DOING -> QA transition with deterministic validation gates:
//! - Scope validation: ensures changes are within allowed paths
//! - Stub detection: detects incomplete code patterns in added lines
//!
//! # Transaction Steps
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Verify task is in DOING with valid worktree/branch/base_sha
//! 3. Verify at least one commit exists since base_sha
//! 4. Run validations (scope + stubs) against `{base_sha}..HEAD`
//! 5. If push_task_branch_on_submit: push task branch to remote
//! 6. Acquire `workflow.lock` for workflow-state mutation
//! 7. Set submitted_at, move DOING -> QA
//! 8. Append submit event and commit workflow branch
//! 9. Release locks

use crate::cli::SubmitArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::diff::{added_lines, changed_files};
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git_worktree::get_current_branch;
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::validate::{validate_scope, validate_stubs_with_config};
use crate::workflow::{TaskIndex, validate_task_id};
use chrono::Utc;
use serde_json::json;

mod helpers;
#[cfg(test)]
mod tests;

use helpers::{commit_submit, count_commits_since, push_task_branch, push_workflow_branch};

/// Execute the `burl submit` command.
///
/// Submits a claimed task from DOING -> QA after passing validation gates.
///
/// # Exit Codes
///
/// - 0: Success
/// - 1: User error (task not in DOING, missing commits, invalid state)
/// - 2: Validation failure (scope/stub violations)
/// - 3: Git error (push failed, etc.)
/// - 4: Lock contention
pub fn cmd_submit(args: SubmitArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;

    // ========================================================================
    // Phase 1: Task Resolution
    // ========================================================================

    let task_id = match &args.task_id {
        Some(id) => validate_task_id(id)?,
        None => {
            // Find the current task by looking for a single task in DOING
            // assigned to this user, or error if ambiguous
            let doing_tasks: Vec<_> = index.tasks_in_bucket("DOING");
            if doing_tasks.is_empty() {
                return Err(BurlError::UserError(
                    "no tasks in DOING. Claim a task first with `burl claim`.".to_string(),
                ));
            }
            if doing_tasks.len() > 1 {
                let ids: Vec<_> = doing_tasks.iter().map(|t| t.id.as_str()).collect();
                return Err(BurlError::UserError(format!(
                    "multiple tasks in DOING: {}.\n\n\
                     Specify which task to submit: `burl submit <TASK-ID>`",
                    ids.join(", ")
                )));
            }
            doing_tasks[0].id.clone()
        }
    };

    // Re-lookup task info
    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Use `burl status` to see available tasks.",
            task_id
        ))
    })?;

    // Verify task is in DOING bucket
    if task_info.bucket != "DOING" {
        return Err(BurlError::UserError(format!(
            "task '{}' is not in DOING (currently in {}).\n\n\
             Only tasks in DOING can be submitted.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "submit")?;

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

    let current_branch = get_current_branch(&worktree_path)?;
    if current_branch != expected_branch {
        return Err(BurlError::UserError(format!(
            "task worktree is on branch '{}', but task expects branch '{}'.\n\n\
             Run `burl doctor` to diagnose or re-claim the task.",
            current_branch, expected_branch
        )));
    }

    // Check base_sha is recorded
    let base_sha = task_file.frontmatter.base_sha.clone().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded base_sha.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    // ========================================================================
    // Phase 4: Verify at least one commit since base_sha
    // ========================================================================

    let commit_count = count_commits_since(&worktree_path, &base_sha)?;
    if commit_count == 0 {
        return Err(BurlError::UserError(format!(
            "no commits on task branch since base_sha ({}).\n\n\
             Make changes and commit them before submitting:\n\
             1. cd {}\n\
             2. Make your changes\n\
             3. git add . && git commit -m \"Your message\"\n\
             4. burl submit {}",
            &base_sha[..8.min(base_sha.len())],
            worktree_path.display(),
            task_id
        )));
    }

    // ========================================================================
    // Phase 5: Run validations (scope + stubs)
    // ========================================================================

    // Get changed files and added lines for validation
    let changed = changed_files(&worktree_path, &base_sha)?;
    let added = added_lines(&worktree_path, &base_sha)?;

    // Validate scope
    let scope_result = validate_scope(&task_file.frontmatter, &changed)?;
    if !scope_result.passed {
        let error_msg = scope_result.format_error(&task_id);
        return Err(BurlError::ValidationError(error_msg));
    }

    // Validate stubs
    let stub_result = validate_stubs_with_config(&config, &added)?;
    if !stub_result.passed {
        let error_msg = stub_result.format_error();
        return Err(BurlError::ValidationError(error_msg));
    }

    // ========================================================================
    // Phase 6: Push task branch (if configured)
    // ========================================================================

    if config.push_task_branch_on_submit {
        push_task_branch(&worktree_path, &config.remote, &expected_branch)?;
    }

    // ========================================================================
    // Phase 7: Workflow State Mutation (under workflow lock)
    // ========================================================================

    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(&ctx, "submit")?;

    // Update task frontmatter
    let now = Utc::now();
    task_file.set_submitted(now);

    // Atomically write updated task file
    task_file.save(&task_info.path)?;

    // Move task file DOING -> QA atomically
    let filename = task_info
        .path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let qa_path = ctx.bucket_path("QA").join(filename);

    crate::fs::move_file(&task_info.path, &qa_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from DOING to QA: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_info.path.display(),
            qa_path.display()
        ))
    })?;

    // ========================================================================
    // Phase 8: Event Logging and Commit
    // ========================================================================

    // Append submit event
    let event = Event::new(EventAction::Submit)
        .with_task(&task_info.id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "branch": expected_branch,
            "base_sha": base_sha,
            "commit_count": commit_count,
            "files_changed": changed.len(),
            "lines_added": added.len(),
            "pushed": config.push_task_branch_on_submit
        }));
    append_event(&ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_submit(&ctx, &task_info.id, &task_file.frontmatter.title)?;

        // Push if auto-push enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // ========================================================================
    // Phase 9: Output
    // ========================================================================

    println!("Submitted task: {}", task_info.id);
    println!("  Title:         {}", task_file.frontmatter.title);
    println!("  From:          DOING");
    println!("  To:            QA");
    println!("  Commits:       {}", commit_count);
    println!("  Files changed: {}", changed.len());
    if config.push_task_branch_on_submit {
        println!(
            "  Pushed:        {} -> {}/{}",
            expected_branch, config.remote, expected_branch
        );
    }
    println!();
    println!("Task is now awaiting review in QA.");

    Ok(())
}
