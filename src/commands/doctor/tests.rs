//! Tests for the doctor command.

use super::*;
use crate::commands::init::cmd_init;
use crate::context::require_initialized_workflow;
use crate::locks::LockMetadata;
use crate::test_support::{DirGuard, create_test_repo};
use chrono::{Duration, Utc};
use serial_test::serial;
use std::path::PathBuf;

#[test]
#[serial]
fn test_doctor_healthy_workflow() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    // Run doctor (read-only mode)
    // Should succeed with no issues (or only minor directory creation issues)
    let ctx = require_initialized_workflow().unwrap();
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    let mut report = DoctorReport::new();

    // Run checks that shouldn't find issues in a fresh workflow
    check_stale_locks(&ctx, &config, &mut report).unwrap();
    check_orphan_locks(&ctx, &config, &mut report).unwrap();
    check_tasks_missing_base_sha(&ctx, &mut report).unwrap();

    // Fresh workflow should have no stale locks, orphan locks, or tasks
    let stale_or_orphan = report
        .issues
        .iter()
        .filter(|i| i.category == "stale_lock" || i.category == "orphan_lock")
        .count();
    assert_eq!(stale_or_orphan, 0);
}

#[test]
#[serial]
fn test_doctor_detects_stale_lock() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    let ctx = require_initialized_workflow().unwrap();
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Create a stale lock manually
    let stale_meta = LockMetadata {
        owner: "test@host".to_string(),
        pid: Some(12345),
        created_at: Utc::now() - Duration::minutes(200), // Beyond default 120 min threshold
        action: "claim".to_string(),
    };

    let lock_path = ctx.task_lock_path("TASK-001");
    std::fs::create_dir_all(&ctx.locks_dir).unwrap();
    std::fs::write(&lock_path, stale_meta.to_json().unwrap()).unwrap();

    // Run doctor
    let mut report = DoctorReport::new();
    check_stale_locks(&ctx, &config, &mut report).unwrap();

    // Should detect the stale lock
    assert!(report.has_issues());
    let stale_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.category == "stale_lock")
        .collect();
    assert_eq!(stale_issues.len(), 1);
    assert!(stale_issues[0].repairable);
}

#[test]
#[serial]
fn test_doctor_repair_clears_stale_lock() {
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    let ctx = require_initialized_workflow().unwrap();
    let _config = Config::load(ctx.config_path()).unwrap_or_default();

    // Create a stale lock manually
    let stale_meta = LockMetadata {
        owner: "test@host".to_string(),
        pid: Some(12345),
        created_at: Utc::now() - Duration::minutes(200),
        action: "claim".to_string(),
    };

    let lock_path = ctx.task_lock_path("TASK-001");
    std::fs::create_dir_all(&ctx.locks_dir).unwrap();
    std::fs::write(&lock_path, stale_meta.to_json().unwrap()).unwrap();

    // Verify lock exists
    assert!(lock_path.exists());

    // Run doctor with repair
    let args = DoctorArgs {
        repair: true,
        force: true,
    };

    // This should succeed and clear the stale lock
    let result = cmd_doctor(args);
    assert!(result.is_ok());

    // Lock should be cleared
    assert!(!lock_path.exists());
}

#[test]
fn test_doctor_requires_force_for_repair() {
    let args = DoctorArgs {
        repair: true,
        force: false,
    };

    // Create a minimal context by running in a temp dir
    let temp_dir = create_test_repo();
    let _guard = DirGuard::new(temp_dir.path());

    // Initialize workflow
    cmd_init().unwrap();

    let result = cmd_doctor(args);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("--force"));
}

#[test]
fn test_issue_creation() {
    let issue = Issue::new(IssueSeverity::Error, "test_category", "test description")
        .with_path("/some/path")
        .with_remediation("run some command")
        .repairable();

    assert_eq!(issue.severity, IssueSeverity::Error);
    assert_eq!(issue.category, "test_category");
    assert_eq!(issue.description, "test description");
    assert_eq!(issue.path, Some("/some/path".to_string()));
    assert_eq!(issue.remediation, Some("run some command".to_string()));
    assert!(issue.repairable);
}

#[test]
fn test_issue_severity_display() {
    assert_eq!(format!("{}", IssueSeverity::Warning), "WARNING");
    assert_eq!(format!("{}", IssueSeverity::Error), "ERROR");
}

#[test]
fn test_doctor_report_has_issues() {
    let mut report = DoctorReport::new();
    assert!(!report.has_issues());

    report
        .issues
        .push(Issue::new(IssueSeverity::Warning, "test", "test issue"));
    assert!(report.has_issues());
}

#[test]
fn test_doctor_report_has_errors() {
    let mut report = DoctorReport::new();
    assert!(!report.has_errors());

    report
        .issues
        .push(Issue::new(IssueSeverity::Warning, "test", "warning issue"));
    assert!(!report.has_errors());

    report
        .issues
        .push(Issue::new(IssueSeverity::Error, "test", "error issue"));
    assert!(report.has_errors());
}

#[test]
fn test_get_bucket_from_path() {
    use super::repairs::get_bucket_from_path;

    let path_ready = PathBuf::from("/some/path/.workflow/READY/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_ready), Some("READY"));

    let path_doing = PathBuf::from("/some/path/.workflow/DOING/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_doing), Some("DOING"));

    let path_qa = PathBuf::from("/some/path/.workflow/QA/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_qa), Some("QA"));

    let path_done = PathBuf::from("/some/path/.workflow/DONE/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_done), Some("DONE"));

    let path_blocked = PathBuf::from("/some/path/.workflow/BLOCKED/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_blocked), Some("BLOCKED"));

    let path_unknown = PathBuf::from("/some/path/OTHER/TASK-001.md");
    assert_eq!(get_bucket_from_path(&path_unknown), None);
}
