//! Implementation of the `burl status` command.
//!
//! Displays workflow status including task counts per bucket and highlights
//! for locked, stalled, or over-attempt tasks.

use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::Result;
use crate::locks;
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, BUCKETS};
use chrono::{Duration, Utc};

/// Stall threshold in hours for DOING tasks.
const DOING_STALL_HOURS: i64 = 24;

/// Stall threshold in hours for QA tasks.
const QA_STALL_HOURS: i64 = 24;

/// Execute the `burl status` command.
///
/// Displays:
/// - Task counts per bucket
/// - Locked tasks
/// - Stale locks
/// - Tasks with high qa_attempts
/// - Stalled tasks (old started_at or submitted_at)
pub fn cmd_status() -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;
    let bucket_counts = index.bucket_counts();

    // Get active locks
    let active_locks = locks::list_locks(&ctx, &config)?;

    // Print header
    println!("Workflow Status");
    println!("===============");
    println!();

    // Print bucket counts
    println!("Buckets:");
    let total: usize = bucket_counts.values().sum();
    for bucket in BUCKETS {
        let count = bucket_counts.get(*bucket).copied().unwrap_or(0);
        println!("  {:8} {:>3}", bucket, count);
    }
    println!("  --------");
    println!("  {:8} {:>3}", "Total", total);
    println!();

    // Collect issues to highlight
    let mut issues: Vec<String> = Vec::new();

    // Check for locked tasks
    let task_locks: Vec<_> = active_locks
        .iter()
        .filter(|l| l.lock_type == locks::LockType::Task)
        .collect();

    if !task_locks.is_empty() {
        issues.push(format!("{} task(s) currently locked:", task_locks.len()));
        for lock in &task_locks {
            let stale_marker = if lock.is_stale { " [STALE]" } else { "" };
            issues.push(format!(
                "  - {} (by {}, {} ago, action: {}){}",
                lock.name,
                lock.metadata.owner,
                lock.metadata.age_string(),
                lock.metadata.action,
                stale_marker
            ));
        }
    }

    // Check for stale locks
    let stale_locks: Vec<_> = active_locks.iter().filter(|l| l.is_stale).collect();
    if !stale_locks.is_empty() && task_locks.is_empty() {
        // Only show this if we didn't already show task locks with stale markers
        issues.push(format!(
            "{} stale lock(s) detected (older than {} minutes)",
            stale_locks.len(),
            config.lock_stale_minutes
        ));
    }

    // Check for tasks with high qa_attempts
    let near_max_attempts: Vec<_> = index
        .all_tasks()
        .filter(|t| t.bucket == "QA")
        .filter_map(|t| {
            if let Ok(task) = TaskFile::load(&t.path)
                && task.frontmatter.qa_attempts >= config.qa_max_attempts.saturating_sub(1)
            {
                return Some((t.id.clone(), task.frontmatter.qa_attempts));
            }
            None
        })
        .collect();

    if !near_max_attempts.is_empty() {
        issues.push(format!(
            "{} task(s) in QA near max attempts ({}):",
            near_max_attempts.len(),
            config.qa_max_attempts
        ));
        for (id, attempts) in &near_max_attempts {
            issues.push(format!("  - {} ({}/{} attempts)", id, attempts, config.qa_max_attempts));
        }
    }

    // Check for stalled DOING tasks
    let now = Utc::now();
    let stall_threshold_doing = Duration::hours(DOING_STALL_HOURS);
    let stalled_doing: Vec<_> = index
        .tasks_in_bucket("DOING")
        .iter()
        .filter_map(|t| {
            if let Ok(task) = TaskFile::load(&t.path)
                && let Some(started_at) = task.frontmatter.started_at
                && now.signed_duration_since(started_at) > stall_threshold_doing
            {
                return Some((t.id.clone(), started_at));
            }
            None
        })
        .collect();

    if !stalled_doing.is_empty() {
        issues.push(format!(
            "{} task(s) in DOING stalled (started > {}h ago):",
            stalled_doing.len(),
            DOING_STALL_HOURS
        ));
        for (id, started) in &stalled_doing {
            let age = now.signed_duration_since(*started);
            issues.push(format!("  - {} (started {}h ago)", id, age.num_hours()));
        }
    }

    // Check for stalled QA tasks
    let stall_threshold_qa = Duration::hours(QA_STALL_HOURS);
    let stalled_qa: Vec<_> = index
        .tasks_in_bucket("QA")
        .iter()
        .filter_map(|t| {
            if let Ok(task) = TaskFile::load(&t.path)
                && let Some(submitted_at) = task.frontmatter.submitted_at
                && now.signed_duration_since(submitted_at) > stall_threshold_qa
            {
                return Some((t.id.clone(), submitted_at));
            }
            None
        })
        .collect();

    if !stalled_qa.is_empty() {
        issues.push(format!(
            "{} task(s) in QA stalled (submitted > {}h ago):",
            stalled_qa.len(),
            QA_STALL_HOURS
        ));
        for (id, submitted) in &stalled_qa {
            let age = now.signed_duration_since(*submitted);
            issues.push(format!("  - {} (submitted {}h ago)", id, age.num_hours()));
        }
    }

    // Print issues if any
    if !issues.is_empty() {
        println!("Highlights:");
        for issue in &issues {
            println!("  {}", issue);
        }
        println!();
    }

    // Print helpful next steps if there are tasks
    if total > 0 {
        let ready_count = bucket_counts.get("READY").copied().unwrap_or(0);
        let doing_count = bucket_counts.get("DOING").copied().unwrap_or(0);
        let qa_count = bucket_counts.get("QA").copied().unwrap_or(0);

        if ready_count > 0 || doing_count > 0 || qa_count > 0 {
            println!("Commands:");
            if ready_count > 0 {
                println!("  burl claim          - Claim the next available task");
            }
            if doing_count > 0 {
                println!("  burl submit TASK-ID - Submit a task for QA");
            }
            if qa_count > 0 {
                println!("  burl validate TASK-ID - Run validation on a QA task");
                println!("  burl approve TASK-ID  - Approve and merge a task");
            }
        }
    } else {
        println!("No tasks in the workflow. Run `burl add \"title\"` to create a task.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::AddArgs;
    use crate::commands::add::cmd_add;
    use crate::commands::init::cmd_init;
    use serial_test::serial;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// RAII guard for changing current directory - restores on drop.
    struct DirGuard {
        original: PathBuf,
    }

    impl DirGuard {
        fn new(new_dir: &std::path::Path) -> Self {
            let original = std::env::current_dir().unwrap();
            std::env::set_current_dir(new_dir).unwrap();
            Self { original }
        }
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    /// Create a temporary git repository for testing.
    fn create_test_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

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

        temp_dir
    }

    #[test]
    #[serial]
    fn test_status_empty_workflow() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let result = cmd_status();
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_status_with_tasks() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        for i in 1..=3 {
            let args = AddArgs {
                title: format!("Task {}", i),
                priority: "medium".to_string(),
                affects: vec![],
                affects_globs: vec![],
                must_not_touch: vec![],
                depends_on: vec![],
                tags: vec![],
            };
            cmd_add(args).unwrap();
        }

        let result = cmd_status();
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_status_requires_initialized_workflow() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Don't initialize - status should fail
        let result = cmd_status();
        assert!(result.is_err());
    }
}
