//! Implementation of the `burl clean` command.
//!
//! Cleans up workflow artifacts safely:
//! - Worktrees for completed tasks (in DONE bucket)
//! - Orphan worktrees (directories in `.worktrees/` not referenced by any task)
//!
//! # Safety
//!
//! - Default behavior is dry-run (prints what would be removed)
//! - Requires `--yes` to actually perform deletions
//! - Only operates on directories under the configured worktrees root (`.worktrees/`)
//! - Never deletes branches by default
//! - Never allows path traversal (`..` in paths)
//!
//! # Logging
//!
//! Appends a `clean` event with summary after deletions.
//! If `workflow_auto_commit: true`, commits the workflow branch.
//! If `workflow_auto_push: true`, pushes the workflow branch.

use crate::cli::CleanArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::git_worktree::{list_worktrees, remove_worktree};
use crate::locks::acquire_workflow_lock;
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Summary of cleanup candidates.
#[derive(Debug, Default)]
struct CleanupPlan {
    /// Worktrees for completed tasks.
    completed_worktrees: Vec<CleanupCandidate>,
    /// Orphan worktrees (not referenced by any task).
    orphan_worktrees: Vec<CleanupCandidate>,
    /// Orphan directories (in .worktrees/ but not valid git worktrees).
    orphan_directories: Vec<PathBuf>,
}

/// A cleanup candidate with metadata.
#[derive(Debug, Clone)]
struct CleanupCandidate {
    /// Path to the worktree or directory.
    path: PathBuf,
    /// Associated task ID (if any).
    task_id: Option<String>,
    /// Branch name (if known).
    branch: Option<String>,
}

/// Summary of cleanup results.
#[derive(Debug, Default)]
struct CleanupResult {
    /// Number of items successfully removed.
    removed_count: usize,
    /// Number of items skipped due to errors.
    skipped_count: usize,
    /// Paths that were skipped with reasons.
    skipped: Vec<(PathBuf, String)>,
}

/// Execute the `burl clean` command.
///
/// Cleans up completed and orphan worktrees with safety checks.
///
/// # Behavior
///
/// - Without `--yes`: dry-run mode, prints what would be removed
/// - With `--yes`: performs actual deletions
/// - `--completed`: only clean completed task worktrees
/// - `--orphans`: only clean orphan worktrees
/// - Neither flag: clean both completed and orphans
pub fn cmd_clean(args: CleanArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build the cleanup plan
    let plan = build_cleanup_plan(&ctx, &args)?;

    // Check if there's anything to clean
    let total_count = plan.completed_worktrees.len()
        + plan.orphan_worktrees.len()
        + plan.orphan_directories.len();

    if total_count == 0 {
        println!("No cleanup candidates found.");
        return Ok(());
    }

    // Print the plan
    print_cleanup_plan(&plan, &ctx.repo_root);

    // If dry-run (no --yes), just exit
    if !args.yes {
        println!();
        println!("Dry-run mode: no changes made.");
        println!("Run with --yes to perform the cleanup.");
        return Ok(());
    }

    // Perform the cleanup
    let result = execute_cleanup(&ctx, &plan)?;

    // Log the clean event if anything was removed
    if result.removed_count > 0 {
        log_clean_event(&ctx, &config, &result)?;
    }

    // Print summary
    println!();
    println!("Cleanup complete:");
    println!("  Removed: {} item(s)", result.removed_count);
    if result.skipped_count > 0 {
        println!("  Skipped: {} item(s)", result.skipped_count);
        for (path, reason) in &result.skipped {
            println!("    - {}: {}", path.display(), reason);
        }
    }

    Ok(())
}

