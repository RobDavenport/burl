//! Branch operations (create, check existence, delete).

use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run_git;
    use crate::test_support::create_test_repo;
    use std::process::Command;

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
}
