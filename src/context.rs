//! Repository and workflow context resolution for burl.
//!
//! This module provides the core "environment resolution" layer that finds
//! the Git repository root from any working directory and resolves the
//! canonical workflow worktree paths.
//!
//! All burl commands must use this module to locate workflow state, ensuring
//! that operations always target the canonical workflow worktree (`.burl/.workflow/`)
//! regardless of where the command is invoked from.

use crate::error::{BurlError, Result};
use crate::git;
use std::env;
use std::path::{Path, PathBuf};

/// Default workflow worktree path relative to repo root.
pub const DEFAULT_WORKFLOW_WORKTREE: &str = ".burl";

/// Default workflow state directory name within the worktree.
pub const DEFAULT_WORKFLOW_STATE_DIR: &str = ".workflow";

/// Default workflow branch name.
pub const DEFAULT_WORKFLOW_BRANCH: &str = "burl";

/// Resolved paths for the burl workflow context.
///
/// This struct provides all the paths needed for burl operations.
/// All paths are absolute.
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    /// Absolute path to the main Git worktree (original clone location).
    pub repo_root: PathBuf,

    /// Absolute path to the workflow worktree (default: `{repo_root}/.burl/`).
    pub workflow_worktree: PathBuf,

    /// Absolute path to the workflow state directory (default: `{repo_root}/.burl/.workflow/`).
    pub workflow_state_dir: PathBuf,

    /// Absolute path to the locks directory (default: `{repo_root}/.burl/.workflow/locks/`).
    pub locks_dir: PathBuf,

    /// Absolute path to the task worktrees directory (default: `{repo_root}/.worktrees/`).
    pub worktrees_dir: PathBuf,
}

impl WorkflowContext {
    /// Resolve the workflow context from the current working directory.
    ///
    /// This function determines all workflow paths by finding the repo root
    /// and applying the fixed V1 defaults for workflow layout.
    ///
    /// # Returns
    ///
    /// * `Ok(WorkflowContext)` - Successfully resolved context
    /// * `Err(BurlError::UserError)` - If not in a git repository (exit code 1)
    pub fn resolve() -> Result<Self> {
        let cwd = env::current_dir().map_err(|e| {
            BurlError::UserError(format!("failed to get current working directory: {}", e))
        })?;

        Self::resolve_from(&cwd)
    }

    /// Resolve the workflow context from a specific directory.
    ///
    /// This is useful for testing or when the working directory is known.
    pub fn resolve_from<P: AsRef<Path>>(cwd: P) -> Result<Self> {
        let cwd = cwd.as_ref();

        // Get the main worktree (original clone location)
        // This handles being invoked from task worktrees or the workflow worktree
        let repo_root = Self::find_main_worktree(cwd)?;

        // Apply V1 fixed defaults for workflow layout
        let workflow_worktree = repo_root.join(DEFAULT_WORKFLOW_WORKTREE);
        let workflow_state_dir = workflow_worktree.join(DEFAULT_WORKFLOW_STATE_DIR);
        let locks_dir = workflow_state_dir.join("locks");
        let worktrees_dir = repo_root.join(".worktrees");

        Ok(Self {
            repo_root,
            workflow_worktree,
            workflow_state_dir,
            locks_dir,
            worktrees_dir,
        })
    }

    /// Find the main worktree from any directory within the repository.
    ///
    /// This handles the case where burl is invoked from:
    /// - The main worktree
    /// - The workflow worktree (`.burl/`)
    /// - A task worktree under `.worktrees/`
    fn find_main_worktree<P: AsRef<Path>>(cwd: P) -> Result<PathBuf> {
        let cwd = cwd.as_ref();

        // First, get the repo root of the current working directory
        let current_toplevel = git::get_repo_root(cwd)?;

        // Try to get the main worktree using git worktree list
        match git::get_main_worktree(cwd) {
            Ok(main_worktree) => {
                // Verify it's a valid path
                if main_worktree.exists() {
                    Ok(main_worktree)
                } else {
                    // Fallback to current toplevel
                    Ok(current_toplevel)
                }
            }
            Err(_) => {
                // git worktree list failed, check if we're in a worktree
                // by looking at the .git file/directory
                Self::infer_main_worktree(&current_toplevel)
            }
        }
    }

