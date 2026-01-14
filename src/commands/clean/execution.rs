//! Cleanup execution logic.

use super::display::make_relative;
use super::planning::path_contains_traversal;
use super::types::{CleanupPlan, CleanupResult};
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::git_worktree::remove_worktree;
use std::fs;
use std::path::Path;

/// Execute the cleanup plan, removing worktrees and directories.
pub fn execute_cleanup(ctx: &WorkflowContext, plan: &CleanupPlan) -> Result<CleanupResult> {
    let mut result = CleanupResult::default();

    // Remove completed task worktrees
    for candidate in &plan.completed_worktrees {
        match remove_worktree_safe(&ctx.repo_root, &candidate.path) {
            Ok(()) => {
                println!(
                    "Removed: {}",
                    make_relative(&candidate.path, &ctx.repo_root)
                );
                result.removed_count += 1;
            }
            Err(e) => {
                result.skipped_count += 1;
                result.skipped.push((candidate.path.clone(), e.to_string()));
            }
        }
    }

    // Remove orphan worktrees
    for candidate in &plan.orphan_worktrees {
        match remove_worktree_safe(&ctx.repo_root, &candidate.path) {
            Ok(()) => {
                println!(
                    "Removed: {}",
                    make_relative(&candidate.path, &ctx.repo_root)
                );
                result.removed_count += 1;
            }
            Err(e) => {
                result.skipped_count += 1;
                result.skipped.push((candidate.path.clone(), e.to_string()));
            }
        }
    }

    // Remove orphan directories (not git worktrees)
    for path in &plan.orphan_directories {
        match remove_directory_safe(path, &ctx.worktrees_dir) {
            Ok(()) => {
                println!("Removed: {}", make_relative(path, &ctx.repo_root));
                result.removed_count += 1;
            }
            Err(e) => {
                result.skipped_count += 1;
                result.skipped.push((path.clone(), e.to_string()));
            }
        }
    }

    Ok(result)
}

/// Safely remove a git worktree.
fn remove_worktree_safe(repo_root: &Path, worktree_path: &Path) -> Result<()> {
    // Safety check: ensure path is under .worktrees/
    let worktrees_dir = repo_root.join(".worktrees");
    if !is_path_under(worktree_path, &worktrees_dir) {
        return Err(BurlError::UserError(format!(
            "refusing to remove worktree outside .worktrees/: {}",
            worktree_path.display()
        )));
    }

    // Safety check: no path traversal
    if path_contains_traversal(worktree_path) {
        return Err(BurlError::UserError(format!(
            "refusing to remove path with traversal: {}",
            worktree_path.display()
        )));
    }

    // Avoid data loss: refuse to force-remove a dirty worktree.
    if crate::git::has_worktree_changes(worktree_path)? {
        return Err(BurlError::UserError(format!(
            "refusing to remove worktree with uncommitted changes: {}\n\n\
             Review with:\n  git -C {} status\n\n\
             If you are sure and want to discard the changes, remove manually:\n  git -C {} worktree remove --force {}",
            worktree_path.display(),
            worktree_path.display(),
            repo_root.display(),
            worktree_path.display(),
        )));
    }

    // Use git worktree remove (with force since we're cleaning up)
    remove_worktree(repo_root, worktree_path, true)
}

/// Safely remove a directory that is not a git worktree.
fn remove_directory_safe(path: &Path, worktrees_dir: &Path) -> Result<()> {
    // Safety check: ensure path is under .worktrees/
    if !is_path_under(path, worktrees_dir) {
        return Err(BurlError::UserError(format!(
            "refusing to remove directory outside .worktrees/: {}",
            path.display()
        )));
    }

    // Safety check: no path traversal
    if path_contains_traversal(path) {
        return Err(BurlError::UserError(format!(
            "refusing to remove path with traversal: {}",
            path.display()
        )));
    }

    // Remove the directory
    fs::remove_dir_all(path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to remove directory '{}': {}",
            path.display(),
            e
        ))
    })?;

    Ok(())
}

/// Check if a path is under a given parent directory.
pub fn is_path_under(path: &Path, parent: &Path) -> bool {
    // Canonicalize both for accurate comparison
    let path_canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let parent_canonical = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());

    path_canonical.starts_with(&parent_canonical)
}
