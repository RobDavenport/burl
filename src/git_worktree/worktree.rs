//! Worktree creation, listing, and setup operations.

use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::git::run_git;
use std::path::{Path, PathBuf};

use super::branch::{branch_exists, create_branch};
use super::naming::{task_branch_name, task_worktree_path};
use super::remote::{fetch_main, get_base_sha};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{create_test_repo, create_test_repo_with_remote};
    use std::process::Command;

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
    fn test_create_worktree() {
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
        super::super::verification::verify_worktree_branch(&info.path, &info.branch).unwrap();

        // Cleanup
        super::super::cleanup::cleanup_task_worktree(path, &info.branch, Some(&info.path), false)
            .unwrap();
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
        super::super::cleanup::cleanup_task_worktree(path, &info1.branch, Some(&info1.path), false)
            .unwrap();
    }
}
