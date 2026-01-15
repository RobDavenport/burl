//! Tests for the init command.

use super::*;
use crate::context::WorkflowContext;
use crate::git::run_git;
use crate::test_support::{DirGuard, create_test_repo};
use serial_test::serial;

use super::git_ops::*;
use super::scaffolding::*;
use super::validation::*;
use super::worktree::*;

#[test]
fn test_check_branch_exists() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // main/master branch should exist after initial commit
    let output = run_git(&ctx.repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    let current_branch = output.stdout.trim();
    assert!(check_branch_exists(&ctx, current_branch).unwrap());

    // Non-existent branch should not exist
    assert!(!check_branch_exists(&ctx, "nonexistent-branch").unwrap());
}

#[test]
fn test_get_current_branch() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    let branch = get_current_branch(&ctx).unwrap();
    // Should be either "main" or "master" depending on git config
    assert!(!branch.is_empty());
}

#[test]
fn test_ensure_workflow_worktree_creates_new() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Workflow worktree should not exist initially
    assert!(!ctx.workflow_worktree.exists());

    // Create the worktree
    let created = ensure_workflow_worktree(&ctx).unwrap();

    // Should report that it was created
    assert!(created);

    // Worktree should now exist
    assert!(ctx.workflow_worktree.exists());

    // Should have .git file (linked worktree)
    assert!(ctx.workflow_worktree.join(".git").exists());
}

#[test]
fn test_ensure_workflow_worktree_idempotent() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // First call creates the worktree
    let first_result = ensure_workflow_worktree(&ctx).unwrap();
    assert!(first_result);

    // Second call should succeed but not create a new worktree
    let second_result = ensure_workflow_worktree(&ctx).unwrap();
    assert!(!second_result);
}

#[test]
fn test_create_workflow_structure() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create the worktree first
    ensure_workflow_worktree(&ctx).unwrap();

    // Create the structure
    create_workflow_structure(&ctx).unwrap();

    // Verify all directories exist
    for bucket in BUCKETS {
        let bucket_path = ctx.bucket_path(bucket);
        assert!(bucket_path.exists(), "Bucket {} should exist", bucket);
        assert!(
            bucket_path.join(".gitkeep").exists(),
            "Bucket {} should have .gitkeep",
            bucket
        );
    }

    // Verify events directory
    assert!(ctx.events_dir().exists());
    assert!(ctx.events_dir().join(".gitkeep").exists());

    // Verify prompts directory
    assert!(ctx.prompts_dir().exists());
    assert!(ctx.prompts_dir().join(".gitkeep").exists());

    // Verify locks directory
    assert!(ctx.locks_dir.exists());
    // locks/ should NOT have .gitkeep (it's untracked)
    assert!(!ctx.locks_dir.join(".gitkeep").exists());

    // Verify agent logs directory
    assert!(ctx.agent_logs_dir().exists());
    // agent-logs/ should NOT have .gitkeep (it's untracked)
    assert!(!ctx.agent_logs_dir().join(".gitkeep").exists());

    // Verify config.yaml
    assert!(ctx.config_path().exists());

    // Verify agents.yaml
    assert!(ctx.agents_config_path().exists());

    // Verify .gitignore
    let gitignore_path = ctx.workflow_state_dir.join(".gitignore");
    assert!(gitignore_path.exists());
    let gitignore_content = std::fs::read_to_string(&gitignore_path).unwrap();
    assert!(gitignore_content.contains("locks/"));
    assert!(gitignore_content.contains("agent-logs/"));
}

#[test]
fn test_create_workflow_structure_idempotent() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create the worktree first
    ensure_workflow_worktree(&ctx).unwrap();

    // Create structure twice - should not error
    create_workflow_structure(&ctx).unwrap();
    create_workflow_structure(&ctx).unwrap();

    // All directories should still exist
    for bucket in BUCKETS {
        assert!(ctx.bucket_path(bucket).exists());
    }
}

