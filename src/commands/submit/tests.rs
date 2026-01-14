//! Tests for the submit command.

use super::*;
use crate::cli::{AddArgs, ClaimArgs};
use crate::commands::add::cmd_add;
use crate::commands::claim::cmd_claim;
use crate::commands::init::cmd_init;
use crate::exit_codes;
use crate::task::TaskFile;
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
