//! Implementation of the `burl claim` command.
//!
//! This module implements the race-safe, transactional claim operation that:
//! - Selects a claimable READY task (or claims explicit ID)
//! - Creates/reuses branch + worktree at `base_sha`
//! - Updates task metadata and moves READY -> DOING atomically
//!
//! # Transaction Steps
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Resolve `base_sha` (fetch origin/main first)
//! 3. Create/reuse branch and worktree
//! 4. Verify workflow worktree has no unexpected tracked modifications
//! 5. Acquire `workflow.lock` for workflow-state mutation
//! 6. Atomically update task frontmatter and move READY -> DOING
//! 7. Append claim event and commit workflow branch
//! 8. Release locks
//!
//! # Rollback
//!
//! If worktree creation fails after branch creation, delete the branch if it
//! was created in this attempt.

mod helpers;
mod scope;
mod selection;
#[cfg(test)]
mod tests;
mod transaction;

use crate::cli::ClaimArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git_worktree::{branch_exists, setup_task_worktree};
use crate::locks::{LockGuard, acquire_claim_lock, acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, slugify_title, validate_task_id};
use chrono::Utc;
use serde_json::json;

use helpers::{commit_claim, get_assignee_string, push_workflow_branch};
use scope::check_scope_conflicts;
use selection::{check_dependencies_satisfied, select_next_task_id};
use transaction::ClaimTransaction;

