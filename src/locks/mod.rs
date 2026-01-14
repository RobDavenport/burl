//! Locking subsystem for burl.
//!
//! This module implements the lock model required for race-safe workflow mutations:
//! - Global workflow lock (`workflow.lock`)
//! - Per-task lock (`TASK-XXX.lock`)
//! - Optional global claim lock (`claim.lock`)
//!
//! # Lock Files
//!
//! Lock files are stored in `.burl/.workflow/locks/` (untracked).
//! They are created using **create_new** semantics (exclusive create) to ensure
//! that only one process can acquire a given lock at a time.
//!
//! # Lock Metadata
//!
//! Each lock file contains JSON metadata:
//! - `owner`: The owner of the lock (e.g., `user@HOST`)
//! - `pid`: The process ID (optional)
//! - `created_at`: RFC3339 timestamp
//! - `action`: The action being performed (claim/submit/approve/etc.)
//!
//! # RAII Guards
//!
//! Locks are managed through RAII guard objects that automatically release
//! the lock when dropped. If deletion fails during drop, a warning is printed
//! but the program does not crash.

mod guard;
mod metadata;
mod operations;
mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use guard::LockGuard;
pub use metadata::LockMetadata;
pub use operations::{
    acquire_claim_lock, acquire_task_lock, acquire_workflow_lock, clear_lock, list_locks,
};
pub use types::{LockInfo, LockType};
