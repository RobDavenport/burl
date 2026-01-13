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

use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Lock metadata stored in lock files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Owner of the lock (e.g., `user@HOST`).
    pub owner: String,

    /// Process ID of the lock holder (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,

    /// Timestamp when the lock was created (RFC3339).
    pub created_at: DateTime<Utc>,

    /// The action being performed (claim/submit/approve/etc.).
    pub action: String,
}

impl LockMetadata {
    /// Create new lock metadata with the current timestamp.
    pub fn new(action: &str) -> Self {
        Self {
            owner: get_owner_string(),
            pid: Some(std::process::id()),
            created_at: Utc::now(),
            action: action.to_string(),
        }
    }

    /// Parse lock metadata from a file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read lock file '{}': {}",
                path.as_ref().display(),
                e
            ))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            BurlError::UserError(format!(
                "failed to parse lock file '{}': {}",
                path.as_ref().display(),
                e
            ))
        })
    }

    /// Serialize lock metadata to JSON string.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize lock metadata: {}", e)))
    }

    /// Calculate the age of the lock.
    pub fn age(&self) -> Duration {
        Utc::now().signed_duration_since(self.created_at)
    }

    /// Format the age as a human-readable string.
    pub fn age_string(&self) -> String {
        let age = self.age();
        let minutes = age.num_minutes();
        let hours = age.num_hours();
        let days = age.num_days();

        if days > 0 {
            format!("{}d {}h", days, hours % 24)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes % 60)
        } else {
            format!("{}m", minutes)
        }
    }

    /// Check if the lock is stale based on the given threshold in minutes.
    pub fn is_stale(&self, stale_minutes: u32) -> bool {
        self.age().num_minutes() > stale_minutes as i64
    }
}

/// Get the owner string for lock metadata.
fn get_owner_string() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!("{}@{}", user, host)
}

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

/// RAII guard for a lock file.
///
/// When dropped, the lock file is automatically deleted.
/// If deletion fails, a warning is printed but no panic occurs.
#[derive(Debug)]
pub struct LockGuard {
    /// Path to the lock file.
    path: PathBuf,

    /// Whether the lock has been released manually.
    released: bool,
}

impl LockGuard {
    /// Create a new lock guard for the given path.
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            released: false,
        }
    }

    /// Get the path to the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Manually release the lock.
    ///
    /// This is useful when you want to release the lock before the guard
    /// goes out of scope, and want to handle errors explicitly.
    pub fn release(mut self) -> Result<()> {
        self.released = true;
        fs::remove_file(&self.path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to release lock '{}': {}",
                self.path.display(),
                e
            ))
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.released
            && let Err(e) = fs::remove_file(&self.path)
        {
            eprintln!(
                "Warning: failed to release lock '{}': {}",
                self.path.display(),
                e
            );
        }
    }
}

/// Acquire a lock file using create_new semantics.
///
/// This function creates a lock file exclusively - if the file already exists,
/// the operation fails with a `LockError`.
///
/// # Arguments
///
/// * `lock_path` - Path to the lock file
/// * `metadata` - Metadata to write to the lock file
///
/// # Returns
///
/// * `Ok(LockGuard)` - Successfully acquired lock with RAII guard
/// * `Err(BurlError::LockError)` - Lock already exists (exit code 4)
fn acquire_lock(lock_path: &Path, metadata: &LockMetadata) -> Result<LockGuard> {
    // Ensure the locks directory exists
    if let Some(parent) = lock_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create locks directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }

    // Try to create the lock file exclusively
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                // Try to read the existing lock metadata for a helpful error message
                let existing_info = match LockMetadata::from_file(lock_path) {
                    Ok(meta) => format!(
                        "\nLock: {} (created {} ago by {})\nAction: {}",
                        lock_path.display(),
                        meta.age_string(),
                        meta.owner,
                        meta.action
                    ),
                    Err(_) => format!("\nLock: {}", lock_path.display()),
                };
                BurlError::LockError(format!("lock is held by another process{}", existing_info))
            } else {
                BurlError::LockError(format!(
                    "failed to acquire lock '{}': {}",
                    lock_path.display(),
                    e
                ))
            }
        })?;

    // Write the metadata to the lock file
    let json = metadata.to_json()?;
    file.write_all(json.as_bytes()).map_err(|e| {
        // Clean up the lock file on write failure
        let _ = fs::remove_file(lock_path);
        BurlError::LockError(format!("failed to write lock metadata: {}", e))
    })?;

    file.sync_all().map_err(|e| {
        // Clean up the lock file on sync failure
        let _ = fs::remove_file(lock_path);
        BurlError::LockError(format!("failed to sync lock file: {}", e))
    })?;

    Ok(LockGuard::new(lock_path.to_path_buf()))
}

