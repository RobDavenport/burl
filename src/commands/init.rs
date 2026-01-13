//! Implementation of the `burl init` command.
//!
//! This module bootstraps (or reattaches) the canonical workflow worktree
//! and creates the on-branch workflow state directory structure.
//!
//! # What `burl init` does
//!
//! 1. Creates or attaches the workflow worktree (default: `.burl/` on branch `burl`)
//! 2. Creates workflow state directories: READY/, DOING/, QA/, DONE/, BLOCKED/, events/
//! 3. Creates `config.yaml` template (if missing)
//! 4. Creates `.gitignore` with `locks/` entry
//! 5. Ensures `locks/` directory exists locally (untracked)
//! 6. Creates `.worktrees/` directory at repo root (local, untracked)
//! 7. Optionally adds `.burl/` and `.worktrees/` to `.git/info/exclude`
//! 8. Commits the scaffolding to the workflow branch (if `workflow_auto_commit` is true)

use crate::config::Config;
use crate::context::{resolve_context, WorkflowContext, DEFAULT_WORKFLOW_BRANCH};
use crate::error::{BurlError, Result};
use crate::fs::atomic_write_file;
use crate::git::run_git;
use crate::locks;
use std::fs;
use std::path::Path;

/// Status buckets that will be created under `.workflow/`.
const BUCKETS: &[&str] = &["READY", "DOING", "QA", "DONE", "BLOCKED"];

/// Execute the `burl init` command.
///
/// This command is **idempotent**: running it multiple times will not error
/// and will not cause destructive changes to existing workflow state.
pub fn cmd_init() -> Result<()> {
    let ctx = resolve_context()?;

    // Check if .burl exists but is not a valid git worktree
    validate_existing_workflow_dir(&ctx)?;

    // Create or attach the workflow worktree
    let worktree_created = ensure_workflow_worktree(&ctx)?;

    // Acquire workflow lock for the scaffolding phase
    // This prevents concurrent init operations
    let _lock_guard = locks::acquire_workflow_lock(&ctx, "init")?;

    // Create the workflow state directory structure
    create_workflow_structure(&ctx)?;

    // Create the .worktrees directory at repo root (untracked)
    create_worktrees_dir(&ctx)?;

    // Add .burl/ and .worktrees/ to .git/info/exclude
    add_to_git_exclude(&ctx)?;

    // Load config to check if auto-commit is enabled
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Commit the workflow structure if auto-commit is enabled
    if config.workflow_auto_commit {
        commit_workflow_structure(&ctx, worktree_created)?;

        // Push if auto-push is enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // Print success message
    println!("Initialized burl workflow.");
    println!();
    println!("Workflow worktree: {}", ctx.workflow_worktree.display());
    println!("Workflow branch:   {}", DEFAULT_WORKFLOW_BRANCH);
    println!();
    println!("Created directories:");
    for bucket in BUCKETS {
        println!("  .burl/.workflow/{}/", bucket);
    }
    println!("  .burl/.workflow/events/");
    println!("  .burl/.workflow/locks/  (untracked)");
    println!("  .worktrees/             (untracked)");
    println!();
    println!("You can now add tasks with `burl add \"task title\"`.");

    Ok(())
}

/// Validate that if `.burl` exists, it's a valid git worktree.
fn validate_existing_workflow_dir(ctx: &WorkflowContext) -> Result<()> {
    let workflow_path = &ctx.workflow_worktree;

    if !workflow_path.exists() {
        return Ok(());
    }

    // Check if it's a git worktree by looking for the .git file/directory
    let git_path = workflow_path.join(".git");

    if !git_path.exists() {
        return Err(BurlError::UserError(format!(
            "directory '{}' exists but is not a git worktree.\n\n\
             To fix this, either:\n\
             1. Delete or rename '{}' and run `burl init` again\n\
             2. Or manually set up the worktree with:\n\
                git worktree add {} {}",
            workflow_path.display(),
            workflow_path.display(),
            workflow_path.display(),
            DEFAULT_WORKFLOW_BRANCH
        )));
    }

    // If .git exists, check if it's a valid worktree or directory for this repo
    if git_path.is_file() {
        // It's a worktree - check if it points to the same repo
        let git_content = fs::read_to_string(&git_path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read '{}': {}",
                git_path.display(),
                e
            ))
        })?;

        if !git_content.starts_with("gitdir:") {
            return Err(BurlError::UserError(format!(
                "directory '{}' has an invalid .git file.\n\n\
                 To fix this, delete or rename '{}' and run `burl init` again.",
                workflow_path.display(),
                workflow_path.display()
            )));
        }

        // Verify the worktree is checked out to the workflow branch
        verify_worktree_branch(ctx)?;
    } else if git_path.is_dir() {
        // It's a full git repo, not a worktree - this shouldn't happen
        return Err(BurlError::UserError(format!(
            "directory '{}' contains a full git repository, not a worktree.\n\n\
             The workflow directory should be a git worktree, not a separate repository.\n\
             To fix this, delete or rename '{}' and run `burl init` again.",
            workflow_path.display(),
            workflow_path.display()
        )));
    }

    Ok(())
}

