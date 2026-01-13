//! Git worktree and task branch helpers for burl.
//!
//! This module provides the git operations needed for task branch and worktree
//! management during the claim, submit, approve, and cleanup lifecycle:
//!
//! - Fetching from remote
//! - Determining base_sha (origin/main HEAD at claim time)
//! - Creating/reusing task branches
//! - Creating/attaching task worktrees
//! - Removing worktrees and deleting branches
//!
//! All git failures are mapped to exit code 3 (BurlError::GitError).

use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::{Path, PathBuf};

/// Result of creating or attaching to a task worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Absolute path to the worktree directory.
    pub path: PathBuf,
    /// Name of the task branch.
    pub branch: String,
    /// Base SHA (commit at which the branch was created or origin/main HEAD).
    pub base_sha: String,
    /// Whether the worktree was reused (already existed).
    pub reused: bool,
}

/// Generate the conventional branch name for a task.
///
/// Format: `task-{numeric_id}-{slug}`
/// Example: `task-001-player-jump`
///
/// # Arguments
///
/// * `task_id` - The task ID (e.g., "TASK-001" or "001")
/// * `slug` - Optional slug for the task (derived from title if not provided)
pub fn task_branch_name(task_id: &str, slug: Option<&str>) -> String {
    // Extract numeric part from task ID (e.g., "TASK-001" -> "001", "001" -> "001")
    let numeric = task_id
        .strip_prefix("TASK-")
        .unwrap_or(task_id)
        .to_lowercase();

    match slug {
        Some(s) if !s.is_empty() => format!("task-{}-{}", numeric, sanitize_slug(s)),
        _ => format!("task-{}", numeric),
    }
}

/// Generate the conventional worktree path for a task.
///
/// Format: `.worktrees/task-{numeric_id}-{slug}/`
/// Example: `.worktrees/task-001-player-jump/`
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `task_id` - The task ID (e.g., "TASK-001")
/// * `slug` - Optional slug for the task
pub fn task_worktree_path(ctx: &WorkflowContext, task_id: &str, slug: Option<&str>) -> PathBuf {
    let branch_name = task_branch_name(task_id, slug);
    ctx.worktrees_dir.join(branch_name)
}

/// Sanitize a string for use in branch names.
///
/// Converts to lowercase, replaces spaces and special chars with hyphens,
/// removes consecutive hyphens, and trims leading/trailing hyphens.
fn sanitize_slug(s: &str) -> String {
    let mut result = String::new();
    let mut last_was_hyphen = true; // Start true to avoid leading hyphen

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            result.push('-');
            last_was_hyphen = true;
        }
    }

    // Trim trailing hyphen
    while result.ends_with('-') {
        result.pop();
    }

    result
}

/// Fetch the main branch from the remote.
///
/// Runs `git fetch <remote> <main_branch>` to ensure we have the latest
/// remote state before determining base_sha.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `remote` - Name of the remote (e.g., "origin")
/// * `main_branch` - Name of the main branch (e.g., "main")
///
/// # Returns
///
/// * `Ok(())` - Fetch succeeded
/// * `Err(BurlError::GitError)` - Fetch failed (exit code 3)
pub fn fetch_main<P: AsRef<Path>>(repo_root: P, remote: &str, main_branch: &str) -> Result<()> {
    let repo_root = repo_root.as_ref();

    // First check if the remote exists
    let remotes = run_git(repo_root, &["remote"])?;
    if !remotes.lines().contains(&remote) {
        return Err(BurlError::GitError(format!(
            "remote '{}' does not exist.\n\n\
             To fix this, either:\n\
             1. Set a different remote in config.yaml (remote: <name>)\n\
             2. Add the remote: git remote add {} <url>",
            remote, remote
        )));
    }

    run_git(repo_root, &["fetch", remote, main_branch]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to fetch {}/{}: {}\n\n\
             Make sure the remote '{}' is accessible and the branch '{}' exists.",
            remote, main_branch, e, remote, main_branch
        ))
    })?;

    Ok(())
}

