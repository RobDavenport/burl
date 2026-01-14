//! Tests for the locks subsystem.

use super::*;
use crate::config::Config;
use crate::context::WorkflowContext;
use chrono::{Duration, Utc};
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
    assert!(matches!(err, crate::error::BurlError::LockError(_)));
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
    let owner = metadata::get_owner_string();
    assert!(owner.contains('@'));
    assert!(!owner.is_empty());
}
