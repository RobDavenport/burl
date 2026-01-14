//! Git worktree and task branch helpers for burl.
//!
//! This module provides the git operations needed for task branch and worktree
//! management during the claim, submit, approve, and cleanup lifecycle:
//!
//! - Fetching from remote
//! - Determining base_sha (origin/main HEAD at claim time)
//! - Creating/reusing task branches
//! - Creating/attaching task worktrees
//! - Removing worktrees and deleting branches
//!
//! All git failures are mapped to exit code 3 (BurlError::GitError).

mod branch;
mod cleanup;
mod naming;
mod remote;
mod verification;
mod worktree;

// Re-export public API
pub use branch::{branch_exists, create_branch, delete_branch};
pub use cleanup::{cleanup_task_worktree, remove_worktree};
pub use naming::{task_branch_name, task_worktree_path};
pub use remote::{fetch_main, get_base_sha};
pub use verification::{get_current_branch, verify_worktree_branch};
pub use worktree::{
    ExistingWorktree, WorktreeInfo, create_worktree, find_worktree_for_branch, list_worktrees,
    setup_task_worktree,
};