/// Verify that the existing workflow worktree is checked out to the workflow branch.
fn verify_worktree_branch(ctx: &WorkflowContext) -> Result<()> {
    let output = run_git(&ctx.workflow_worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let current_branch = output.stdout.trim();

    if current_branch != DEFAULT_WORKFLOW_BRANCH {
        return Err(BurlError::UserError(format!(
            "workflow worktree is checked out to '{}', expected '{}'.\n\n\
             To fix this, either:\n\
             1. Checkout the correct branch: git -C {} checkout {}\n\
             2. Or delete '{}' and run `burl init` again.",
            current_branch,
            DEFAULT_WORKFLOW_BRANCH,
            ctx.workflow_worktree.display(),
            DEFAULT_WORKFLOW_BRANCH,
            ctx.workflow_worktree.display()
        )));
    }

    Ok(())
}

/// Ensure the workflow worktree exists and is properly set up.
/// Returns true if a new worktree was created, false if it already existed.
fn ensure_workflow_worktree(ctx: &WorkflowContext) -> Result<bool> {
    let workflow_path = &ctx.workflow_worktree;

    // If worktree already exists and is valid, we're done
    if workflow_path.exists() {
        return Ok(false);
    }

    // Check if the workflow branch already exists
    let branch_exists = check_branch_exists(ctx, DEFAULT_WORKFLOW_BRANCH)?;

    if branch_exists {
        // Branch exists - attach worktree to existing branch
        run_git(
            &ctx.repo_root,
            &["worktree", "add", workflow_path.to_str().unwrap(), DEFAULT_WORKFLOW_BRANCH],
        ).map_err(|e| {
            BurlError::GitError(format!(
                "failed to create worktree at '{}': {}\n\n\
                 Try manually: git worktree add {} {}",
                workflow_path.display(),
                e,
                workflow_path.display(),
                DEFAULT_WORKFLOW_BRANCH
            ))
        })?;
    } else {
        // Branch doesn't exist - create new branch and worktree
        // Get the current branch name to use as the base
        let base_branch = get_current_branch(ctx)?;

        run_git(
            &ctx.repo_root,
            &[
                "worktree",
                "add",
                "-b",
                DEFAULT_WORKFLOW_BRANCH,
                workflow_path.to_str().unwrap(),
                &base_branch,
            ],
        ).map_err(|e| {
            BurlError::GitError(format!(
                "failed to create worktree with new branch at '{}': {}\n\n\
                 Try manually: git worktree add -b {} {} {}",
                workflow_path.display(),
                e,
                DEFAULT_WORKFLOW_BRANCH,
                workflow_path.display(),
                base_branch
            ))
        })?;
    }

    Ok(true)
}

/// Check if a branch exists in the repository.
fn check_branch_exists(ctx: &WorkflowContext, branch: &str) -> Result<bool> {
    let result = run_git(
        &ctx.repo_root,
        &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
    );

    Ok(result.is_ok())
}

/// Get the current branch name.
fn get_current_branch(ctx: &WorkflowContext) -> Result<String> {
    let output = run_git(&ctx.repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = output.stdout.trim();

    // Handle detached HEAD
    if branch == "HEAD" {
        // Fallback to using HEAD directly
        Ok("HEAD".to_string())
    } else {
        Ok(branch.to_string())
    }
}

/// Create the workflow state directory structure.
fn create_workflow_structure(ctx: &WorkflowContext) -> Result<()> {
    // Create .workflow directory
    fs::create_dir_all(&ctx.workflow_state_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create workflow state directory '{}': {}",
            ctx.workflow_state_dir.display(),
            e
        ))
    })?;

    // Create bucket directories with .gitkeep files
    for bucket in BUCKETS {
        let bucket_path = ctx.bucket_path(bucket);
        create_dir_with_gitkeep(&bucket_path)?;
    }

    // Create events directory with .gitkeep
    let events_path = ctx.events_dir();
    create_dir_with_gitkeep(&events_path)?;

    // Create locks directory (no .gitkeep - it's untracked)
    fs::create_dir_all(&ctx.locks_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create locks directory '{}': {}",
            ctx.locks_dir.display(),
            e
        ))
    })?;

    // Create config.yaml if it doesn't exist
    let config_path = ctx.config_path();
    if !config_path.exists() {
        let default_config = Config::default();
        let yaml = default_config.to_yaml()?;
        atomic_write_file(&config_path, &yaml)?;
    }

    // Create .gitignore in .workflow to ignore locks/
    let gitignore_path = ctx.workflow_state_dir.join(".gitignore");
    if !gitignore_path.exists() {
        atomic_write_file(&gitignore_path, "# Local lock files (machine-specific, never commit)\nlocks/\n")?;
    }

    Ok(())
}

