//! Integration tests for claim command.

use crate::cli::{AddArgs, ClaimArgs};
use crate::commands::add::cmd_add;
use crate::commands::claim::cmd_claim;
use crate::commands::init::cmd_init;
use crate::task::TaskFile;
use crate::test_support::{DirGuard, create_test_repo_with_remote};
use serial_test::serial;

#[test]
#[serial]
fn test_claim_explicit_task() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add a task
    let add_args = AddArgs {
        title: "Test claim task".to_string(),
        priority: "high".to_string(),
        affects: vec!["src/lib.rs".to_string()],
        affects_globs: vec![],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    };
    cmd_add(add_args).unwrap();

    // Claim the task
    let claim_args = ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    };
    cmd_claim(claim_args).unwrap();

    // Verify task moved to DOING
    let doing_path = temp_dir
        .path()
        .join(".burl/.workflow/DOING/TASK-001-test-claim-task.md");
    assert!(doing_path.exists(), "Task should be in DOING bucket");

    // Verify task file has claim metadata
    let task = TaskFile::load(&doing_path).unwrap();
    assert!(task.frontmatter.assigned_to.is_some());
    assert!(task.frontmatter.started_at.is_some());
    assert!(task.frontmatter.branch.is_some());
    assert!(task.frontmatter.worktree.is_some());
    assert!(task.frontmatter.base_sha.is_some());

    // Verify worktree was created
    let worktree_path = temp_dir.path().join(".worktrees/task-001-test-claim-task");
    assert!(
        worktree_path.exists(),
        "Worktree should exist at {:?}",
        worktree_path
    );
}

#[test]
#[serial]
fn test_claim_next_task_deterministic() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add tasks with different priorities
    cmd_add(AddArgs {
        title: "Low priority task".to_string(),
        priority: "low".to_string(),
        affects: vec!["tests/a.rs".to_string()],
        affects_globs: vec![],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    cmd_add(AddArgs {
        title: "High priority task".to_string(),
        priority: "high".to_string(),
        affects: vec!["tests/b.rs".to_string()],
        affects_globs: vec![],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    cmd_add(AddArgs {
        title: "Medium priority task".to_string(),
        priority: "medium".to_string(),
        affects: vec!["tests/c.rs".to_string()],
        affects_globs: vec![],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    // Claim without task ID - should pick high priority first
    let claim_args = ClaimArgs { task_id: None };
    cmd_claim(claim_args).unwrap();

    // TASK-002 (high priority) should be in DOING
    let doing_path = temp_dir
        .path()
        .join(".burl/.workflow/DOING/TASK-002-high-priority-task.md");
    assert!(
        doing_path.exists(),
        "High priority task should be claimed first"
    );
}

#[test]
#[serial]
fn test_claim_fails_for_non_ready_task() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add and claim a task
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

    // First claim should succeed
    cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    })
    .unwrap();

    // Second claim should fail (task is now in DOING)
    let result = cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    });

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in READY"));
}

#[test]
#[serial]
fn test_claim_fails_with_unmet_dependencies() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add task with dependency
    cmd_add(AddArgs {
        title: "Dependent task".to_string(),
        priority: "high".to_string(),
        affects: vec![],
        affects_globs: vec![],
        must_not_touch: vec![],
        depends_on: vec!["TASK-002".to_string()], // Depends on non-existent task
        tags: vec![],
    })
    .unwrap();

    // Claim should fail due to unmet dependency
    let result = cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    });

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("dependencies not satisfied")
    );
}

#[test]
#[serial]
fn test_claim_with_scope_conflict_fails_by_default() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add two tasks with overlapping scope
    cmd_add(AddArgs {
        title: "First task".to_string(),
        priority: "high".to_string(),
        affects: vec!["src/lib.rs".to_string()],
        affects_globs: vec!["src/**".to_string()],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    cmd_add(AddArgs {
        title: "Second task".to_string(),
        priority: "high".to_string(),
        affects: vec!["src/main.rs".to_string()],
        affects_globs: vec!["src/**".to_string()],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    // Claim first task
    cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    })
    .unwrap();

    // Claim second task should fail due to scope conflict
    let result = cmd_claim(ClaimArgs {
        task_id: Some("TASK-002".to_string()),
    });

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("scope conflict") || err_msg.contains("conflict"),
        "Error should mention scope conflict: {}",
        err_msg
    );
}

#[test]
#[serial]
fn test_claim_scope_conflict_hybrid_uses_diffs_when_available() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Enable hybrid diff-based conflict detection.
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
    std::fs::write(&config_path, "conflict_detection: hybrid\n").unwrap();

    // Task 1: broad scope
    cmd_add(AddArgs {
        title: "First task".to_string(),
        priority: "high".to_string(),
        affects: vec![],
        affects_globs: vec!["src/**".to_string()],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    // Task 2: narrow scope that is a subset of task 1's declared scope
    cmd_add(AddArgs {
        title: "Second task".to_string(),
        priority: "high".to_string(),
        affects: vec![],
        affects_globs: vec!["src/foo/**".to_string()],
        must_not_touch: vec![],
        depends_on: vec![],
        tags: vec![],
    })
    .unwrap();

    // Claim task 1
    cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    })
    .unwrap();

    // Make a committed change outside src/foo/** in task 1's worktree.
    let worktree_path = temp_dir.path().join(".worktrees/task-001-first-task");
    std::fs::create_dir_all(worktree_path.join("src")).unwrap();
    std::fs::write(worktree_path.join("src/bar.rs"), "pub fn bar() {}\n").unwrap();

    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&worktree_path)
            .args(args)
            .output()
            .expect("failed to run git")
    };
    git(&["add", "."]);
    git(&["commit", "-m", "Add bar"]);

    // Claiming task 2 should succeed in hybrid mode because task 1's actual diff
    // does not overlap with src/foo/**.
    let result = cmd_claim(ClaimArgs {
        task_id: Some("TASK-002".to_string()),
    });

    assert!(result.is_ok(), "expected claim to succeed: {:?}", result);
}

#[test]
#[serial]
fn test_claim_base_sha_is_set() {
    let temp_dir = create_test_repo_with_remote();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Add a task
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

    // Claim the task
    cmd_claim(ClaimArgs {
        task_id: Some("TASK-001".to_string()),
    })
    .unwrap();

    // Verify base_sha was set
    let doing_path = temp_dir
        .path()
        .join(".burl/.workflow/DOING/TASK-001-test-task.md");
    let task = TaskFile::load(&doing_path).unwrap();

    assert!(task.frontmatter.base_sha.is_some());
    let base_sha = task.frontmatter.base_sha.unwrap();
    assert_eq!(base_sha.len(), 40, "base_sha should be a full SHA");
}
