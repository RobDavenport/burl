//! Cleanup operations for worktrees and branches.

use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::Path;

use super::branch::{branch_exists, delete_branch};
use super::worktree::find_worktree_for_branch;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run_git;
    use crate::git_worktree::branch::create_branch;
    use crate::git_worktree::worktree::{create_worktree, find_worktree_for_branch};
    use crate::test_support::create_test_repo;

    #[test]
    fn test_remove_worktree() {
        let temp_dir = create_test_repo();
        let path = temp_dir.path();

        // Create branch
        use std::process::Command;
        Command::new("git")
            .current_dir(path)
            .args(["branch", "removable"])
            .output()
            .expect("failed to create branch");

        let worktree_path = path.join("removable-worktree");
        create_worktree(path, &worktree_path, "removable").unwrap();

        // Verify it exists
        assert!(worktree_path.exists());

        // Remove it
        remove_worktree(path, &worktree_path, false).unwrap();

        // Verify it's gone
        let found = find_worktree_for_branch(path, "removable").unwrap();
        assert!(found.is_none());
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
}
