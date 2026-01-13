//! Git command runner for burl.
//!
//! Provides a safe wrapper around git commands with captured stdout/stderr
//! and structured error handling. All git operations should go through this module.

use crate::error::{BurlError, Result};
use std::path::Path;
use std::process::{Command, Output};

/// Result of a successful git command execution.
#[derive(Debug, Clone)]
pub struct GitOutput {
    /// Standard output from the command (trimmed).
    pub stdout: String,
    /// Standard error from the command (trimmed).
    pub stderr: String,
}

impl GitOutput {
    /// Create a new GitOutput from raw output bytes.
    fn from_output(output: &Output) -> Self {
        Self {
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        }
    }

    /// Returns true if stdout is empty.
    pub fn is_empty(&self) -> bool {
        self.stdout.is_empty()
    }

    /// Returns stdout lines as a vector.
    pub fn lines(&self) -> Vec<&str> {
        if self.stdout.is_empty() {
            Vec::new()
        } else {
            self.stdout.lines().collect()
        }
    }
}

/// Run a git command with the specified working directory.
///
/// # Arguments
///
/// * `cwd` - The working directory to run the command in
/// * `args` - The git command arguments (without "git" prefix)
///
/// # Returns
///
/// * `Ok(GitOutput)` - On successful execution (exit code 0)
/// * `Err(BurlError::GitError)` - On non-zero exit code (mapped to exit code 3)
///
/// # Examples
///
/// ```no_run
/// use burl::git::run_git;
/// use std::path::Path;
///
/// let output = run_git(Path::new("."), &["status", "--porcelain"])?;
/// println!("Changes: {}", output.stdout);
/// # Ok::<(), burl::error::BurlError>(())
/// ```
pub fn run_git<P: AsRef<Path>>(cwd: P, args: &[&str]) -> Result<GitOutput> {
    let cwd = cwd.as_ref();

    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| {
            BurlError::GitError(format!(
                "failed to execute git {}: {}",
                args.first().unwrap_or(&""),
                e
            ))
        })?;

    let git_output = GitOutput::from_output(&output);

    if output.status.success() {
        Ok(git_output)
    } else {
        let exit_code = output.status.code().unwrap_or(-1);
        let error_msg = if git_output.stderr.is_empty() {
            git_output.stdout.clone()
        } else {
            git_output.stderr.clone()
        };

        Err(BurlError::GitError(format!(
            "git {} failed (exit code {}): {}",
            args.first().unwrap_or(&""),
            exit_code,
            error_msg
        )))
    }
}

/// Get the repository root directory using `git rev-parse --show-toplevel`.
///
/// This works correctly from any location within a git repository,
/// including from within worktrees.
///
/// # Arguments
///
/// * `cwd` - The current working directory to start the search from
///
/// # Returns
///
/// * `Ok(PathBuf)` - The absolute path to the repository root
/// * `Err(BurlError::UserError)` - If not inside a git repository (exit code 1)
pub fn get_repo_root<P: AsRef<Path>>(cwd: P) -> Result<std::path::PathBuf> {
    let output = run_git_for_repo_detection(cwd.as_ref(), &["rev-parse", "--show-toplevel"])?;
    Ok(std::path::PathBuf::from(&output.stdout))
}

/// Internal helper that returns a UserError instead of GitError for repo detection.
/// This ensures "not in a git repo" is a clean user error (exit 1) not a git error (exit 3).
fn run_git_for_repo_detection<P: AsRef<Path>>(cwd: P, args: &[&str]) -> Result<GitOutput> {
    let cwd = cwd.as_ref();

    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| {
            BurlError::UserError(format!("failed to execute git: {} (is git installed?)", e))
        })?;

    let git_output = GitOutput::from_output(&output);

    if output.status.success() {
        Ok(git_output)
    } else {
        // Check if this is a "not a git repository" error
        let stderr = &git_output.stderr;
        if stderr.contains("not a git repository") || stderr.contains("fatal:") {
            Err(BurlError::UserError(
                "not inside a git repository. Run this command from within a git repository."
                    .to_string(),
            ))
        } else {
            Err(BurlError::UserError(format!(
                "git command failed: {}",
                if stderr.is_empty() {
                    &git_output.stdout
                } else {
                    stderr
                }
            )))
        }
    }
}

/// Get the path to the main worktree (the original clone location).
///
/// When run from within a linked worktree, this returns the path to the
/// main worktree. When run from the main worktree, it returns that path.
///
/// # Arguments
///
/// * `cwd` - The current working directory
///
/// # Returns
///
/// * `Ok(PathBuf)` - The absolute path to the main worktree
/// * `Err(BurlError)` - On failure
pub fn get_main_worktree<P: AsRef<Path>>(cwd: P) -> Result<std::path::PathBuf> {
    let cwd = cwd.as_ref();
    // git worktree list --porcelain gives us the main worktree first
    let output = run_git(cwd, &["worktree", "list", "--porcelain"])?;

    // Parse the first worktree entry
    // Format: "worktree /path/to/worktree\nHEAD abc123\nbranch refs/heads/main\n\nworktree ..."
    for line in output.stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            return Ok(std::path::PathBuf::from(path));
        }
    }

    // Fallback to rev-parse if worktree list doesn't work
    get_repo_root(cwd)
}

