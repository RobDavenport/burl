//! Lock acquisition, listing, and clearing operations.

use super::guard::LockGuard;
use super::metadata::LockMetadata;
use super::types::{LockInfo, LockType};
use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

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