/// Create a directory with a .gitkeep file to ensure it's tracked by git.
fn create_dir_with_gitkeep(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create directory '{}': {}",
            path.display(),
            e
        ))
    })?;

    let gitkeep = path.join(".gitkeep");
    if !gitkeep.exists() {
        atomic_write_file(&gitkeep, "")?;
    }

    Ok(())
}

/// Create the .worktrees directory at repo root (untracked).
fn create_worktrees_dir(ctx: &WorkflowContext) -> Result<()> {
    if !ctx.worktrees_dir.exists() {
        fs::create_dir_all(&ctx.worktrees_dir).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create worktrees directory '{}': {}",
                ctx.worktrees_dir.display(),
                e
            ))
        })?;
    }

    Ok(())
}

/// Add .burl/ and .worktrees/ to .git/info/exclude.
fn add_to_git_exclude(ctx: &WorkflowContext) -> Result<()> {
    let exclude_path = ctx.repo_root.join(".git").join("info").join("exclude");

    // Ensure the info directory exists
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create git info directory: {}",
                e
            ))
        })?;
    }

    // Read existing content or start with empty
    let existing_content = fs::read_to_string(&exclude_path).unwrap_or_default();

    // Check what entries need to be added
    let mut entries_to_add = Vec::new();

    if !existing_content.lines().any(|line| line.trim() == ".burl/") {
        entries_to_add.push(".burl/");
    }

    if !existing_content.lines().any(|line| line.trim() == ".worktrees/") {
        entries_to_add.push(".worktrees/");
    }

    // If entries need to be added, append them
    if !entries_to_add.is_empty() {
        let mut new_content = existing_content;

        // Ensure there's a newline before our additions
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }

        // Add a comment and our entries
        if !new_content.contains("# burl workflow directories") {
            new_content.push_str("\n# burl workflow directories\n");
        }

        for entry in entries_to_add {
            new_content.push_str(entry);
            new_content.push('\n');
        }

        atomic_write_file(&exclude_path, &new_content)?;
    }

    Ok(())
}

/// Commit the workflow structure to the workflow branch.
fn commit_workflow_structure(ctx: &WorkflowContext, is_new_worktree: bool) -> Result<()> {
    // Check if there are any changes to commit
    let status = run_git(&ctx.workflow_worktree, &["status", "--porcelain"])?;

    if status.stdout.is_empty() {
        // Nothing to commit
        return Ok(());
    }

    // Stage all workflow files
    run_git(&ctx.workflow_worktree, &["add", ".workflow/"])?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;

    if staged.stdout.is_empty() {
        // Nothing was staged (maybe only untracked files in locks/)
        return Ok(());
    }

    // Create commit message
    let commit_msg = if is_new_worktree {
        "Initialize burl workflow structure\n\nCreated:\n- .workflow/READY/\n- .workflow/DOING/\n- .workflow/QA/\n- .workflow/DONE/\n- .workflow/BLOCKED/\n- .workflow/events/\n- .workflow/config.yaml\n- .workflow/.gitignore"
    } else {
        "Update burl workflow structure"
    };

    run_git(&ctx.workflow_worktree, &["commit", "-m", commit_msg]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to commit workflow structure: {}\n\n\
             You may need to configure git user.name and user.email:\n\
             git config user.name \"Your Name\"\n\
             git config user.email \"you@example.com\"",
            e
        ))
    })?;

    Ok(())
}

/// Push the workflow branch to the remote.
fn push_workflow_branch(ctx: &WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", "-u", &config.remote, DEFAULT_WORKFLOW_BRANCH],
    ).map_err(|e| {
        BurlError::GitError(format!(
            "failed to push workflow branch: {}\n\n\
             You can push manually with:\n\
             git -C {} push -u {} {}",
            e,
            ctx.workflow_worktree.display(),
            config.remote,
            DEFAULT_WORKFLOW_BRANCH
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a temporary git repository for testing.
    fn create_test_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        // Configure git user for commits
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        // Create initial commit (required for worktree creation)
        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        temp_dir
    }

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
            assert!(bucket_path.join(".gitkeep").exists(), "Bucket {} should have .gitkeep", bucket);
        }

        // Verify events directory
        assert!(ctx.events_dir().exists());
        assert!(ctx.events_dir().join(".gitkeep").exists());

        // Verify locks directory
        assert!(ctx.locks_dir.exists());
        // locks/ should NOT have .gitkeep (it's untracked)
        assert!(!ctx.locks_dir.join(".gitkeep").exists());

        // Verify config.yaml
        assert!(ctx.config_path().exists());

        // Verify .gitignore
        let gitignore_path = ctx.workflow_state_dir.join(".gitignore");
        assert!(gitignore_path.exists());
        let gitignore_content = std::fs::read_to_string(&gitignore_path).unwrap();
        assert!(gitignore_content.contains("locks/"));
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
        assert_eq!(content.matches(".burl/").count(), content2.matches(".burl/").count());
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
    fn test_full_init_flow() {
        let temp_dir = create_test_repo();

        // Change to the test directory to simulate running from repo root
        let _original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

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

        // Restore original directory
        std::env::set_current_dir(_original_dir).ok();
    }
}
