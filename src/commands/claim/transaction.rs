//! Transaction management for claim operation.

use crate::git_worktree::{WorktreeInfo, delete_branch};

/// Information about a claim operation for rollback purposes.
pub struct ClaimTransaction {
    /// Whether the branch was created in this transaction (for rollback).
    pub branch_created: bool,
    /// The branch name.
    pub branch_name: String,
    /// The worktree info.
    pub worktree_info: Option<WorktreeInfo>,
}

impl ClaimTransaction {
    pub fn new() -> Self {
        Self {
            branch_created: false,
            branch_name: String::new(),
            worktree_info: None,
        }
    }

    /// Rollback the transaction by deleting the branch if it was created.
    pub fn rollback(self, repo_root: &std::path::Path) {
        if self.branch_created && !self.branch_name.is_empty() {
            // Try to delete the branch - ignore errors during rollback
            let _ = delete_branch(repo_root, &self.branch_name, true);
        }
    }
}
