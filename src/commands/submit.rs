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
use crate::git::run_git;
use crate::git_worktree::get_current_branch;
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::validate::{validate_scope, validate_stubs_with_config};
use crate::workflow::{TaskIndex, validate_task_id};
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;

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

    // Check worktree exists and is valid
    let worktree_path = task_file.frontmatter.worktree.as_ref().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded worktree.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    let worktree_path = if PathBuf::from(worktree_path).is_absolute() {
        PathBuf::from(worktree_path)
    } else {
        ctx.repo_root.join(worktree_path)
    };

    if !worktree_path.exists() {
        return Err(BurlError::UserError(format!(
            "task worktree does not exist at '{}'.\n\n\
             Run `burl doctor` to diagnose and repair this inconsistency.",
            worktree_path.display()
        )));
    }

    // Check branch is recorded and matches current branch in worktree
    let expected_branch = task_file.frontmatter.branch.clone().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded branch.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

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

/// Count commits since base_sha in the worktree.
fn count_commits_since(worktree: &std::path::Path, base_sha: &str) -> Result<u32> {
    let range = format!("{}..HEAD", base_sha);
    let output = run_git(worktree, &["rev-list", "--count", &range])?;

    output.stdout.trim().parse::<u32>().map_err(|e| {
        BurlError::GitError(format!(
            "failed to parse commit count '{}': {}",
            output.stdout.trim(),
            e
        ))
    })
}

/// Push the task branch to the remote.
fn push_task_branch(worktree: &std::path::Path, remote: &str, branch: &str) -> Result<()> {
    run_git(worktree, &["push", remote, branch]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to push task branch '{}' to '{}': {}\n\n\
             Fix any issues and try again, or set `push_task_branch_on_submit: false` in config.",
            branch, remote, e
        ))
    })?;
    Ok(())
}

/// Commit the submit to the workflow branch.
fn commit_submit(ctx: &crate::context::WorkflowContext, task_id: &str, title: &str) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage submit changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("Submit task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit submit: {}", e)))?;

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
    use crate::commands::add::cmd_add;
    use crate::commands::claim::cmd_claim;
    use crate::commands::init::cmd_init;
    use crate::exit_codes;
    use crate::test_support::{DirGuard, create_test_repo_with_remote};
    use serial_test::serial;
    use std::process::Command;

    #[test]
    #[serial]
    fn test_submit_no_commits_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task
        cmd_add(AddArgs {
            title: "Test submit task".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/lib.rs".to_string()],
            affects_globs: vec![],
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

        // Try to submit without making any commits - should fail
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("no commits"));
    }

    #[test]
    #[serial]
    fn test_submit_with_scope_violation_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task with restricted scope
        cmd_add(AddArgs {
            title: "Test scope task".to_string(),
            priority: "high".to_string(),
            affects: vec!["allowed/file.rs".to_string()],
            affects_globs: vec![],
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

        // Find the worktree path
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-scope-task");

        // Make a change to a file OUTSIDE the allowed scope
        std::fs::create_dir_all(worktree_path.join("not_allowed")).unwrap();
        std::fs::write(
            worktree_path.join("not_allowed/bad.rs"),
            "// This is outside scope\n",
        )
        .unwrap();

        // Commit the out-of-scope change
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Out of scope change"])
            .output()
            .expect("failed to commit");

        // Try to submit - should fail with validation error
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::VALIDATION_FAILURE);
    }

    #[test]
    #[serial]
    fn test_submit_with_stub_violation_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task
        cmd_add(AddArgs {
            title: "Test stub task".to_string(),
            priority: "high".to_string(),
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

        // Find the worktree path
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-stub-task");

        // Make a change with a stub pattern
        std::fs::create_dir_all(worktree_path.join("src")).unwrap();
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() {\n    // TODO: implement this\n}\n",
        )
        .unwrap();

        // Commit the stub
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Add stub"])
            .output()
            .expect("failed to commit");

        // Try to submit - should fail with validation error
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::VALIDATION_FAILURE);
    }

    #[test]
    #[serial]
    fn test_submit_valid_task_moves_to_qa() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task with glob scope to allow new files
        cmd_add(AddArgs {
            title: "Test valid submit".to_string(),
            priority: "high".to_string(),
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

        // Find the worktree path
        let worktree_path = temp_dir
            .path()
            .join(".worktrees/task-001-test-valid-submit");

        // Make a valid change (no stubs)
        std::fs::create_dir_all(worktree_path.join("src")).unwrap();
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() {\n    println!(\"Hello, world!\");\n}\n",
        )
        .unwrap();

        // Commit the change
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Add valid implementation"])
            .output()
            .expect("failed to commit");

        // Submit the task
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_ok(), "Submit should succeed: {:?}", result);

        // Verify task moved to QA
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-valid-submit.md");
        assert!(qa_path.exists(), "Task should be in QA bucket");

        // Verify DOING is empty
        let doing_path = temp_dir
            .path()
            .join(".burl/.workflow/DOING/TASK-001-test-valid-submit.md");
        assert!(!doing_path.exists(), "Task should no longer be in DOING");

        // Verify submitted_at was set
        let task = TaskFile::load(&qa_path).unwrap();
        assert!(task.frontmatter.submitted_at.is_some());
    }

    #[test]
    #[serial]
    fn test_submit_task_not_in_doing_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task (but don't claim it - stays in READY)
        cmd_add(AddArgs {
            title: "Test ready task".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Try to submit without claiming - should fail
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not in DOING"));
    }

    #[test]
    #[serial]
    fn test_submit_nonexistent_task_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Try to submit a task that doesn't exist
        let result = cmd_submit(SubmitArgs {
            task_id: Some("TASK-999".to_string()),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_submit_without_task_id_single_doing() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add and claim a task
        cmd_add(AddArgs {
            title: "Test auto select".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec!["src/**".to_string()],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Find the worktree path
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-auto-select");

        // Make a valid change
        std::fs::create_dir_all(worktree_path.join("src")).unwrap();
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() { println!(\"test\"); }\n",
        )
        .unwrap();

        Command::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Add implementation"])
            .output()
            .expect("failed to commit");

        // Submit without specifying task ID - should auto-select the only DOING task
        let result = cmd_submit(SubmitArgs { task_id: None });

        assert!(result.is_ok(), "Submit should succeed: {:?}", result);

        // Verify task moved to QA
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-auto-select.md");
        assert!(qa_path.exists(), "Task should be in QA bucket");
    }
}