/// Get the base SHA for a task (the HEAD of remote/main).
///
/// This should be called after fetch_main to ensure we have the latest state.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `remote` - Name of the remote (e.g., "origin")
/// * `main_branch` - Name of the main branch (e.g., "main")
///
/// # Returns
///
/// * `Ok(String)` - The full SHA of remote/main HEAD
/// * `Err(BurlError::GitError)` - Failed to resolve SHA (exit code 3)
pub fn get_base_sha<P: AsRef<Path>>(
    repo_root: P,
    remote: &str,
    main_branch: &str,
) -> Result<String> {
    let remote_ref = format!("{}/{}", remote, main_branch);

    let output = run_git(repo_root, &["rev-parse", &remote_ref]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to resolve base SHA for '{}': {}\n\n\
             Make sure you have fetched from the remote first (git fetch {} {}).",
            remote_ref, e, remote, main_branch
        ))
    })?;

    Ok(output.stdout)
}

/// Check if a branch exists locally.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `branch` - Name of the branch to check
pub fn branch_exists<P: AsRef<Path>>(repo_root: P, branch: &str) -> Result<bool> {
    let output = run_git(
        repo_root,
        &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
    );
    Ok(output.is_ok())
}

/// Create a new branch at the specified commit.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `branch` - Name of the branch to create
/// * `base_sha` - Commit SHA to create the branch at
///
/// # Returns
///
/// * `Ok(())` - Branch created successfully
/// * `Err(BurlError::GitError)` - Failed to create branch (exit code 3)
pub fn create_branch<P: AsRef<Path>>(repo_root: P, branch: &str, base_sha: &str) -> Result<()> {
    run_git(repo_root, &["branch", branch, base_sha]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to create branch '{}' at {}: {}",
            branch, base_sha, e
        ))
    })?;
    Ok(())
}

/// Information about an existing worktree.
#[derive(Debug, Clone)]
pub struct ExistingWorktree {
    /// Path to the worktree.
    pub path: PathBuf,
    /// Branch the worktree is on.
    pub branch: Option<String>,
    /// HEAD commit SHA.
    pub head_sha: String,
}

/// List all worktrees in the repository.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
pub fn list_worktrees<P: AsRef<Path>>(repo_root: P) -> Result<Vec<ExistingWorktree>> {
    let output = run_git(repo_root, &["worktree", "list", "--porcelain"])?;

    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_head: Option<String> = None;
    let mut current_branch: Option<String> = None;

    for line in output.stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            // Save previous worktree if complete
            if let (Some(path), Some(head)) = (current_path.take(), current_head.take()) {
                worktrees.push(ExistingWorktree {
                    path,
                    branch: current_branch.take(),
                    head_sha: head,
                });
            }
            current_path = Some(PathBuf::from(path));
        } else if let Some(sha) = line.strip_prefix("HEAD ") {
            current_head = Some(sha.to_string());
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            // Branch ref is like "refs/heads/branch-name"
            current_branch = branch_ref.strip_prefix("refs/heads/").map(String::from);
        } else if line == "detached" {
            current_branch = None;
        }
    }

    // Don't forget the last worktree
    if let (Some(path), Some(head)) = (current_path, current_head) {
        worktrees.push(ExistingWorktree {
            path,
            branch: current_branch,
            head_sha: head,
        });
    }

    Ok(worktrees)
}

/// Find an existing worktree for a branch.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `branch` - Name of the branch to find
pub fn find_worktree_for_branch<P: AsRef<Path>>(
    repo_root: P,
    branch: &str,
) -> Result<Option<ExistingWorktree>> {
    let worktrees = list_worktrees(repo_root)?;

    Ok(worktrees
        .into_iter()
        .find(|wt| wt.branch.as_deref() == Some(branch)))
}