#[test]
fn test_create_worktrees_dir() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Should not exist initially
    assert!(!ctx.worktrees_dir.exists());

    // Create it
    create_worktrees_dir(&ctx).unwrap();

    // Should exist now
    assert!(ctx.worktrees_dir.exists());

    // Idempotent - second call should not error
    create_worktrees_dir(&ctx).unwrap();
    assert!(ctx.worktrees_dir.exists());
}

#[test]
fn test_add_to_git_exclude() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Add entries
    add_to_git_exclude(&ctx).unwrap();

    // Verify entries were added
    let exclude_path = ctx.repo_root.join(".git").join("info").join("exclude");
    let content = std::fs::read_to_string(&exclude_path).unwrap();
    assert!(content.contains(".burl/"));
    assert!(content.contains(".worktrees/"));

    // Idempotent - second call should not duplicate entries
    add_to_git_exclude(&ctx).unwrap();
    let content2 = std::fs::read_to_string(&exclude_path).unwrap();
    assert_eq!(
        content.matches(".burl/").count(),
        content2.matches(".burl/").count()
    );
}

#[test]
fn test_validate_existing_workflow_dir_nonexistent() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Should succeed when .burl doesn't exist
    validate_existing_workflow_dir(&ctx).unwrap();
}

#[test]
fn test_validate_existing_workflow_dir_valid_worktree() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create a valid worktree
    ensure_workflow_worktree(&ctx).unwrap();

    // Should succeed for valid worktree
    validate_existing_workflow_dir(&ctx).unwrap();
}

#[test]
fn test_validate_existing_workflow_dir_invalid_directory() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create a plain directory (not a worktree)
    std::fs::create_dir_all(&ctx.workflow_worktree).unwrap();

    // Should fail with helpful error
    let result = validate_existing_workflow_dir(&ctx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not a git worktree"));
}

#[test]
fn test_commit_workflow_structure() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create the worktree and structure
    ensure_workflow_worktree(&ctx).unwrap();
    create_workflow_structure(&ctx).unwrap();

    // Commit the structure
    commit_workflow_structure(&ctx, true).unwrap();

    // Verify commit was made
    let log = run_git(&ctx.workflow_worktree, &["log", "--oneline", "-1"]).unwrap();
    assert!(log.stdout.contains("Initialize burl workflow structure"));
}

#[test]
fn test_commit_workflow_structure_idempotent() {
    let temp_dir = create_test_repo();
    let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

    // Create and commit
    ensure_workflow_worktree(&ctx).unwrap();
    create_workflow_structure(&ctx).unwrap();
    commit_workflow_structure(&ctx, true).unwrap();

    // Get commit count
    let log1 = run_git(&ctx.workflow_worktree, &["rev-list", "--count", "HEAD"]).unwrap();

    // Second commit should be no-op (nothing to commit)
    commit_workflow_structure(&ctx, false).unwrap();

    // Commit count should be the same
    let log2 = run_git(&ctx.workflow_worktree, &["rev-list", "--count", "HEAD"]).unwrap();
    assert_eq!(log1.stdout, log2.stdout);
}

#[test]
#[serial]
fn test_full_init_flow() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Resolve context (this simulates what cmd_init does internally)
    let ctx = resolve_context().unwrap();

    // Run the init steps manually (we can't call cmd_init directly due to lock acquisition)
    ensure_workflow_worktree(&ctx).unwrap();
    create_workflow_structure(&ctx).unwrap();
    create_worktrees_dir(&ctx).unwrap();
    add_to_git_exclude(&ctx).unwrap();
    commit_workflow_structure(&ctx, true).unwrap();

    // Verify the workflow is fully set up
    assert!(ctx.workflow_worktree.exists());
    assert!(ctx.workflow_state_dir.exists());
    assert!(ctx.worktrees_dir.exists());
    for bucket in BUCKETS {
        assert!(ctx.bucket_path(bucket).exists());
    }
    assert!(ctx.config_path().exists());
    assert!(ctx.events_dir().exists());
    assert!(ctx.locks_dir.exists());
}
