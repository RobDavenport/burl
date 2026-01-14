//! Remote operations for fetching and determining base SHA.

use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::create_test_repo_with_remote;

    #[test]
    fn test_fetch_main_missing_remote() {
        let temp_dir = crate::test_support::create_test_repo();
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
}