/// Create a new worktree for a branch.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `worktree_path` - Path where the worktree should be created
/// * `branch` - Name of the branch (must already exist)
///
/// # Returns
///
/// * `Ok(())` - Worktree created successfully
/// * `Err(BurlError::GitError)` - Failed to create worktree (exit code 3)
pub fn create_worktree<P: AsRef<Path>>(
    repo_root: P,
    worktree_path: &Path,
    branch: &str,
) -> Result<()> {
    let worktree_str = worktree_path.to_string_lossy();

    // Ensure parent directory exists
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            BurlError::GitError(format!(
                "failed to create worktrees directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }

    run_git(repo_root, &["worktree", "add", &worktree_str, branch]).map_err(|e| {
        BurlError::GitError(format!(
            "failed to create worktree at '{}' for branch '{}': {}",
            worktree_str, branch, e
        ))
    })?;

    Ok(())
}

/// Create or reuse a task worktree.
///
/// This is the main entry point for setting up a task worktree during claim.
/// It handles:
/// 1. Fetching from remote to get latest state
/// 2. Determining base_sha
/// 3. Creating or reusing the task branch
/// 4. Creating or attaching to the task worktree
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `task_id` - The task ID (e.g., "TASK-001")
/// * `slug` - Optional slug for the task (derived from title)
/// * `remote` - Name of the remote (from config)
/// * `main_branch` - Name of the main branch (from config)
/// * `existing_branch` - Optional existing branch name from task metadata (for reuse)
/// * `existing_worktree` - Optional existing worktree path from task metadata (for reuse)
///
/// # Returns
///
/// * `Ok(WorktreeInfo)` - Information about the created/reused worktree
/// * `Err(BurlError::GitError)` - Git operation failed (exit code 3)
pub fn setup_task_worktree(
    ctx: &WorkflowContext,
    task_id: &str,
    slug: Option<&str>,
    remote: &str,
    main_branch: &str,
    existing_branch: Option<&str>,
    existing_worktree: Option<&str>,
) -> Result<WorktreeInfo> {
    // Step 1: Fetch main to ensure we have latest state
    fetch_main(&ctx.repo_root, remote, main_branch)?;

    // Step 2: Get base_sha
    let base_sha = get_base_sha(&ctx.repo_root, remote, main_branch)?;

    // Step 3: Determine branch name (use existing or generate new)
    let branch_name = match existing_branch {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => task_branch_name(task_id, slug),
    };

    // Step 4: Determine worktree path (use existing or generate new)
    let worktree_path = match existing_worktree {
        Some(wt) if !wt.is_empty() => {
            // If it's a relative path, make it absolute from repo root
            let path = PathBuf::from(wt);
            if path.is_absolute() {
                path
            } else {
                ctx.repo_root.join(path)
            }
        }
        _ => task_worktree_path(ctx, task_id, slug),
    };

    // Step 5: Check if worktree already exists and points to correct branch
    if let Some(existing) = find_worktree_for_branch(&ctx.repo_root, &branch_name)? {
        // Worktree exists for this branch - reuse it
        // Verify the path matches or update to actual path
        let actual_path = existing.path.clone();

        return Ok(WorktreeInfo {
            path: actual_path,
            branch: branch_name,
            base_sha,
            reused: true,
        });
    }

    // Step 6: Check if worktree path exists but might be orphaned
    if worktree_path.exists() {
        // Check if it's a valid worktree
        let worktrees = list_worktrees(&ctx.repo_root)?;
        let path_matches = worktrees
            .iter()
            .any(|wt| wt.path == worktree_path || paths_equivalent(&wt.path, &worktree_path));

        if path_matches {
            // It's a valid worktree for a different branch - this is an error
            return Err(BurlError::GitError(format!(
                "worktree path '{}' already exists for a different branch.\n\n\
                 Either remove the existing worktree or choose a different task slug.",
                worktree_path.display()
            )));
        } else {
            // Path exists but isn't a worktree - could be leftover directory
            return Err(BurlError::GitError(format!(
                "path '{}' already exists but is not a valid worktree.\n\n\
                 Remove the directory manually and try again:\n\
                 rm -rf {}",
                worktree_path.display(),
                worktree_path.display()
            )));
        }
    }

    // Step 7: Create branch if it doesn't exist
    if !branch_exists(&ctx.repo_root, &branch_name)? {
        create_branch(&ctx.repo_root, &branch_name, &base_sha)?;
    }

    // Step 8: Create worktree
    create_worktree(&ctx.repo_root, &worktree_path, &branch_name)?;

    Ok(WorktreeInfo {
        path: worktree_path,
        branch: branch_name,
        base_sha,
        reused: false,
    })
}

/// Check if two paths are equivalent (handling symlinks, case, etc.).
fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a_canon), Ok(b_canon)) => a_canon == b_canon,
        _ => a == b,
    }
}

