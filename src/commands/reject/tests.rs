//! Tests for the reject command.

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