    /// Infer the main worktree by checking if we're in a linked worktree.
    ///
    /// Linked worktrees have a `.git` file (not directory) that points to
    /// the actual git directory.
    fn infer_main_worktree(current_toplevel: &Path) -> Result<PathBuf> {
        let git_path = current_toplevel.join(".git");

        if git_path.is_file() {
            // This is a linked worktree - .git is a file containing the gitdir path
            let git_content = std::fs::read_to_string(&git_path)
                .map_err(|e| BurlError::GitError(format!("failed to read .git file: {}", e)))?;

            // Parse "gitdir: /path/to/.git/worktrees/name"
            if let Some(gitdir) = git_content.strip_prefix("gitdir: ") {
                let gitdir = gitdir.trim();
                // The main worktree's .git is typically at ../.git or ../../.git relative to the worktree gitdir
                // The gitdir format is: {main_worktree}/.git/worktrees/{name}
                let gitdir_path = PathBuf::from(gitdir);
                if let Some(worktrees_parent) = gitdir_path.parent()
                    && let Some(git_parent) = worktrees_parent.parent()
                    && let Some(main_worktree) = git_parent.parent()
                    && main_worktree.exists()
                {
                    return Ok(main_worktree.to_path_buf());
                }
            }
        }

        // Not a linked worktree or couldn't parse - this IS the main worktree
        Ok(current_toplevel.to_path_buf())
    }

    /// Check if the workflow worktree exists.
    pub fn workflow_exists(&self) -> bool {
        self.workflow_worktree.exists() && self.workflow_state_dir.exists()
    }

    /// Ensure the workflow is initialized, returning an error if not.
    ///
    /// This should be called by all commands except `init` to provide
    /// a helpful error message guiding users to run `burl init`.
    pub fn ensure_initialized(&self) -> Result<()> {
        if !self.workflow_worktree.exists() {
            return Err(BurlError::UserError(format!(
                "burl workflow not initialized.\n\
                 Expected workflow worktree at: {}\n\n\
                 Run `burl init` to initialize the workflow in this repository.",
                self.workflow_worktree.display()
            )));
        }

        if !self.workflow_state_dir.exists() {
            return Err(BurlError::UserError(format!(
                "burl workflow state directory not found.\n\
                 Expected: {}\n\n\
                 Run `burl init` to initialize the workflow in this repository.",
                self.workflow_state_dir.display()
            )));
        }

        Ok(())
    }

    /// Ensure the workflow worktree is clean before state-changing operations.
    ///
    /// This is a precondition check that should be called before acquiring
    /// the workflow lock for mutations.
    pub fn ensure_workflow_clean(&self) -> Result<()> {
        if self.workflow_exists() {
            git::ensure_workflow_worktree_clean(&self.workflow_worktree)?;
        }
        Ok(())
    }

    /// Get the path to a status bucket directory.
    ///
    /// # Arguments
    ///
    /// * `bucket` - The bucket name (READY, DOING, QA, DONE, BLOCKED)
    pub fn bucket_path(&self, bucket: &str) -> PathBuf {
        self.workflow_state_dir.join(bucket)
    }

    /// Get the path to the config file.
    pub fn config_path(&self) -> PathBuf {
        self.workflow_state_dir.join("config.yaml")
    }

    /// Get the path to the events directory.
    pub fn events_dir(&self) -> PathBuf {
        self.workflow_state_dir.join("events")
    }

    /// Get the path to the main events log file.
    pub fn events_file(&self) -> PathBuf {
        self.events_dir().join("events.ndjson")
    }

    /// Get the path to a task lock file.
    pub fn task_lock_path(&self, task_id: &str) -> PathBuf {
        self.locks_dir.join(format!("{}.lock", task_id))
    }

    /// Get the path to the workflow lock file.
    pub fn workflow_lock_path(&self) -> PathBuf {
        self.locks_dir.join("workflow.lock")
    }

    /// Get the path to the claim lock file.
    pub fn claim_lock_path(&self) -> PathBuf {
        self.locks_dir.join("claim.lock")
    }
}

/// Convenience function to resolve context and ensure workflow is initialized.
///
/// Use this in most commands (except `init`) to get the workflow context
/// with proper error handling for uninitialized workflows.
pub fn require_initialized_workflow() -> Result<WorkflowContext> {
    let ctx = WorkflowContext::resolve()?;
    ctx.ensure_initialized()?;
    Ok(ctx)
}

