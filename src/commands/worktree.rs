//! Implementation of the `burl worktree` command.
//!
//! Prints the recorded worktree path for a task.

use crate::cli::WorktreeArgs;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::task::TaskFile;
use crate::workflow::{validate_task_id, TaskIndex, BUCKETS};

/// Execute the `burl worktree` command.
///
/// Prints the recorded worktree path from the task's frontmatter.
/// If no worktree is recorded, exits with an error.
pub fn cmd_worktree(args: WorktreeArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;

    // Task ID is required for this command
    let task_id_str = args.task_id.ok_or_else(|| {
        BurlError::UserError(
            "task ID is required.\n\n\
             Usage: burl worktree TASK-001"
                .to_string(),
        )
    })?;

    // Validate and normalize task ID
    let task_id = validate_task_id(&task_id_str)?;

    // Build task index and find the task
    let index = TaskIndex::build(&ctx)?;

    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Searched buckets: {}\n\n\
             Use `burl status` to see all tasks.",
            task_id,
            BUCKETS.join(", ")
        ))
    })?;

    // Load the task file
    let task = TaskFile::load(&task_info.path)?;

    // Get the worktree path
    let worktree = task.frontmatter.worktree.ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no worktree (not claimed?).\n\n\
             The worktree is created when a task is claimed.\n\
             Current bucket: {}\n\n\
             To claim this task, run: burl claim {}",
            task_id, task_info.bucket, task_id
        ))
    })?;

    // Print just the worktree path (for scripting)
    println!("{}", worktree);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AddArgs, WorktreeArgs};
    use crate::commands::add::cmd_add;
    use crate::commands::init::cmd_init;
    use serial_test::serial;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// RAII guard for changing current directory - restores on drop.
    struct DirGuard {
        original: PathBuf,
    }

    impl DirGuard {
        fn new(new_dir: &std::path::Path) -> Self {
            let original = std::env::current_dir().unwrap();
            std::env::set_current_dir(new_dir).unwrap();
            Self { original }
        }
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    /// Create a temporary git repository for testing.
    fn create_test_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        temp_dir
    }

    #[test]
    #[serial]
    fn test_worktree_requires_task_id() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let args = WorktreeArgs { task_id: None };
        let result = cmd_worktree(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("task ID is required"));
    }

    #[test]
    #[serial]
    fn test_worktree_task_not_found() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let args = WorktreeArgs {
            task_id: Some("TASK-999".to_string()),
        };
        let result = cmd_worktree(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_worktree_no_worktree_recorded() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let add_args = AddArgs {
            title: "Test task".to_string(),
            priority: "medium".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        cmd_add(add_args).unwrap();

        let args = WorktreeArgs {
            task_id: Some("TASK-001".to_string()),
        };
        let result = cmd_worktree(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("no worktree") || err.to_string().contains("not claimed"));
    }

    #[test]
    #[serial]
    fn test_worktree_rejects_path_traversal() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let args = WorktreeArgs {
            task_id: Some("../TASK-001".to_string()),
        };
        let result = cmd_worktree(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }
}
