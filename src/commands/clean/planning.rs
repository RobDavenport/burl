//! Cleanup plan building logic.

use super::types::{CleanupCandidate, CleanupPlan};
use crate::cli::CleanArgs;
use crate::context::WorkflowContext;
use crate::error::Result;
use crate::git_worktree::list_worktrees;
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Build the cleanup plan by scanning for cleanup candidates.
pub fn build_cleanup_plan(ctx: &WorkflowContext, args: &CleanArgs) -> Result<CleanupPlan> {
    let mut plan = CleanupPlan::default();

    // Determine what to clean
    // If neither --completed nor --orphans is specified, clean both
    let clean_completed = args.completed || !args.orphans;
    let clean_orphans = args.orphans || !args.completed;

    // Build task index
    let index = TaskIndex::build(ctx)?;

    // Collect all worktree paths referenced by tasks (for orphan detection)
    let mut referenced_paths: HashSet<PathBuf> = HashSet::new();
    let mut completed_worktree_paths: Vec<CleanupCandidate> = Vec::new();

    for task_info in index.all_tasks() {
        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if let Some(worktree_path) = &task.frontmatter.worktree {
            let full_path = normalize_worktree_path(&ctx.repo_root, worktree_path);

            // Normalize for comparison
            let canonical = full_path
                .canonicalize()
                .unwrap_or_else(|_| full_path.clone());
            referenced_paths.insert(canonical.clone());
            referenced_paths.insert(full_path.clone());

            // If task is in DONE, add to completed worktrees list
            if clean_completed && task_info.bucket == "DONE" && full_path.exists() {
                completed_worktree_paths.push(CleanupCandidate {
                    path: full_path,
                    task_id: Some(task_info.id.clone()),
                    branch: task.frontmatter.branch.clone(),
                });
            }
        }
    }

    plan.completed_worktrees = completed_worktree_paths;

    // Find orphan worktrees if requested
    if clean_orphans {
        find_orphan_worktrees(ctx, &referenced_paths, &mut plan)?;
    }

    Ok(plan)
}

/// Normalize a worktree path to be absolute.
fn normalize_worktree_path(repo_root: &Path, worktree_path: &str) -> PathBuf {
    let path = PathBuf::from(worktree_path);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

/// Find orphan worktrees under .worktrees/ that are not referenced by any task.
fn find_orphan_worktrees(
    ctx: &WorkflowContext,
    referenced_paths: &HashSet<PathBuf>,
    plan: &mut CleanupPlan,
) -> Result<()> {
    // Check if .worktrees/ exists
    if !ctx.worktrees_dir.exists() {
        return Ok(());
    }

    // Get list of git worktrees
    let git_worktrees: HashSet<PathBuf> = list_worktrees(&ctx.repo_root)
        .map(|wts| {
            wts.into_iter()
                .filter_map(|wt| wt.path.canonicalize().ok())
                .collect()
        })
        .unwrap_or_default();

    // Scan .worktrees/ directory
    let entries = match fs::read_dir(&ctx.worktrees_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip non-directories
        if !path.is_dir() {
            continue;
        }

        // Safety check: reject any path with ".." components
        if path_contains_traversal(&path) {
            continue;
        }

        // Normalize for comparison
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());

        // Check if this path is referenced by any task
        let is_referenced =
            referenced_paths.contains(&canonical) || referenced_paths.contains(&path);

        if is_referenced {
            continue;
        }

        // Check if it's a valid git worktree
        let is_git_worktree = git_worktrees.contains(&canonical);

        if is_git_worktree {
            // It's a git worktree but not referenced by any task
            // Try to get the branch name
            let branch = get_worktree_branch(&ctx.repo_root, &path);
            plan.orphan_worktrees.push(CleanupCandidate {
                path,
                task_id: None,
                branch,
            });
        } else {
            // It's just a directory, not a valid git worktree
            plan.orphan_directories.push(path);
        }
    }

    Ok(())
}

/// Check if a path contains any ".." traversal components.
pub fn path_contains_traversal(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return true;
        }
    }
    false
}

/// Get the branch name for a worktree, if possible.
fn get_worktree_branch(repo_root: &Path, worktree_path: &Path) -> Option<String> {
    let worktrees = list_worktrees(repo_root).ok()?;
    let canonical = worktree_path.canonicalize().ok()?;

    for wt in worktrees {
        let wt_canonical = wt.path.canonicalize().ok()?;
        if wt_canonical == canonical {
            return wt.branch;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_worktree_path_relative() {
        let repo_root = PathBuf::from("/home/user/repo");
        let result = normalize_worktree_path(&repo_root, ".worktrees/task-001");
        assert_eq!(result, PathBuf::from("/home/user/repo/.worktrees/task-001"));
    }

    #[test]
    fn test_normalize_worktree_path_absolute() {
        let repo_root = PathBuf::from("/home/user/repo");
        let result = normalize_worktree_path(&repo_root, "/absolute/path/worktree");
        assert_eq!(result, PathBuf::from("/absolute/path/worktree"));
    }
}