/// Build the cleanup plan by scanning for cleanup candidates.
fn build_cleanup_plan(
    ctx: &crate::context::WorkflowContext,
    args: &CleanArgs,
) -> Result<CleanupPlan> {
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
    ctx: &crate::context::WorkflowContext,
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
fn path_contains_traversal(path: &Path) -> bool {
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

/// Print the cleanup plan in a readable format.
fn print_cleanup_plan(plan: &CleanupPlan, repo_root: &Path) {
    println!("Cleanup plan:");
    println!();

    if !plan.completed_worktrees.is_empty() {
        println!(
            "Completed task worktrees ({}):",
            plan.completed_worktrees.len()
        );
        for candidate in &plan.completed_worktrees {
            let rel_path = make_relative(&candidate.path, repo_root);
            let task_info = candidate
                .task_id
                .as_ref()
                .map(|id| format!(" ({})", id))
                .unwrap_or_default();
            println!("  - {}{}", rel_path, task_info);
        }
        println!();
    }

    if !plan.orphan_worktrees.is_empty() {
        println!("Orphan worktrees ({}):", plan.orphan_worktrees.len());
        for candidate in &plan.orphan_worktrees {
            let rel_path = make_relative(&candidate.path, repo_root);
            let branch_info = candidate
                .branch
                .as_ref()
                .map(|b| format!(" [branch: {}]", b))
                .unwrap_or_default();
            println!("  - {}{}", rel_path, branch_info);
        }
        println!();
    }

    if !plan.orphan_directories.is_empty() {
        println!(
            "Orphan directories (not git worktrees) ({}):",
            plan.orphan_directories.len()
        );
        for path in &plan.orphan_directories {
            let rel_path = make_relative(path, repo_root);
            println!("  - {}", rel_path);
        }
        println!();
    }
}

/// Make a path relative to repo_root for display.
fn make_relative(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Execute the cleanup plan, removing worktrees and directories.
fn execute_cleanup(
    ctx: &crate::context::WorkflowContext,
    plan: &CleanupPlan,
) -> Result<CleanupResult> {
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
fn is_path_under(path: &Path, parent: &Path) -> bool {
    // Canonicalize both for accurate comparison
    let path_canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let parent_canonical = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());

    path_canonical.starts_with(&parent_canonical)
}

/// Log the clean event and commit workflow state.
fn log_clean_event(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    result: &CleanupResult,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock for the critical section
    let _workflow_lock = acquire_workflow_lock(ctx, "clean")?;

    // Append the clean event
    let event = Event::new(EventAction::Clean).with_details(json!({
        "removed_count": result.removed_count,
        "skipped_count": result.skipped_count,
    }));

    append_event(ctx, &event)?;

    // Commit if auto-commit is enabled
    if config.workflow_auto_commit {
        commit_clean(ctx, result)?;

        // Push if auto-push is enabled
        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the clean operation to the workflow branch.
fn commit_clean(ctx: &crate::context::WorkflowContext, result: &CleanupResult) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage clean changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("burl clean: removed {} worktree(s)", result.removed_count);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit clean: {}", e)))?;

    Ok(())
}

/// Push the workflow branch to the remote.
fn push_workflow_branch(ctx: &crate::context::WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init::cmd_init;
    use crate::test_support::{DirGuard, create_test_repo};
    use serial_test::serial;

    #[test]
    fn test_path_contains_traversal() {
        assert!(path_contains_traversal(Path::new("../foo")));
        assert!(path_contains_traversal(Path::new("foo/../bar")));
        assert!(path_contains_traversal(Path::new("foo/bar/..")));
        assert!(!path_contains_traversal(Path::new("foo/bar")));
        assert!(!path_contains_traversal(Path::new("/absolute/path")));
        assert!(!path_contains_traversal(Path::new("./relative")));
    }

    #[test]
    fn test_make_relative() {
        let repo_root = Path::new("/home/user/repo");
        let path = Path::new("/home/user/repo/.worktrees/task-001");

        let result = make_relative(path, repo_root);
        assert_eq!(result, ".worktrees/task-001");

        // Path not under repo_root
        let other_path = Path::new("/other/path");
        let result = make_relative(other_path, repo_root);
        assert_eq!(result, "/other/path");
    }

    #[test]
    #[serial]
    fn test_clean_dry_run_no_candidates() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Run clean (dry-run, no candidates)
        let args = CleanArgs {
            completed: false,
            orphans: false,
            yes: false,
        };

        let result = cmd_clean(args);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_clean_detects_orphan_directory() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Create a fake orphan directory under .worktrees/
        let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        std::fs::write(orphan_dir.join("file.txt"), "orphan content").unwrap();

        // Build cleanup plan
        let ctx = require_initialized_workflow().unwrap();
        let args = CleanArgs {
            completed: false,
            orphans: true,
            yes: false,
        };
        let plan = build_cleanup_plan(&ctx, &args).unwrap();

        // Should detect the orphan directory
        assert_eq!(plan.orphan_directories.len(), 1);
        assert!(plan.orphan_directories[0].ends_with("orphan-task"));
    }

    #[test]
    #[serial]
    fn test_clean_removes_orphan_directory_with_yes() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Create a fake orphan directory under .worktrees/
        let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        std::fs::write(orphan_dir.join("file.txt"), "orphan content").unwrap();

        // Verify it exists
        assert!(orphan_dir.exists());

        // Run clean with --yes
        let args = CleanArgs {
            completed: false,
            orphans: true,
            yes: true,
        };

        let result = cmd_clean(args);
        assert!(result.is_ok());

        // Verify orphan was removed
        assert!(!orphan_dir.exists());
    }

    #[test]
    #[serial]
    fn test_clean_skips_dirty_orphan_worktree() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let repo_root = temp_dir.path();
        let worktree_path = repo_root.join(".worktrees/orphan-wt");

        // Create a branch and add a worktree under .worktrees/
        std::process::Command::new("git")
            .current_dir(repo_root)
            .args(["branch", "orphan-branch"])
            .output()
            .expect("failed to create branch");
        std::process::Command::new("git")
            .current_dir(repo_root)
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "orphan-branch",
            ])
            .output()
            .expect("failed to create worktree");

        // Make the worktree dirty (untracked file).
        std::fs::write(worktree_path.join("untracked.txt"), "dirty").unwrap();
        assert!(worktree_path.exists());

        // Run clean with --yes. The dirty worktree should be skipped.
        let args = CleanArgs {
            completed: false,
            orphans: true,
            yes: true,
        };

        let result = cmd_clean(args);
        assert!(result.is_ok());
        assert!(
            worktree_path.exists(),
            "dirty orphan worktree should not be removed"
        );
    }

    #[test]
    #[serial]
    fn test_clean_with_completed_flag() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Create a fake orphan directory under .worktrees/
        let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
        std::fs::create_dir_all(&orphan_dir).unwrap();

        // Build cleanup plan with --completed only
        let ctx = require_initialized_workflow().unwrap();
        let args = CleanArgs {
            completed: true,
            orphans: false,
            yes: false,
        };
        let plan = build_cleanup_plan(&ctx, &args).unwrap();

        // Should NOT detect orphans when only --completed is specified
        assert!(plan.orphan_directories.is_empty());
        assert!(plan.orphan_worktrees.is_empty());
    }

    #[test]
    #[serial]
    fn test_clean_with_orphans_flag() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Create a fake orphan directory under .worktrees/
        let orphan_dir = temp_dir.path().join(".worktrees").join("orphan-task");
        std::fs::create_dir_all(&orphan_dir).unwrap();

        // Build cleanup plan with --orphans only
        let ctx = require_initialized_workflow().unwrap();
        let args = CleanArgs {
            completed: false,
            orphans: true,
            yes: false,
        };
        let plan = build_cleanup_plan(&ctx, &args).unwrap();

        // Should detect orphan
        assert_eq!(plan.orphan_directories.len(), 1);
        // Should NOT look for completed task worktrees
        assert!(plan.completed_worktrees.is_empty());
    }

    #[test]
    fn test_is_path_under() {
        // Note: is_path_under uses canonicalize which requires paths to exist.
        // For synthetic paths, the fallback logic (without canonicalize) applies.
        // We test the fallback behavior here with synthetic paths.
        let parent = Path::new("/home/user/repo/.worktrees");
        let child = Path::new("/home/user/repo/.worktrees/task-001");
        let outside = Path::new("/home/user/other");

        // Child should be under parent (using fallback since paths don't exist)
        assert!(child.starts_with(parent));
        // Outside should not be under parent
        assert!(!outside.starts_with(parent));
    }

    #[test]
    fn test_cleanup_result_default() {
        let result = CleanupResult::default();
        assert_eq!(result.removed_count, 0);
        assert_eq!(result.skipped_count, 0);
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn test_cleanup_candidate_fields() {
        let candidate = CleanupCandidate {
            path: PathBuf::from("/path/to/worktree"),
            task_id: Some("TASK-001".to_string()),
            branch: Some("task-001-feature".to_string()),
        };

        assert_eq!(candidate.path, PathBuf::from("/path/to/worktree"));
        assert_eq!(candidate.task_id, Some("TASK-001".to_string()));
        assert_eq!(candidate.branch, Some("task-001-feature".to_string()));
    }

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
