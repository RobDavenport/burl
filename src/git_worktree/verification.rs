//! Worktree verification operations.

use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::Path;

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
    use crate::test_support::create_test_repo;
    use std::process::Command;

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
}