/// Execute the `burl claim` command.
///
/// Claims a READY task for work by:
/// 1. Selecting the task (explicit ID or next available)
/// 2. Checking dependencies and scope conflicts
/// 3. Creating/reusing branch and worktree
/// 4. Updating task metadata and moving to DOING
/// 5. Committing workflow state
pub fn cmd_claim(args: ClaimArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;

    // ========================================================================
    // Phase 1: Task Selection
    // ========================================================================

    // Acquire global claim lock if needed (when selecting next task)
    let _claim_lock: Option<LockGuard> = if args.task_id.is_none() && config.use_global_claim_lock {
        Some(acquire_claim_lock(&ctx)?)
    } else {
        None
    };

    let task_id = match &args.task_id {
        Some(id) => {
            // Explicit task ID provided
            validate_task_id(id)?
        }
        None => {
            // Select next available task
            let ready_tasks: Vec<&crate::workflow::TaskInfo> = index.tasks_in_bucket("READY");
            select_next_task_id(&ctx, &ready_tasks)?.ok_or_else(|| {
                BurlError::UserError(
                    "no claimable tasks in READY.\n\n\
                     All READY tasks may have unmet dependencies, or there are no READY tasks.\n\
                     Use `burl status` to see the workflow state."
                        .to_string(),
                )
            })?
        }
    };

    // Re-lookup task info now that we have the ID
    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Use `burl status` to see available tasks.",
            task_id
        ))
    })?;

    // Verify task is in READY bucket
    if task_info.bucket != "READY" {
        return Err(BurlError::UserError(format!(
            "task '{}' is not in READY (currently in {}).\n\n\
             Only tasks in READY can be claimed.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "claim")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 3: Dependency and Scope Checks
    // ========================================================================

    // Rebuild index since we now have the lock
    let index = TaskIndex::build(&ctx)?;

    // Check dependencies
    check_dependencies_satisfied(&task_file, &index)?;

    // Check scope conflicts with DOING tasks
    check_scope_conflicts(
        &ctx,
        &task_file,
        &index,
        config.conflict_detection,
        config.conflict_policy,
    )?;

    // ========================================================================
    // Phase 4: Re-claim Check (after reject)
    // ========================================================================

    // If the task already has recorded branch/worktree values (from a prior claim/reject),
    // check if they still exist and are valid
    let existing_branch = task_file.frontmatter.branch.as_deref();
    let existing_worktree = task_file.frontmatter.worktree.as_deref();
    let existing_base_sha = task_file.frontmatter.base_sha.as_deref();

    let existing_git_refs = crate::task_git::validate_task_git_refs_if_present(
        &ctx,
        &task_info.id,
        existing_branch,
        existing_worktree,
    )?;
    let existing_worktree_for_setup = existing_git_refs
        .as_ref()
        .map(|r| r.worktree_path.to_string_lossy().to_string());

    // Check if this is a re-claim with existing state
    if let Some(refs) = &existing_git_refs {
        let branch = refs.branch.as_str();
        let worktree_path = &refs.worktree_path;

        // Check if worktree exists
        if worktree_path.exists() {
            // Verify it's on the correct branch
            if let Ok(current_branch) = crate::git_worktree::get_current_branch(worktree_path)
                && current_branch != branch
            {
                return Err(BurlError::UserError(format!(
                    "task has recorded worktree at '{}' but it's on branch '{}', not '{}'.\n\n\
                     Run `burl doctor` to diagnose and repair this inconsistency.",
                    worktree_path.display(),
                    current_branch,
                    branch
                )));
            }
        } else if branch_exists(&ctx.repo_root, branch)? {
            // Branch exists but worktree is missing
            return Err(BurlError::UserError(format!(
                "task has recorded branch '{}' but worktree at '{}' is missing.\n\n\
                 Run `burl doctor` to diagnose and repair this inconsistency,\n\
                 or manually recreate the worktree:\n  git worktree add {} {}",
                branch,
                worktree_path.display(),
                worktree_path.display(),
                branch
            )));
        }
    }

    // ========================================================================
    // Phase 5: Create/Reuse Branch and Worktree
    // ========================================================================

    let mut transaction = ClaimTransaction::new();

    // Check if branch already existed before setup
    let branch_existed_before = if let Some(branch) = existing_branch {
        branch_exists(&ctx.repo_root, branch)?
    } else {
        false
    };

    // Setup task worktree (handles fetch, base_sha, branch, worktree creation)
    let slug = slugify_title(&task_file.frontmatter.title);
    let worktree_info = match setup_task_worktree(
        &ctx,
        &task_info.id,
        Some(&slug),
        &config.remote,
        &config.main_branch,
        existing_branch,
        existing_worktree_for_setup.as_deref(),
    ) {
        Ok(info) => {
            transaction.branch_name = info.branch.clone();
            transaction.branch_created = !branch_existed_before && !info.reused;
            transaction.worktree_info = Some(info.clone());
            info
        }
        Err(e) => {
            // Rollback not needed if setup_task_worktree handles its own cleanup
            return Err(e);
        }
    };

    // Don't change base_sha on reuse (PRD policy)
    let base_sha = if worktree_info.reused {
        if let Some(sha) = existing_base_sha {
            sha.to_string()
        } else {
            worktree_info.base_sha.clone()
        }
    } else {
        worktree_info.base_sha.clone()
    };

    // ========================================================================
    // Phase 6: Workflow State Mutation (under workflow lock)
    // ========================================================================

    // Verify workflow worktree is clean before acquiring lock
    if let Err(e) = ctx.ensure_workflow_clean() {
        transaction.rollback(&ctx.repo_root);
        return Err(e);
    }

    // Acquire workflow lock
    let _workflow_lock = match acquire_workflow_lock(&ctx, "claim") {
        Ok(lock) => lock,
        Err(e) => {
            transaction.rollback(&ctx.repo_root);
            return Err(e);
        }
    };

    // Update task frontmatter
    let assignee = get_assignee_string();
    let now = Utc::now();

    task_file.set_assigned(&assignee, Some(now));
    task_file.set_git_info(
        &worktree_info.branch,
        &worktree_info.path.to_string_lossy(),
        &base_sha,
    );

    // Atomically write updated task file
    if let Err(e) = task_file.save(&task_info.path) {
        transaction.rollback(&ctx.repo_root);
        return Err(e);
    }

    // Move task file READY -> DOING atomically
    let filename = match task_info.path.file_name() {
        Some(name) => name,
        None => {
            transaction.rollback(&ctx.repo_root);
            return Err(BurlError::UserError("invalid task file path".to_string()));
        }
    };
    let doing_path = ctx.bucket_path("DOING").join(filename);

    // Move task file into DOING.
    if let Err(e) = crate::fs::move_file(&task_info.path, &doing_path) {
        transaction.rollback(&ctx.repo_root);
        return Err(BurlError::UserError(format!(
            "failed to move task from READY to DOING: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_info.path.display(),
            doing_path.display()
        )));
    }

    // ========================================================================
    // Phase 7: Event Logging and Commit
    // ========================================================================

    // Append claim event
    let event = Event::new(EventAction::Claim)
        .with_task(&task_info.id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "branch": worktree_info.branch,
            "worktree": worktree_info.path.to_string_lossy(),
            "base_sha": base_sha,
            "reused": worktree_info.reused,
            "assigned_to": assignee
        }));
    append_event(&ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_claim(&ctx, &task_info.id, &task_file.frontmatter.title)?;

        // Push if auto-push enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // ========================================================================
    // Phase 8: Output
    // ========================================================================

    // Print worktree path for agents to cd into
    println!("{}", worktree_info.path.display());

    // Print additional info to stderr so it doesn't interfere with scripted use
    eprintln!();
    eprintln!("Claimed task: {}", task_info.id);
    eprintln!("  Title:    {}", task_file.frontmatter.title);
    eprintln!("  Branch:   {}", worktree_info.branch);
    eprintln!("  Base SHA: {}", base_sha);
    if worktree_info.reused {
        eprintln!("  (reused existing worktree)");
    }
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. cd {}", worktree_info.path.display());
    eprintln!("  2. Make your changes");
    eprintln!("  3. Run `burl submit {}` when ready for QA", task_info.id);

    Ok(())
}
