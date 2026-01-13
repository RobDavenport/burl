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

use crate::cli::RejectArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, validate_task_id};
use chrono::Utc;
use serde_json::json;

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

/// Commit the rejection to the workflow branch.
fn commit_reject(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    reason: &str,
    destination: &str,
) -> Result<()> {
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

    let commit_msg = format!(
        "Reject task {} -> {}: {}",
        task_id, destination, short_reason
    );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::AddArgs;
    use crate::cli::ClaimArgs;
    use crate::cli::SubmitArgs;
    use crate::commands::add::cmd_add;
    use crate::commands::claim::cmd_claim;
    use crate::commands::init::cmd_init;
    use crate::commands::submit::cmd_submit;
    use crate::exit_codes;
    use crate::test_support::{DirGuard, create_test_repo_with_remote};
    use serial_test::serial;
    use std::path::PathBuf;
    use std::process::Command as ProcessCommand;
    use tempfile::TempDir;

    /// Helper to create a task in QA state with valid changes.
    fn setup_task_in_qa(temp_dir: &TempDir) -> PathBuf {
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-reject");

        // Add a task with glob scope
        cmd_add(AddArgs {
            title: "Test reject".to_string(),
            priority: "medium".to_string(),
            affects: vec![],
            affects_globs: vec!["src/**".to_string()],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Claim the task
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Make a valid change
        std::fs::create_dir_all(worktree_path.join("src")).unwrap();
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() {\n    println!(\"Hello, world!\");\n}\n",
        )
        .unwrap();

        // Commit the change
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Add valid implementation"])
            .output()
            .expect("failed to commit");

        // Submit to QA
        cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        worktree_path
    }

    #[test]
    #[serial]
    fn test_reject_task_not_in_qa_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task (stays in READY)
        cmd_add(AddArgs {
            title: "Test task".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Try to reject task in READY - should fail
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Test reason".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not in QA"));
    }

    #[test]
    #[serial]
    fn test_reject_nonexistent_task_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Try to reject a task that doesn't exist
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-999".to_string(),
            reason: "Test reason".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_reject_empty_reason_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Try to reject with empty reason - should fail
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    #[serial]
    fn test_reject_whitespace_only_reason_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Try to reject with whitespace-only reason - should fail
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "   ".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    #[serial]
    fn test_reject_happy_path_moves_to_ready() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\nqa_max_attempts: 3\n").unwrap();

        // Setup task in QA
        let worktree_path = setup_task_in_qa(&temp_dir);

        // Reject the task
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Tests are failing".to_string(),
        });

        assert!(result.is_ok(), "Reject should succeed: {:?}", result);

        // Verify task moved to READY
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        assert!(ready_path.exists(), "Task should be in READY bucket");

        // Verify QA is empty
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-reject.md");
        assert!(!qa_path.exists(), "Task should no longer be in QA");

        // Verify worktree was preserved
        assert!(
            worktree_path.exists(),
            "Worktree should be preserved for rework"
        );

        // Verify qa_attempts was incremented
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 1);

        // Verify submitted_at was cleared
        assert!(task.frontmatter.submitted_at.is_none());

        // Verify reason was appended to QA Report
        assert!(task.body.contains("Tests are failing"));
        assert!(task.body.contains("Rejection:"));
    }

    #[test]
    #[serial]
    fn test_reject_increments_qa_attempts() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command and high max attempts
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\nqa_max_attempts: 5\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // First rejection
        cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "First rejection".to_string(),
        })
        .unwrap();

        // Verify qa_attempts is 1
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 1);

        // Re-claim and re-submit for second rejection
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-reject");
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() {\n    println!(\"Second attempt\");\n}\n",
        )
        .unwrap();
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .unwrap();
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Second attempt"])
            .output()
            .unwrap();

        cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Second rejection
        cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Second rejection".to_string(),
        })
        .unwrap();

        // Verify qa_attempts is 2
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 2);
    }

    #[test]
    #[serial]
    fn test_reject_max_attempts_moves_to_blocked() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with low max attempts (1)
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\nqa_max_attempts: 1\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Reject the task - should move to BLOCKED since qa_max_attempts = 1
        let result = cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Final rejection".to_string(),
        });

        assert!(result.is_ok(), "Reject should succeed: {:?}", result);

        // Verify task moved to BLOCKED
        let blocked_path = temp_dir
            .path()
            .join(".burl/.workflow/BLOCKED/TASK-001-test-reject.md");
        assert!(blocked_path.exists(), "Task should be in BLOCKED bucket");

        // Verify READY doesn't have the task
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        assert!(!ready_path.exists(), "Task should not be in READY");

        // Verify QA is empty
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-reject.md");
        assert!(!qa_path.exists(), "Task should not be in QA");

        // Verify task metadata
        let task = TaskFile::load(&blocked_path).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 1);
    }

    #[test]
    #[serial]
    fn test_reject_priority_boost_on_retry() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with auto_priority_boost_on_retry enabled (default)
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(
            &config_path,
            "build_command: \"\"\nqa_max_attempts: 3\nauto_priority_boost_on_retry: true\n",
        )
        .unwrap();

        // Setup task in QA (with medium priority)
        setup_task_in_qa(&temp_dir);

        // Verify task has medium priority before rejection
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-reject.md");
        let task_before = TaskFile::load(&qa_path).unwrap();
        assert_eq!(task_before.frontmatter.priority, "medium");

        // Reject the task
        cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Needs rework".to_string(),
        })
        .unwrap();

        // Verify priority was boosted to high
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.priority, "high");
    }

    #[test]
    #[serial]
    fn test_reject_preserves_branch_and_worktree() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        let worktree_path = setup_task_in_qa(&temp_dir);

        // Get original branch and worktree from task file
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-reject.md");
        let task_before = TaskFile::load(&qa_path).unwrap();
        let original_branch = task_before.frontmatter.branch.clone();
        let original_worktree = task_before.frontmatter.worktree.clone();

        // Reject the task
        cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Needs changes".to_string(),
        })
        .unwrap();

        // Verify worktree still exists
        assert!(worktree_path.exists(), "Worktree should be preserved");

        // Verify branch and worktree fields are preserved in task file
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.branch, original_branch);
        assert_eq!(task.frontmatter.worktree, original_worktree);
    }

    #[test]
    #[serial]
    fn test_reject_appends_qa_report_with_details() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Reject the task with a specific reason
        let rejection_reason = "Unit tests failing: TestXYZ assertion error on line 42";
        cmd_reject(RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: rejection_reason.to_string(),
        })
        .unwrap();

        // Verify QA Report contains the details
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-reject.md");
        let task = TaskFile::load(&ready_path).unwrap();

        assert!(
            task.body.contains("QA Report"),
            "Should have QA Report section"
        );
        assert!(
            task.body.contains("Rejection:"),
            "Should have Rejection header"
        );
        assert!(
            task.body.contains(rejection_reason),
            "Should contain rejection reason"
        );
        assert!(task.body.contains("Actor:"), "Should have Actor field");
        assert!(task.body.contains("Attempt:"), "Should have Attempt field");
    }

    #[test]
    fn test_get_actor_string() {
        let actor = get_actor_string();
        assert!(actor.contains('@'));
        assert!(!actor.is_empty());
    }
}