/// Convenience function to resolve context without requiring initialization.
///
/// Use this in the `init` command where the workflow may not exist yet.
pub fn resolve_context() -> Result<WorkflowContext> {
    WorkflowContext::resolve()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::create_test_repo;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_from_repo_root() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        // Verify repo root is correct
        let expected_root = temp_dir.path().canonicalize().unwrap();
        let actual_root = ctx.repo_root.canonicalize().unwrap();
        assert_eq!(actual_root, expected_root);

        // Verify default paths
        assert!(ctx.workflow_worktree.ends_with(".burl"));
        assert!(ctx.workflow_state_dir.ends_with(".workflow"));
        assert!(ctx.worktrees_dir.ends_with(".worktrees"));
    }

    #[test]
    fn test_resolve_from_subdirectory() {
        let temp_dir = create_test_repo();
        let subdir = temp_dir.path().join("src").join("nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let ctx = WorkflowContext::resolve_from(&subdir).unwrap();

        // Should still find the repo root
        let expected_root = temp_dir.path().canonicalize().unwrap();
        let actual_root = ctx.repo_root.canonicalize().unwrap();
        assert_eq!(actual_root, expected_root);
    }

    #[test]
    fn test_resolve_outside_repo_fails() {
        let temp_dir = TempDir::new().unwrap(); // Not a git repo
        let result = WorkflowContext::resolve_from(temp_dir.path());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BurlError::UserError(_)));
        assert!(err.to_string().contains("not inside a git repository"));
    }

    #[test]
    fn test_workflow_exists_false_by_default() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        assert!(!ctx.workflow_exists());
    }

    #[test]
    fn test_workflow_exists_true_when_created() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        // Create the workflow directories
        std::fs::create_dir_all(&ctx.workflow_state_dir).unwrap();

        assert!(ctx.workflow_exists());
    }

    #[test]
    fn test_ensure_initialized_fails_when_not_initialized() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let result = ctx.ensure_initialized();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("burl init"));
    }

    #[test]
    fn test_ensure_initialized_succeeds_when_initialized() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        // Create the workflow directories
        std::fs::create_dir_all(&ctx.workflow_state_dir).unwrap();

        let result = ctx.ensure_initialized();
        assert!(result.is_ok());
    }

    #[test]
    fn test_bucket_path() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let ready_path = ctx.bucket_path("READY");
        assert!(ready_path.ends_with("READY"));
        assert!(ready_path.to_string_lossy().contains(".workflow"));
    }

    #[test]
    fn test_config_path() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let config_path = ctx.config_path();
        assert!(config_path.ends_with("config.yaml"));
    }

    #[test]
    fn test_lock_paths() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let task_lock = ctx.task_lock_path("TASK-001");
        assert!(task_lock.ends_with("TASK-001.lock"));

        let workflow_lock = ctx.workflow_lock_path();
        assert!(workflow_lock.ends_with("workflow.lock"));

        let claim_lock = ctx.claim_lock_path();
        assert!(claim_lock.ends_with("claim.lock"));
    }

    #[test]
    fn test_events_dir() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let events_dir = ctx.events_dir();
        assert!(events_dir.ends_with("events"));
    }

    #[test]
    fn test_events_file() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        let events_file = ctx.events_file();
        assert!(events_file.ends_with("events.ndjson"));
        assert!(events_file.to_string_lossy().contains("events"));
    }

    #[test]
    fn test_resolve_from_task_worktree() {
        let temp_dir = create_test_repo();
        let main_path = temp_dir.path();

        // Create a branch for the worktree
        Command::new("git")
            .current_dir(main_path)
            .args(["branch", "task-001"])
            .output()
            .expect("failed to create branch");

        // Create a worktree
        let worktree_path = main_path.join(".worktrees").join("task-001");
        std::fs::create_dir_all(worktree_path.parent().unwrap()).unwrap();

        Command::new("git")
            .current_dir(main_path)
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "task-001",
            ])
            .output()
            .expect("failed to create worktree");

        // Resolve context from within the task worktree
        let ctx = WorkflowContext::resolve_from(&worktree_path).unwrap();

        // Should resolve to the main worktree
        let expected_root = main_path.canonicalize().unwrap();
        let actual_root = ctx.repo_root.canonicalize().unwrap();
        assert_eq!(actual_root, expected_root);
    }

    #[test]
    fn test_resolve_from_workflow_worktree() {
        let temp_dir = create_test_repo();
        let main_path = temp_dir.path();

        // Create the burl branch
        Command::new("git")
            .current_dir(main_path)
            .args(["branch", "burl"])
            .output()
            .expect("failed to create burl branch");

        // Create the workflow worktree
        let workflow_path = main_path.join(".burl");
        Command::new("git")
            .current_dir(main_path)
            .args(["worktree", "add", workflow_path.to_str().unwrap(), "burl"])
            .output()
            .expect("failed to create workflow worktree");

        // Resolve context from within the workflow worktree
        let ctx = WorkflowContext::resolve_from(&workflow_path).unwrap();

        // Should resolve to the main worktree
        let expected_root = main_path.canonicalize().unwrap();
        let actual_root = ctx.repo_root.canonicalize().unwrap();
        assert_eq!(actual_root, expected_root);

        // Workflow paths should still point to .burl under the main worktree
        assert!(ctx.workflow_worktree.starts_with(&ctx.repo_root));
    }

    #[test]
    fn test_ensure_workflow_clean_passes_when_clean() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();

        // Create workflow directories (simulating initialized workflow)
        std::fs::create_dir_all(&ctx.workflow_state_dir).unwrap();

        // Initialize git in the workflow worktree to make it a proper worktree
        // For this test, we'll just check that the function works when the workflow doesn't exist
        let result = ctx.ensure_workflow_clean();
        // Should pass because workflow_exists() returns true but the path is the main repo which is clean
        assert!(result.is_ok());
    }
}
