//! Data types for the clean command.

use std::path::PathBuf;

/// Summary of cleanup candidates.
#[derive(Debug, Default)]
pub struct CleanupPlan {
    /// Worktrees for completed tasks.
    pub completed_worktrees: Vec<CleanupCandidate>,
    /// Orphan worktrees (not referenced by any task).
    pub orphan_worktrees: Vec<CleanupCandidate>,
    /// Orphan directories (in .worktrees/ but not valid git worktrees).
    pub orphan_directories: Vec<PathBuf>,
}

/// A cleanup candidate with metadata.
#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    /// Path to the worktree or directory.
    pub path: PathBuf,
    /// Associated task ID (if any).
    pub task_id: Option<String>,
    /// Branch name (if known).
    pub branch: Option<String>,
}

/// Summary of cleanup results.
#[derive(Debug, Default)]
pub struct CleanupResult {
    /// Number of items successfully removed.
    pub removed_count: usize,
    /// Number of items skipped due to errors.
    pub skipped_count: usize,
    /// Paths that were skipped with reasons.
    pub skipped: Vec<(PathBuf, String)>,
}
