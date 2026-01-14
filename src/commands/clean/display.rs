//! Display and formatting utilities for clean command output.

use super::types::CleanupPlan;
use std::path::Path;

/// Print the cleanup plan in a readable format.
pub fn print_cleanup_plan(plan: &CleanupPlan, repo_root: &Path) {
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
pub fn make_relative(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