/// Remove a worktree.
///
/// Uses `git worktree remove <path>`. Does NOT use --force by default
/// to avoid data loss.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `worktree_path` - Path to the worktree to remove
/// * `force` - If true, use --force (required if worktree has uncommitted changes)
///
/// # Returns
///
/// * `Ok(())` - Worktree removed successfully
/// * `Err(BurlError::GitError)` - Failed to remove worktree (exit code 3)
pub fn remove_worktree<P: AsRef<Path>>(
    repo_root: P,
    worktree_path: &Path,
    force: bool,
) -> Result<()> {
    let worktree_str = worktree_path.to_string_lossy();

    let args: Vec<&str> = if force {
        vec!["worktree", "remove", "--force", &worktree_str]
    } else {
        vec!["worktree", "remove", &worktree_str]
    };

    run_git(repo_root, &args).map_err(|e| {
        let force_hint = if !force {
            "\n\nIf the worktree has uncommitted changes and you want to remove it anyway,\n\
             use force removal (not recommended without reviewing changes first)."
        } else {
            ""
        };

        BurlError::GitError(format!(
            "failed to remove worktree '{}': {}{}",
            worktree_str, e, force_hint
        ))
    })?;

    Ok(())
}

/// Delete a branch.
///
/// Uses `git branch -d <branch>` (safe delete, requires fully merged).
/// Does NOT use `-D` (force delete) by default to prevent data loss.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `branch` - Name of the branch to delete
/// * `force` - If true, use -D (force delete, even if not merged)
///
/// # Returns
///
/// * `Ok(())` - Branch deleted successfully
/// * `Err(BurlError::GitError)` - Failed to delete branch (exit code 3)
pub fn delete_branch<P: AsRef<Path>>(repo_root: P, branch: &str, force: bool) -> Result<()> {
    let delete_flag = if force { "-D" } else { "-d" };

    run_git(repo_root, &["branch", delete_flag, branch]).map_err(|e| {
        let force_hint = if !force {
            "\n\nIf the branch is not fully merged and you want to delete it anyway,\n\
             use force deletion (not recommended without verifying the changes are safe to lose)."
        } else {
            ""
        };

        BurlError::GitError(format!(
            "failed to delete branch '{}': {}{}",
            branch, e, force_hint
        ))
    })?;

    Ok(())
}

/// Clean up a task's worktree and branch.
///
/// This removes the worktree first (if it exists), then deletes the branch.
/// Used during approve (after merge) or cleanup operations.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
/// * `branch` - Name of the task branch
/// * `worktree_path` - Optional path to the worktree (if known)
/// * `force` - If true, force removal even if uncommitted changes exist
///
/// # Returns
///
/// * `Ok(())` - Cleanup completed successfully
/// * `Err(BurlError::GitError)` - Cleanup failed (exit code 3)
pub fn cleanup_task_worktree<P: AsRef<Path>>(
    repo_root: P,
    branch: &str,
    worktree_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let repo_root = repo_root.as_ref();

    // First, try to find and remove the worktree
    if let Some(path) = worktree_path {
        if path.exists() {
            remove_worktree(repo_root, path, force)?;
        }
    } else {
        // Try to find worktree by branch
        if let Some(wt) = find_worktree_for_branch(repo_root, branch)? {
            remove_worktree(repo_root, &wt.path, force)?;
        }
    }

    // Then delete the branch (if it exists)
    if branch_exists(repo_root, branch)? {
        delete_branch(repo_root, branch, force)?;
    }

    Ok(())
}

