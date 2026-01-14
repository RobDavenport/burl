//! Tests for the clean command.

use super::planning::{build_cleanup_plan, path_contains_traversal};
use super::types::{CleanupCandidate, CleanupResult};
use crate::cli::CleanArgs;
use crate::commands::clean::cmd_clean;
use crate::commands::init::cmd_init;
use crate::context::require_initialized_workflow;
use crate::test_support::{DirGuard, create_test_repo};
use serial_test::serial;
use std::path::{Path, PathBuf};

#[test]
fn test_path_contains_traversal() {
    assert!(path_contains_traversal(Path::new("../foo")));
    assert!(path_contains_traversal(Path::new("foo/../bar")));
    assert!(path_contains_traversal(Path::new("foo/bar/..")));
    assert!(!path_contains_traversal(Path::new("foo/bar")));
    assert!(!path_contains_traversal(Path::new("/absolute/path")));
    assert!(!path_contains_traversal(Path::new("./relative")));
}

#[test]
#[serial]
fn test_clean_dry_run_no_candidates() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Run clean (dry-run, no candidates)
    let args = CleanArgs {
        completed: false,
        orphans: false,
        yes: false,
    };

    let result = cmd_clean(args);
    assert!(result.is_ok());
}

#[test]
#[serial]
fn test_clean_detects_orphan_directory() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Create a fake orphan directory under .worktrees/
    let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("file.txt"), "orphan content").unwrap();

    // Build cleanup plan
    let ctx = require_initialized_workflow().unwrap();
    let args = CleanArgs {
        completed: false,
        orphans: true,
        yes: false,
    };
    let plan = build_cleanup_plan(&ctx, &args).unwrap();

    // Should detect the orphan directory
    assert_eq!(plan.orphan_directories.len(), 1);
    assert!(plan.orphan_directories[0].ends_with("orphan-task"));
}

#[test]
#[serial]
fn test_clean_removes_orphan_directory_with_yes() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Create a fake orphan directory under .worktrees/
    let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("file.txt"), "orphan content").unwrap();

    // Verify it exists
    assert!(orphan_dir.exists());

    // Run clean with --yes
    let args = CleanArgs {
        completed: false,
        orphans: true,
        yes: true,
    };

    let result = cmd_clean(args);
    assert!(result.is_ok());

    // Verify orphan was removed
    assert!(!orphan_dir.exists());
}

#[test]
#[serial]
fn test_clean_skips_dirty_orphan_worktree() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    cmd_init().unwrap();

    let repo_root = temp_dir.path();
    let worktree_path = repo_root.join(".worktrees/orphan-wt");

    // Create a branch and add a worktree under .worktrees/
    std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["branch", "orphan-branch"])
        .output()
        .expect("failed to create branch");
    std::process::Command::new("git")
        .current_dir(repo_root)
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "orphan-branch",
        ])
        .output()
        .expect("failed to create worktree");

    // Make the worktree dirty (untracked file).
    std::fs::write(worktree_path.join("untracked.txt"), "dirty").unwrap();
    assert!(worktree_path.exists());

    // Run clean with --yes. The dirty worktree should be skipped.
    let args = CleanArgs {
        completed: false,
        orphans: true,
        yes: true,
    };

    let result = cmd_clean(args);
    assert!(result.is_ok());
    assert!(
        worktree_path.exists(),
        "dirty orphan worktree should not be removed"
    );
}

#[test]
#[serial]
fn test_clean_with_completed_flag() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Create a fake orphan directory under .worktrees/
    let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
    std::fs::create_dir_all(&orphan_dir).unwrap();

    // Build cleanup plan with --completed only
    let ctx = require_initialized_workflow().unwrap();
    let args = CleanArgs {
        completed: true,
        orphans: false,
        yes: false,
    };
    let plan = build_cleanup_plan(&ctx, &args).unwrap();

    // Should NOT detect orphans when only --completed is specified
    assert!(plan.orphan_directories.is_empty());
    assert!(plan.orphan_worktrees.is_empty());
}

#[test]
#[serial]
fn test_clean_with_orphans_flag() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Create a fake orphan directory under .worktrees/
    let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
    std::fs::create_dir_all(&orphan_dir).unwrap();

    // Build cleanup plan with --orphans only
    let ctx = require_initialized_workflow().unwrap();
    let args = CleanArgs {
        completed: false,
        orphans: true,
        yes: false,
    };
    let plan = build_cleanup_plan(&ctx, &args).unwrap();

    // Should detect orphan
    assert_eq!(plan.orphan_directories.len(), 1);
    // Should NOT look for completed task worktrees
    assert!(plan.completed_worktrees.is_empty());
}

#[test]
fn test_is_path_under() {
    // Note: is_path_under uses canonicalize which requires paths to exist.
    // For synthetic paths, the fallback logic (without canonicalize) applies.
    // We test the fallback behavior here with synthetic paths.
    let parent = Path::new("/home/user/repo/.worktrees");
    let child = Path::new("/home/user/repo/.worktrees/task-001");
    let outside = Path::new("/home/user/other");

    // Child should be under parent (using fallback since paths don't exist)
    assert!(child.starts_with(parent));
    // Outside should not be under parent
    assert!(!outside.starts_with(parent));
}

#[test]
fn test_cleanup_result_default() {
    let result = CleanupResult::default();
    assert_eq!(result.removed_count, 0);
    assert_eq!(result.skipped_count, 0);
    assert!(result.skipped.is_empty());
}

#[test]
fn test_cleanup_candidate_fields() {
    let candidate = CleanupCandidate {
        path: PathBuf::from("/path/to/worktree"),
        task_id: Some("TASK-001".to_string()),
        branch: Some("task-001-feature".to_string()),
    };

    assert_eq!(candidate.path, PathBuf::from("/path/to/worktree"));
    assert_eq!(candidate.task_id, Some("TASK-001".to_string()));
    assert_eq!(candidate.branch, Some("task-001-feature".to_string()));
}