/// Acquire the global workflow lock.
///
/// This lock must be held during the critical section that mutates
/// workflow state and commits the workflow branch.
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `action` - The action being performed (for lock metadata)
///
/// # Returns
///
/// * `Ok(LockGuard)` - Successfully acquired lock
/// * `Err(BurlError::LockError)` - Lock already held (exit code 4)
pub fn acquire_workflow_lock(ctx: &WorkflowContext, action: &str) -> Result<LockGuard> {
    let metadata = LockMetadata::new(action);
    acquire_lock(&ctx.workflow_lock_path(), &metadata)
}

/// Acquire a per-task lock.
///
/// This lock must be held when transitioning a specific task.
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `task_id` - The task ID (e.g., "TASK-001")
/// * `action` - The action being performed (for lock metadata)
///
/// # Returns
///
/// * `Ok(LockGuard)` - Successfully acquired lock
/// * `Err(BurlError::LockError)` - Lock already held (exit code 4)
pub fn acquire_task_lock(ctx: &WorkflowContext, task_id: &str, action: &str) -> Result<LockGuard> {
    let metadata = LockMetadata::new(action);
    acquire_lock(&ctx.task_lock_path(task_id), &metadata)
}

/// Acquire the global claim lock.
///
/// This lock is used when `burl claim` is called without a task ID
/// to serialize selection from READY.
///
/// # Arguments
///
/// * `ctx` - The workflow context
///
/// # Returns
///
/// * `Ok(LockGuard)` - Successfully acquired lock
/// * `Err(BurlError::LockError)` - Lock already held (exit code 4)
pub fn acquire_claim_lock(ctx: &WorkflowContext) -> Result<LockGuard> {
    let metadata = LockMetadata::new("claim");
    acquire_lock(&ctx.claim_lock_path(), &metadata)
}

/// List all active locks in the workflow.
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `config` - The workflow configuration (for stale threshold)
///
/// # Returns
///
/// A vector of `LockInfo` for all active locks.
pub fn list_locks(ctx: &WorkflowContext, config: &Config) -> Result<Vec<LockInfo>> {
    let mut locks = Vec::new();

    // Ensure locks directory exists
    if !ctx.locks_dir.exists() {
        return Ok(locks);
    }

    // Read all lock files
    let entries = fs::read_dir(&ctx.locks_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to read locks directory '{}': {}",
            ctx.locks_dir.display(),
            e
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            BurlError::UserError(format!("failed to read locks directory entry: {}", e))
        })?;

        let path = entry.path();

        // Skip non-lock files
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }

        // Parse the lock metadata
        let metadata = match LockMetadata::from_file(&path) {
            Ok(meta) => meta,
            Err(_) => continue, // Skip invalid lock files
        };

        // Determine lock type and name
        let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let (lock_type, name) = if filename == "workflow" {
            (LockType::Workflow, "workflow".to_string())
        } else if filename == "claim" {
            (LockType::Claim, "claim".to_string())
        } else {
            (LockType::Task, filename.to_string())
        };

        let is_stale = metadata.is_stale(config.lock_stale_minutes);

        locks.push(LockInfo {
            path,
            name,
            lock_type,
            metadata,
            is_stale,
        });
    }

    // Sort by name for consistent output
    locks.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(locks)
}