/// Check if the working directory has uncommitted tracked changes.
///
/// Uses `git status --porcelain --untracked-files=no` to check for
/// staged or unstaged changes to tracked files.
///
/// # Arguments
///
/// * `cwd` - The working directory to check
///
/// # Returns
///
/// * `Ok(true)` - If there are uncommitted tracked changes
/// * `Ok(false)` - If the working directory is clean (tracked files only)
/// * `Err(BurlError)` - On failure
pub fn has_uncommitted_changes<P: AsRef<Path>>(cwd: P) -> Result<bool> {
    let output = run_git(cwd, &["status", "--porcelain", "--untracked-files=no"])?;
    Ok(!output.is_empty())
}

/// Verify that the workflow worktree has no uncommitted tracked changes.
///
/// This is a precondition check for any command that commits workflow state.
/// If the workflow worktree has changes, the operation should be aborted
/// with actionable guidance.
///
/// # Arguments
///
/// * `workflow_worktree` - Path to the workflow worktree (e.g., `.burl/`)
///
/// # Returns
///
/// * `Ok(())` - If the worktree is clean
/// * `Err(BurlError::UserError)` - If there are uncommitted changes
pub fn ensure_workflow_worktree_clean<P: AsRef<Path>>(workflow_worktree: P) -> Result<()> {
    let workflow_worktree = workflow_worktree.as_ref();

    if has_uncommitted_changes(workflow_worktree)? {
        Err(BurlError::UserError(format!(
            "workflow worktree has uncommitted changes.\n\
             Path: {}\n\n\
             Please commit, stash, or revert changes in the workflow worktree before proceeding.\n\
             You can check the status with: git -C {} status",
            workflow_worktree.display(),
            workflow_worktree.display()
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::create_test_repo;
    use tempfile::TempDir;

    #[test]
    fn test_run_git_success() {
        let temp_dir = create_test_repo();
        let result = run_git(temp_dir.path(), &["status", "--porcelain"]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_git_captures_stdout() {
        let temp_dir = create_test_repo();
        let result = run_git(temp_dir.path(), &["rev-parse", "--show-toplevel"]);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.stdout.is_empty());
    }

    #[test]
    fn test_run_git_failure_returns_git_error() {
        let temp_dir = create_test_repo();
        let result = run_git(temp_dir.path(), &["checkout", "nonexistent-branch"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BurlError::GitError(_)));
    }

    #[test]
    fn test_get_repo_root_from_root() {
        let temp_dir = create_test_repo();
        let result = get_repo_root(temp_dir.path());
        assert!(result.is_ok());
        let root = result.unwrap();
        // Canonicalize both paths for comparison (handles symlinks, case, etc.)
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = root.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_get_repo_root_from_subdirectory() {
        let temp_dir = create_test_repo();
        let subdir = temp_dir.path().join("subdir").join("nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let result = get_repo_root(&subdir);
        assert!(result.is_ok());
        let root = result.unwrap();
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = root.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_get_repo_root_outside_repo_returns_user_error() {
        let temp_dir = TempDir::new().unwrap(); // Not a git repo
        let result = get_repo_root(temp_dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should be UserError (exit 1), not GitError (exit 3)
        assert!(matches!(err, BurlError::UserError(_)));
        assert!(err.to_string().contains("not inside a git repository"));
    }

    #[test]
    fn test_has_uncommitted_changes_clean_repo() {
        let temp_dir = create_test_repo();
        let result = has_uncommitted_changes(temp_dir.path());
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Clean repo
    }

    #[test]
    fn test_has_uncommitted_changes_with_changes() {
        let temp_dir = create_test_repo();
        // Modify a tracked file
        std::fs::write(temp_dir.path().join("README.md"), "# Modified\n").unwrap();

        let result = has_uncommitted_changes(temp_dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap()); // Has changes
    }

    #[test]
    fn test_has_uncommitted_changes_ignores_untracked() {
        let temp_dir = create_test_repo();
        // Add an untracked file
        std::fs::write(temp_dir.path().join("untracked.txt"), "untracked\n").unwrap();

        let result = has_uncommitted_changes(temp_dir.path());
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Untracked files don't count
    }

    #[test]
    fn test_ensure_workflow_worktree_clean_success() {
        let temp_dir = create_test_repo();
        let result = ensure_workflow_worktree_clean(temp_dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_ensure_workflow_worktree_clean_fails_with_changes() {
        let temp_dir = create_test_repo();
        std::fs::write(temp_dir.path().join("README.md"), "# Modified\n").unwrap();

        let result = ensure_workflow_worktree_clean(temp_dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BurlError::UserError(_)));
        assert!(err.to_string().contains("uncommitted changes"));
    }

    #[test]
    fn test_git_output_lines() {
        let output = GitOutput {
            stdout: "line1\nline2\nline3".to_string(),
            stderr: String::new(),
        };
        assert_eq!(output.lines(), vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_git_output_lines_empty() {
        let output = GitOutput {
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(output.lines().is_empty());
    }

    #[test]
    fn test_git_output_is_empty() {
        let empty = GitOutput {
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(empty.is_empty());

        let not_empty = GitOutput {
            stdout: "something".to_string(),
            stderr: String::new(),
        };
        assert!(!not_empty.is_empty());
    }
}
