//! Tests for the approve command.

use super::*;
use crate::cli::{AddArgs, ClaimArgs, SubmitArgs};
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
    let worktree_path = temp_dir.path().join(".worktrees/task-001-test-approve");

    // Add a task with glob scope
    cmd_add(AddArgs {
        title: "Test approve".to_string(),
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
fn test_approve_task_not_in_qa_fails() {
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

    // Try to approve task in READY - should fail
    let result = cmd_approve(ApproveArgs {
        task_id: "TASK-001".to_string(),
    });

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
    assert!(err.to_string().contains("not in QA"));
}

#[test]
#[serial]
fn test_approve_nonexistent_task_fails() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Try to approve a task that doesn't exist
    let result = cmd_approve(ApproveArgs {
        task_id: "TASK-999".to_string(),
    });

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
    assert!(err.to_string().contains("not found"));
}

#[test]
#[serial]
fn test_approve_happy_path() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Write config with empty build_command to skip build validation
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
    std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

    // Setup task in QA
    setup_task_in_qa(&temp_dir);

    // Approve the task
    let result = cmd_approve(ApproveArgs {
        task_id: "TASK-001".to_string(),
    });

    assert!(result.is_ok(), "Approve should succeed: {:?}", result);

    // Verify task moved to DONE
    let done_path = temp_dir
        .path()
        .join(".burl/.workflow/DONE/TASK-001-test-approve.md");
    assert!(done_path.exists(), "Task should be in DONE bucket");

    // Verify QA is empty
    let qa_path = temp_dir
        .path()
        .join(".burl/.workflow/QA/TASK-001-test-approve.md");
    assert!(!qa_path.exists(), "Task should no longer be in QA");

    // Verify completed_at was set
    let task = TaskFile::load(&done_path).unwrap();
    assert!(task.frontmatter.completed_at.is_some());

    // Verify worktree was cleaned up
    let worktree_path = temp_dir.path().join(".worktrees/task-001-test-approve");
    assert!(!worktree_path.exists(), "Worktree should be removed");

    // Verify main branch contains the changes
    let main_log = ProcessCommand::new("git")
        .current_dir(temp_dir.path())
        .args(["log", "--oneline", "main"])
        .output()
        .expect("failed to get git log");
    let log_output = String::from_utf8_lossy(&main_log.stdout);
    assert!(
        log_output.contains("Add valid implementation"),
        "Main should contain the task commit"
    );
}

#[test]
#[serial]
fn test_approve_skips_cleanup_when_worktree_dirty() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    cmd_init().unwrap();

    // Skip build validation to keep the test focused.
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
    std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

    let worktree_path = setup_task_in_qa(&temp_dir);

    // Make the worktree "dirty" in a way that shouldn't block rebase/merge.
    std::fs::write(worktree_path.join("untracked.txt"), "keep me").unwrap();

    let result = cmd_approve(ApproveArgs {
        task_id: "TASK-001".to_string(),
    });
    assert!(result.is_ok(), "Approve should succeed: {:?}", result);

    let done_path = temp_dir
        .path()
        .join(".burl/.workflow/DONE/TASK-001-test-approve.md");
    assert!(done_path.exists(), "Task should be in DONE bucket");

    assert!(
        worktree_path.exists(),
        "Dirty worktree should be preserved to avoid data loss"
    );
}

#[test]
#[serial]
fn test_approve_with_rebase_conflict_rejects() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Write config with empty build_command
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
    std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

    // Setup task in QA
    let worktree_path = setup_task_in_qa(&temp_dir);

    // Create a conflicting commit on main AFTER claiming the task
    // First, checkout main in the repo root
    ProcessCommand::new("git")
        .current_dir(temp_dir.path())
        .args(["checkout", "main"])
        .output()
        .expect("failed to checkout main");

    // Create the same file with different content
    std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
    std::fs::write(
        temp_dir.path().join("src/lib.rs"),
        "fn main() {\n    println!(\"Conflict!\");\n}\n",
    )
    .unwrap();

    ProcessCommand::new("git")
        .current_dir(temp_dir.path())
        .args(["add", "."])
        .output()
        .expect("failed to add files");
    ProcessCommand::new("git")
        .current_dir(temp_dir.path())
        .args(["commit", "-m", "Conflicting commit on main"])
        .output()
        .expect("failed to commit");

    // Try to approve - should fail with rebase conflict
    let result = cmd_approve(ApproveArgs {
        task_id: "TASK-001".to_string(),
    });

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should be a GitError for rebase conflict
    assert_eq!(err.exit_code(), exit_codes::GIT_FAILURE);
    assert!(
        err.to_string().contains("rebase conflict")
            || err.to_string().contains("approval rejected")
    );

    // Verify task moved to READY (rejected)
    let ready_path = temp_dir
        .path()
        .join(".burl/.workflow/READY/TASK-001-test-approve.md");
    assert!(
        ready_path.exists(),
        "Task should be in READY bucket after rejection"
    );

    // Verify QA is empty
    let qa_path = temp_dir
        .path()
        .join(".burl/.workflow/QA/TASK-001-test-approve.md");
    assert!(!qa_path.exists(), "Task should no longer be in QA");

    // Verify worktree was preserved (for rework)
    assert!(
        worktree_path.exists(),
        "Worktree should be preserved for rework"
    );

    // Verify qa_attempts was incremented
    let task = TaskFile::load(&ready_path).unwrap();
    assert_eq!(task.frontmatter.qa_attempts, 1);
}
