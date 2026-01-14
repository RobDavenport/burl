//! Lock type definitions and information structures.

use super::metadata::LockMetadata;
use std::path::PathBuf;

/// Type of lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    /// Global workflow lock for serializing workflow mutations.
    Workflow,
    /// Per-task lock for serializing task transitions.
    Task,
    /// Global claim lock for serializing "claim next" operations.
    Claim,
}

impl LockType {
    /// Get the filename suffix for this lock type.
    pub fn as_str(&self) -> &'static str {
        match self {
            LockType::Workflow => "workflow",
            LockType::Task => "task",
            LockType::Claim => "claim",
        }
    }
}

/// Information about an active lock.
#[derive(Debug, Clone)]
pub struct LockInfo {
    /// The lock file path.
    pub path: PathBuf,

    /// The lock name (e.g., "workflow", "TASK-001", "claim").
    pub name: String,

    /// The lock type.
    pub lock_type: LockType,

    /// The lock metadata.
    pub metadata: LockMetadata,

    /// Whether the lock is stale.
    pub is_stale: bool,
}

impl std::fmt::Display for LockInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (owner: {}, age: {}, action: {}{})",
            self.name,
            self.metadata.owner,
            self.metadata.age_string(),
            self.metadata.action,
            if self.is_stale { ", STALE" } else { "" }
        )
    }
}