/// Get the current branch name in a worktree.
///
/// # Arguments
///
/// * `worktree_path` - Path to the worktree
///
/// # Returns
///
/// * `Ok(String)` - The current branch name
/// * `Err(BurlError::GitError)` - Failed to get branch name (exit code 3)
pub fn get_current_branch<P: AsRef<Path>>(worktree_path: P) -> Result<String> {
    let output = run_git(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok(output.stdout)
}

/// Verify that a worktree is on the expected branch.
///
/// # Arguments
///
/// * `worktree_path` - Path to the worktree
/// * `expected_branch` - The branch the worktree should be on
///
/// # Returns
///
/// * `Ok(())` - Worktree is on the expected branch
/// * `Err(BurlError::GitError)` - Worktree is on a different branch
pub fn verify_worktree_branch<P: AsRef<Path>>(
    worktree_path: P,
    expected_branch: &str,
) -> Result<()> {
    let actual_branch = get_current_branch(&worktree_path)?;

    if actual_branch != expected_branch {
        return Err(BurlError::GitError(format!(
            "worktree is on branch '{}' but expected '{}'.\n\n\
             The worktree may have been modified outside of burl.",
            actual_branch, expected_branch
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{create_test_repo, create_test_repo_with_remote};
    use std::process::Command;

    #[test]
    fn test_task_branch_name() {
        assert_eq!(task_branch_name("TASK-001", None), "task-001");
        assert_eq!(task_branch_name("TASK-001", Some("")), "task-001");
        assert_eq!(
            task_branch_name("TASK-001", Some("player-jump")),
            "task-001-player-jump"
        );
        assert_eq!(
            task_branch_name("TASK-001", Some("Player Jump")),
            "task-001-player-jump"
        );
        assert_eq!(task_branch_name("001", Some("feature")), "task-001-feature");
    }

    #[test]
    fn test_sanitize_slug() {
        assert_eq!(sanitize_slug("player-jump"), "player-jump");
        assert_eq!(sanitize_slug("Player Jump"), "player-jump");
        assert_eq!(sanitize_slug("Player  Jump"), "player-jump");
        assert_eq!(sanitize_slug("Feature: New Thing!"), "feature-new-thing");
        assert_eq!(sanitize_slug("  spaces  "), "spaces");
        assert_eq!(sanitize_slug("CamelCase"), "camelcase");
        assert_eq!(sanitize_slug("with_underscores"), "with-underscores");
    }

    #[test]
    fn test_branch_exists() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Rename default branch to main
        let _ = Command::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        // main should exist
        assert!(branch_exists(path, "main").unwrap());

        // non-existent branch should not exist
        assert!(!branch_exists(path, "nonexistent").unwrap());
    }

    #[test]
    fn test_create_branch() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Get current HEAD
        let head = run_git(path, &["rev-parse", "HEAD"]).unwrap().stdout;

        // Create a new branch
        create_branch(path, "test-branch", &head).unwrap();

        // Verify it exists
        assert!(branch_exists(path, "test-branch").unwrap());
    }

    #[test]
    fn test_list_worktrees() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Initially only the main worktree
        let worktrees = list_worktrees(path).unwrap();
        assert_eq!(worktrees.len(), 1);

        // Create a branch and worktree
        Command::new("git")
            .current_dir(path)
            .args(["branch", "test-branch"])
            .output()
            .expect("failed to create branch");

        let worktree_path = path.join("test-worktree");
        Command::new("git")
            .current_dir(path)
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "test-branch",
            ])
            .output()
            .expect("failed to create worktree");

        let worktrees = list_worktrees(path).unwrap();
        assert_eq!(worktrees.len(), 2);

        // Find the new worktree
        let test_wt = worktrees
            .iter()
            .find(|wt| wt.branch.as_deref() == Some("test-branch"));
        assert!(test_wt.is_some());
    }

    #[test]
    fn test_find_worktree_for_branch() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Create a branch and worktree
        Command::new("git")
            .current_dir(path)
            .args(["branch", "find-test"])
            .output()
            .expect("failed to create branch");

        let worktree_path = path.join("find-test-worktree");
        Command::new("git")
            .current_dir(path)
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "find-test",
            ])
            .output()
            .expect("failed to create worktree");

        // Should find the worktree
        let found = find_worktree_for_branch(path, "find-test").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().branch, Some("find-test".to_string()));

        // Should not find non-existent branch
        let not_found = find_worktree_for_branch(path, "nonexistent").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_create_and_remove_worktree() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Create branch
        Command::new("git")
            .current_dir(path)
            .args(["branch", "removable"])
            .output()
            .expect("failed to create branch");

        let worktree_path = path.join("removable-worktree");
        create_worktree(path, &worktree_path, "removable").unwrap();

        // Verify it exists
        assert!(worktree_path.exists());
        let found = find_worktree_for_branch(path, "removable").unwrap();
        assert!(found.is_some());

        // Remove it
        remove_worktree(path, &worktree_path, false).unwrap();

        // Verify it's gone
        let found = find_worktree_for_branch(path, "removable").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_delete_branch() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Rename default branch to main
        let _ = Command::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        // Create and then delete a branch
        let head = run_git(path, &["rev-parse", "HEAD"]).unwrap().stdout;
        create_branch(path, "deletable", &head).unwrap();

        assert!(branch_exists(path, "deletable").unwrap());

        delete_branch(path, "deletable", false).unwrap();

        assert!(!branch_exists(path, "deletable").unwrap());
    }

    #[test]
    fn test_get_current_branch() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Rename default branch to main
        let _ = Command::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        let branch = get_current_branch(path).unwrap();
        assert_eq!(branch, "main");
    }

    #[test]
    fn test_verify_worktree_branch_success() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Rename default branch to main
        let _ = Command::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        let result = verify_worktree_branch(path, "main");
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_worktree_branch_failure() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Rename default branch to main
        let _ = Command::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        let result = verify_worktree_branch(path, "wrong-branch");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected"));
    }

    #[test]
    fn test_fetch_main_missing_remote() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        let result = fetch_main(path, "nonexistent", "main");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("does not exist"));
        assert!(matches!(err, BurlError::GitError(_)));
    }

    #[test]
    fn test_fetch_main_success() {
        let temp_dir = create_test_repo_with_remote();
        let path = temp_dir.path();

        let result = fetch_main(path, "origin", "main");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_base_sha_success() {
        let temp_dir = create_test_repo_with_remote();
        let path = temp_dir.path();

        // Fetch first
        fetch_main(path, "origin", "main").unwrap();

        // Get base SHA
        let sha = get_base_sha(path, "origin", "main").unwrap();
        assert!(!sha.is_empty());
        assert_eq!(sha.len(), 40); // Full SHA
    }

    #[test]
    fn test_cleanup_task_worktree() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Create branch and worktree
        let head = run_git(path, &["rev-parse", "HEAD"]).unwrap().stdout;
        create_branch(path, "cleanup-test", &head).unwrap();

        let worktree_path = path.join("cleanup-worktree");
        create_worktree(path, &worktree_path, "cleanup-test").unwrap();

        // Verify they exist
        assert!(worktree_path.exists());
        assert!(branch_exists(path, "cleanup-test").unwrap());

        // Cleanup
        cleanup_task_worktree(path, "cleanup-test", Some(&worktree_path), false).unwrap();

        // Verify they're gone
        assert!(!branch_exists(path, "cleanup-test").unwrap());
    }

    #[test]
    fn test_integration_setup_task_worktree() {
        let temp_dir = create_test_repo_with_remote();
        let path = temp_dir.path();

        // Create a minimal WorkflowContext
        let ctx = WorkflowContext {
            repo_root: path.to_path_buf(),
            workflow_worktree: path.join(".burl"),
            workflow_state_dir: path.join(".burl").join(".workflow"),
            locks_dir: path.join(".burl").join(".workflow").join("locks"),
            worktrees_dir: path.join(".worktrees"),
        };

        // Setup a task worktree
        let info = setup_task_worktree(
            &ctx,
            "TASK-001",
            Some("test-feature"),
            "origin",
            "main",
            None,
            None,
        )
        .unwrap();

        // Verify the result
        assert!(!info.reused);
        assert_eq!(info.branch, "task-001-test-feature");
        assert!(!info.base_sha.is_empty());
        assert!(info.path.exists());

        // Verify the worktree is on the correct branch
        verify_worktree_branch(&info.path, &info.branch).unwrap();

        // Cleanup
        cleanup_task_worktree(path, &info.branch, Some(&info.path), false).unwrap();
    }

    #[test]
    fn test_integration_reuse_existing_worktree() {
        let temp_dir = create_test_repo_with_remote();
        let path = temp_dir.path();

        let ctx = WorkflowContext {
            repo_root: path.to_path_buf(),
            workflow_worktree: path.join(".burl"),
            workflow_state_dir: path.join(".burl").join(".workflow"),
            locks_dir: path.join(".burl").join(".workflow").join("locks"),
            worktrees_dir: path.join(".worktrees"),
        };

        // First setup
        let info1 = setup_task_worktree(
            &ctx,
            "TASK-002",
            Some("reuse-test"),
            "origin",
            "main",
            None,
            None,
        )
        .unwrap();
        assert!(!info1.reused);

        // Second setup with same task should reuse
        let info2 = setup_task_worktree(
            &ctx,
            "TASK-002",
            Some("reuse-test"),
            "origin",
            "main",
            Some(&info1.branch),
            Some(&info1.path.to_string_lossy()),
        )
        .unwrap();
        assert!(info2.reused);
        assert_eq!(info2.branch, info1.branch);

        // Cleanup
        cleanup_task_worktree(path, &info1.branch, Some(&info1.path), false).unwrap();
    }
}