/// Clear a lock file.
///
/// This removes the lock file from the filesystem. The caller is responsible
/// for verifying that clearing the lock is appropriate (e.g., checking --force).
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `lock_id` - The lock identifier ("workflow", "claim", or a task ID like "TASK-001")
///
/// # Returns
///
/// * `Ok(LockInfo)` - Information about the cleared lock (for audit purposes)
/// * `Err(BurlError::UserError)` - Lock file doesn't exist or invalid lock ID
pub fn clear_lock(ctx: &WorkflowContext, lock_id: &str, config: &Config) -> Result<LockInfo> {
    // Determine the lock path based on the lock ID
    let lock_path = match lock_id {
        "workflow" => ctx.workflow_lock_path(),
        "claim" => ctx.claim_lock_path(),
        task_id => ctx.task_lock_path(task_id),
    };

    // Check if the lock file exists
    if !lock_path.exists() {
        return Err(BurlError::UserError(format!(
            "lock '{}' does not exist at: {}",
            lock_id,
            lock_path.display()
        )));
    }

    // Read the lock metadata before removing
    let metadata = LockMetadata::from_file(&lock_path)?;

    // Determine lock type
    let lock_type = match lock_id {
        "workflow" => LockType::Workflow,
        "claim" => LockType::Claim,
        _ => LockType::Task,
    };

    let is_stale = metadata.is_stale(config.lock_stale_minutes);

    let lock_info = LockInfo {
        path: lock_path.clone(),
        name: lock_id.to_string(),
        lock_type,
        metadata,
        is_stale,
    };

    // Remove the lock file
    fs::remove_file(&lock_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to clear lock '{}': {}",
            lock_path.display(),
            e
        ))
    })?;

    Ok(lock_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a temporary git repository with workflow structure for testing.
    fn create_test_workflow() -> (TempDir, WorkflowContext) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        // Configure git user for commits
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

        // Create initial commit
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

        // Create workflow structure
        let ctx = WorkflowContext::resolve_from(path).unwrap();
        std::fs::create_dir_all(&ctx.locks_dir).unwrap();

        (temp_dir, ctx)
    }

    #[test]
    fn test_lock_metadata_creation() {
        let meta = LockMetadata::new("claim");

        assert!(!meta.owner.is_empty());
        assert!(meta.pid.is_some());
        assert_eq!(meta.action, "claim");
        // created_at should be recent (within last minute)
        assert!(meta.age().num_minutes() < 1);
    }

    #[test]
    fn test_lock_metadata_serialization() {
        let meta = LockMetadata::new("submit");
        let json = meta.to_json().unwrap();

        assert!(json.contains("owner"));
        assert!(json.contains("created_at"));
        assert!(json.contains("submit"));

        // Should be valid JSON that can be parsed back
        let parsed: LockMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.action, "submit");
    }

    #[test]
    fn test_lock_metadata_age_string() {
        let mut meta = LockMetadata::new("test");

        // Just created - should be 0m
        let age_str = meta.age_string();
        assert!(age_str.contains('m'));

        // Simulate an old lock (2 hours ago)
        meta.created_at = Utc::now() - Duration::hours(2);
        let age_str = meta.age_string();
        assert!(age_str.contains('h'));

        // Simulate a very old lock (3 days ago)
        meta.created_at = Utc::now() - Duration::days(3);
        let age_str = meta.age_string();
        assert!(age_str.contains('d'));
    }

    #[test]
    fn test_lock_metadata_is_stale() {
        let mut meta = LockMetadata::new("test");

        // Fresh lock should not be stale
        assert!(!meta.is_stale(120));

        // Old lock should be stale
        meta.created_at = Utc::now() - Duration::minutes(150);
        assert!(meta.is_stale(120));
    }

    #[test]
    fn test_acquire_workflow_lock_success() {
        let (_temp_dir, ctx) = create_test_workflow();

        let guard = acquire_workflow_lock(&ctx, "test_action").unwrap();

        // Lock file should exist
        assert!(ctx.workflow_lock_path().exists());

        // Read and verify metadata
        let meta = LockMetadata::from_file(ctx.workflow_lock_path()).unwrap();
        assert_eq!(meta.action, "test_action");

        // Drop the guard
        drop(guard);

        // Lock file should be removed
        assert!(!ctx.workflow_lock_path().exists());
    }

    #[test]
    fn test_acquire_task_lock_success() {
        let (_temp_dir, ctx) = create_test_workflow();

        let guard = acquire_task_lock(&ctx, "TASK-001", "claim").unwrap();

        // Lock file should exist
        let lock_path = ctx.task_lock_path("TASK-001");
        assert!(lock_path.exists());

        // Read and verify metadata
        let meta = LockMetadata::from_file(&lock_path).unwrap();
        assert_eq!(meta.action, "claim");

        // Drop the guard
        drop(guard);

        // Lock file should be removed
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_acquire_claim_lock_success() {
        let (_temp_dir, ctx) = create_test_workflow();

        let guard = acquire_claim_lock(&ctx).unwrap();

        // Lock file should exist
        assert!(ctx.claim_lock_path().exists());

        // Drop the guard
        drop(guard);

        // Lock file should be removed
        assert!(!ctx.claim_lock_path().exists());
    }

    #[test]
    fn test_acquire_same_lock_twice_fails() {
        let (_temp_dir, ctx) = create_test_workflow();

        // First acquisition should succeed
        let guard1 = acquire_workflow_lock(&ctx, "first").unwrap();

        // Second acquisition should fail with LockError
        let result = acquire_workflow_lock(&ctx, "second");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BurlError::LockError(_)));
        assert!(err.to_string().contains("held by another process"));

        // Drop first guard
        drop(guard1);

        // Now acquisition should succeed
        let guard2 = acquire_workflow_lock(&ctx, "third").unwrap();
        drop(guard2);
    }

    #[test]
    fn test_lock_guard_manual_release() {
        let (_temp_dir, ctx) = create_test_workflow();

        let guard = acquire_workflow_lock(&ctx, "test").unwrap();

        // Manually release
        guard.release().unwrap();

        // Lock file should be removed
        assert!(!ctx.workflow_lock_path().exists());
    }

    #[test]
    fn test_list_locks_empty() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        let locks = list_locks(&ctx, &config).unwrap();
        assert!(locks.is_empty());
    }

    #[test]
    fn test_list_locks_with_locks() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        // Create some locks
        let _workflow_guard = acquire_workflow_lock(&ctx, "workflow_action").unwrap();
        let _task_guard = acquire_task_lock(&ctx, "TASK-001", "claim").unwrap();
        let _claim_guard = acquire_claim_lock(&ctx).unwrap();

        let locks = list_locks(&ctx, &config).unwrap();
        assert_eq!(locks.len(), 3);

        // Check that all locks are listed
        let names: Vec<&str> = locks.iter().map(|l| l.name.as_str()).collect();
        assert!(names.contains(&"workflow"));
        assert!(names.contains(&"TASK-001"));
        assert!(names.contains(&"claim"));
    }

    #[test]
    fn test_list_locks_detects_stale() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        // Manually create a stale lock file
        let stale_meta = LockMetadata {
            owner: "test@host".to_string(),
            pid: Some(12345),
            created_at: Utc::now() - Duration::minutes(200), // Beyond default 120 min threshold
            action: "old_action".to_string(),
        };

        let lock_path = ctx.task_lock_path("TASK-002");
        std::fs::write(&lock_path, stale_meta.to_json().unwrap()).unwrap();

        let locks = list_locks(&ctx, &config).unwrap();
        assert_eq!(locks.len(), 1);

        let lock = &locks[0];
        assert_eq!(lock.name, "TASK-002");
        assert!(lock.is_stale);
    }

    #[test]
    fn test_clear_lock_success() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        // Create a lock (but don't keep the guard to simulate orphan lock)
        let meta = LockMetadata::new("test");
        let lock_path = ctx.workflow_lock_path();
        std::fs::write(&lock_path, meta.to_json().unwrap()).unwrap();

        // Lock file should exist
        assert!(lock_path.exists());

        // Clear the lock
        let cleared = clear_lock(&ctx, "workflow", &config).unwrap();

        // Lock file should be removed
        assert!(!lock_path.exists());

        // Cleared info should have correct metadata
        assert_eq!(cleared.name, "workflow");
        assert_eq!(cleared.lock_type, LockType::Workflow);
    }

    #[test]
    fn test_clear_lock_nonexistent_fails() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        let result = clear_lock(&ctx, "TASK-999", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_clear_task_lock() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        // Create a task lock manually
        let meta = LockMetadata::new("claim");
        let lock_path = ctx.task_lock_path("TASK-001");
        std::fs::write(&lock_path, meta.to_json().unwrap()).unwrap();

        // Clear the lock
        let cleared = clear_lock(&ctx, "TASK-001", &config).unwrap();

        // Lock file should be removed
        assert!(!lock_path.exists());
        assert_eq!(cleared.lock_type, LockType::Task);
    }

    #[test]
    fn test_clear_claim_lock() {
        let (_temp_dir, ctx) = create_test_workflow();
        let config = Config::default();

        // Create a claim lock manually
        let meta = LockMetadata::new("claim");
        let lock_path = ctx.claim_lock_path();
        std::fs::write(&lock_path, meta.to_json().unwrap()).unwrap();

        // Clear the lock
        let cleared = clear_lock(&ctx, "claim", &config).unwrap();

        // Lock file should be removed
        assert!(!lock_path.exists());
        assert_eq!(cleared.lock_type, LockType::Claim);
    }

    #[test]
    fn test_lock_info_display() {
        let (_temp_dir, ctx) = create_test_workflow();
        let _config = Config::default();

        let meta = LockMetadata::new("test_action");
        let lock_info = LockInfo {
            path: ctx.workflow_lock_path(),
            name: "workflow".to_string(),
            lock_type: LockType::Workflow,
            metadata: meta,
            is_stale: false,
        };

        let display = format!("{}", lock_info);
        assert!(display.contains("workflow"));
        assert!(display.contains("test_action"));
        assert!(!display.contains("STALE"));

        // Test stale display
        let stale_info = LockInfo {
            is_stale: true,
            ..lock_info
        };
        let stale_display = format!("{}", stale_info);
        assert!(stale_display.contains("STALE"));
    }

    #[test]
    fn test_get_owner_string() {
        let owner = get_owner_string();
        assert!(owner.contains('@'));
        assert!(!owner.is_empty());
    }
}
