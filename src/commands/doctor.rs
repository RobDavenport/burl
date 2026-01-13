//! Implementation of the `burl doctor` command.
//!
//! Diagnoses workflow health and optionally repairs detected issues.
//!
//! # Read-only mode (default)
//!
//! Reports:
//! - Stale locks (based on config threshold)
//! - Orphan lock files (lock exists but task doesn't)
//! - Tasks in DOING/QA missing `base_sha`
//! - Tasks in DOING/QA with missing worktree directory
//! - Orphan worktrees under `.worktrees/` not referenced by any task
//! - Tasks that reference a branch that does not exist locally
//! - Bucket/metadata mismatches (e.g., READY task with `started_at` set)
//!
//! # Repair mode (`--repair --force`)
//!
//! Safe repairs only:
//! - Clear stale locks
//! - Recreate missing directories (`locks/`, `events/`)
//! - Fix bucket placement based on metadata

use crate::cli::DoctorArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::git_worktree::{branch_exists, list_worktrees};
use crate::locks;
use crate::task::TaskFile;
use crate::workflow::{BUCKETS, TaskIndex};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Severity level for issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Warning: potential problem but not critical.
    Warning,
    /// Error: something is wrong and should be fixed.
    Error,
}

impl std::fmt::Display for IssueSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueSeverity::Warning => write!(f, "WARNING"),
            IssueSeverity::Error => write!(f, "ERROR"),
        }
    }
}

/// A detected issue with a recommended fix.
#[derive(Debug, Clone)]
pub struct Issue {
    /// Severity level.
    pub severity: IssueSeverity,
    /// Category of the issue.
    pub category: String,
    /// Description of the issue.
    pub description: String,
    /// Path or identifier involved.
    pub path: Option<String>,
    /// Recommended remediation command or action.
    pub remediation: Option<String>,
    /// Whether this issue can be auto-repaired.
    pub repairable: bool,
}

impl Issue {
    fn new(severity: IssueSeverity, category: &str, description: &str) -> Self {
        Self {
            severity,
            category: category.to_string(),
            description: description.to_string(),
            path: None,
            remediation: None,
            repairable: false,
        }
    }

    fn with_path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }

    fn with_remediation(mut self, remediation: &str) -> Self {
        self.remediation = Some(remediation.to_string());
        self
    }

    fn repairable(mut self) -> Self {
        self.repairable = true;
        self
    }
}

/// Result of running the doctor check.
pub struct DoctorReport {
    /// List of detected issues.
    pub issues: Vec<Issue>,
    /// List of repairs that were performed (in repair mode).
    pub repairs: Vec<String>,
}

impl DoctorReport {
    fn new() -> Self {
        Self {
            issues: Vec::new(),
            repairs: Vec::new(),
        }
    }

    fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    #[allow(dead_code)]
    fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }
}

/// Execute the `burl doctor` command.
pub fn cmd_doctor(args: DoctorArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Validate repair args
    if args.repair && !args.force {
        return Err(BurlError::UserError(
            "refusing to repair without --force flag.\n\n\
             Repairs may modify workflow state. Please review the issues first with `burl doctor`,\n\
             then run `burl doctor --repair --force` to apply safe repairs."
                .to_string(),
        ));
    }

    let mut report = DoctorReport::new();

    // Run all checks
    check_missing_directories(&ctx, &mut report)?;
    check_stale_locks(&ctx, &config, &mut report)?;
    check_orphan_locks(&ctx, &config, &mut report)?;
    check_tasks_missing_base_sha(&ctx, &mut report)?;
    check_tasks_missing_worktree(&ctx, &mut report)?;
    check_orphan_worktrees(&ctx, &mut report)?;
    check_tasks_missing_branch(&ctx, &mut report)?;
    check_bucket_metadata_mismatches(&ctx, &mut report)?;

    // If repair mode, apply safe repairs
    if args.repair && args.force {
        apply_repairs(&ctx, &config, &mut report)?;
    }

    // Print report
    print_report(&report, args.repair);

    // Exit code: 0 if healthy (no issues or all repaired), 1 if issues remain
    if !args.repair && report.has_issues() {
        // Return error to indicate issues were found (exit code 1)
        return Err(BurlError::UserError(format!(
            "Found {} issue(s). Run `burl doctor --repair --force` to apply safe repairs.",
            report.issues.len()
        )));
    }

    // In repair mode, check if there are still unrepaired issues
    let remaining_issues: Vec<_> = report.issues.iter().filter(|i| !i.repairable).collect();

    if args.repair && !remaining_issues.is_empty() {
        return Err(BurlError::UserError(format!(
            "Repairs applied, but {} issue(s) remain that cannot be auto-repaired.",
            remaining_issues.len()
        )));
    }

    if report.has_issues() && !args.repair {
        println!();
    }

    Ok(())
}

/// Check for missing directories (locks/, events/).
fn check_missing_directories(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    // Check locks directory
    if !ctx.locks_dir.exists() {
        report.issues.push(
            Issue::new(
                IssueSeverity::Warning,
                "missing_directory",
                "Locks directory does not exist",
            )
            .with_path(&ctx.locks_dir.display().to_string())
            .with_remediation("Directory will be created automatically when needed")
            .repairable(),
        );
    }

    // Check events directory
    let events_dir = ctx.events_dir();
    if !events_dir.exists() {
        report.issues.push(
            Issue::new(
                IssueSeverity::Warning,
                "missing_directory",
                "Events directory does not exist",
            )
            .with_path(&events_dir.display().to_string())
            .with_remediation("Directory will be created automatically when needed")
            .repairable(),
        );
    }

    Ok(())
}

/// Check for stale locks.
fn check_stale_locks(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    report: &mut DoctorReport,
) -> Result<()> {
    let locks = locks::list_locks(ctx, config)?;

    for lock in locks.iter().filter(|l| l.is_stale) {
        report.issues.push(
            Issue::new(
                IssueSeverity::Warning,
                "stale_lock",
                &format!(
                    "Lock '{}' is stale (age: {}, threshold: {} min)",
                    lock.name,
                    lock.metadata.age_string(),
                    config.lock_stale_minutes
                ),
            )
            .with_path(&lock.path.display().to_string())
            .with_remediation(&format!("burl lock clear {} --force", lock.name))
            .repairable(),
        );
    }

    Ok(())
}

/// Check for orphan lock files (lock exists but task doesn't).
fn check_orphan_locks(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    report: &mut DoctorReport,
) -> Result<()> {
    let locks = locks::list_locks(ctx, config)?;
    let index = TaskIndex::build(ctx)?;

    for lock in &locks {
        // Skip workflow and claim locks
        if lock.name == "workflow" || lock.name == "claim" {
            continue;
        }

        // Check if the task exists
        if index.find(&lock.name).is_none() {
            report.issues.push(
                Issue::new(
                    IssueSeverity::Warning,
                    "orphan_lock",
                    &format!(
                        "Lock '{}' exists but task does not exist in any bucket",
                        lock.name
                    ),
                )
                .with_path(&lock.path.display().to_string())
                .with_remediation(&format!("burl lock clear {} --force", lock.name))
                .repairable(),
            );
        }
    }

    Ok(())
}

/// Check for tasks in DOING/QA missing base_sha.
fn check_tasks_missing_base_sha(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    let index = TaskIndex::build(ctx)?;

    for task_info in index.all_tasks() {
        if task_info.bucket != "DOING" && task_info.bucket != "QA" {
            continue;
        }

        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue, // Skip unreadable tasks
        };

        if task.frontmatter.base_sha.is_none() {
            report.issues.push(
                Issue::new(
                    IssueSeverity::Error,
                    "missing_base_sha",
                    &format!(
                        "Task {} in {} is missing base_sha",
                        task_info.id, task_info.bucket
                    ),
                )
                .with_path(&task_info.path.display().to_string())
                .with_remediation("Manually set base_sha in the task file or reject and re-claim"),
            );
        }
    }

    Ok(())
}

/// Check for tasks in DOING/QA with missing worktree directory.
fn check_tasks_missing_worktree(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    let index = TaskIndex::build(ctx)?;

    for task_info in index.all_tasks() {
        if task_info.bucket != "DOING" && task_info.bucket != "QA" {
            continue;
        }

        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue, // Skip unreadable tasks
        };

        if let Some(worktree_path) = &task.frontmatter.worktree {
            let full_path = if PathBuf::from(worktree_path).is_absolute() {
                PathBuf::from(worktree_path)
            } else {
                ctx.repo_root.join(worktree_path)
            };

            if !full_path.exists() {
                report.issues.push(
                    Issue::new(
                        IssueSeverity::Error,
                        "missing_worktree",
                        &format!(
                            "Task {} in {} references worktree that does not exist",
                            task_info.id, task_info.bucket
                        ),
                    )
                    .with_path(&full_path.display().to_string())
                    .with_remediation(&format!(
                        "Recreate the worktree or reject and re-claim the task:\n\
                         git worktree add {} {}",
                        full_path.display(),
                        task.frontmatter.branch.as_deref().unwrap_or("<branch>")
                    )),
                );
            }
        }
    }

    Ok(())
}

/// Check for orphan worktrees under .worktrees/ not referenced by any task.
fn check_orphan_worktrees(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    // Get all worktrees from git
    let worktrees = match list_worktrees(&ctx.repo_root) {
        Ok(wts) => wts,
        Err(_) => return Ok(()), // Skip if we can't list worktrees
    };

    // Get all worktree paths referenced by tasks
    let index = TaskIndex::build(ctx)?;
    let mut referenced_paths: HashSet<PathBuf> = HashSet::new();

    for task_info in index.all_tasks() {
        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if let Some(worktree_path) = &task.frontmatter.worktree {
            let full_path = if PathBuf::from(worktree_path).is_absolute() {
                PathBuf::from(worktree_path)
            } else {
                ctx.repo_root.join(worktree_path)
            };

            // Normalize path for comparison
            if let Ok(canonical) = full_path.canonicalize() {
                referenced_paths.insert(canonical);
            } else {
                referenced_paths.insert(full_path);
            }
        }
    }

    // Check each worktree under .worktrees/
    for wt in &worktrees {
        // Only check worktrees under .worktrees/
        let wt_path_str = wt.path.to_string_lossy();
        if !wt_path_str.contains(".worktrees") {
            continue;
        }

        // Normalize path for comparison
        let canonical = wt.path.canonicalize().unwrap_or(wt.path.clone());

        if !referenced_paths.contains(&canonical) && !referenced_paths.contains(&wt.path) {
            report.issues.push(
                Issue::new(
                    IssueSeverity::Warning,
                    "orphan_worktree",
                    &format!(
                        "Worktree at '{}' is not referenced by any task",
                        wt.path.display()
                    ),
                )
                .with_path(&wt.path.display().to_string())
                .with_remediation(&format!(
                    "Remove the orphan worktree if no longer needed:\n\
                     git worktree remove {}",
                    wt.path.display()
                )),
            );
        }
    }

    Ok(())
}

/// Check for tasks that reference a branch that does not exist locally.
fn check_tasks_missing_branch(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    let index = TaskIndex::build(ctx)?;

    for task_info in index.all_tasks() {
        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if let Some(branch) = &task.frontmatter.branch {
            let exists = match branch_exists(&ctx.repo_root, branch) {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !exists {
                report.issues.push(
                    Issue::new(
                        IssueSeverity::Warning,
                        "missing_branch",
                        &format!(
                            "Task {} references branch '{}' which does not exist locally",
                            task_info.id, branch
                        ),
                    )
                    .with_path(&task_info.path.display().to_string())
                    .with_remediation(&format!(
                        "Fetch the branch from remote or clear the branch reference:\n\
                         git fetch origin {}:{}",
                        branch, branch
                    )),
                );
            }
        }
    }

    Ok(())
}

/// Check for bucket/metadata mismatches.
fn check_bucket_metadata_mismatches(
    ctx: &crate::context::WorkflowContext,
    report: &mut DoctorReport,
) -> Result<()> {
    let index = TaskIndex::build(ctx)?;

    for task_info in index.all_tasks() {
        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let fm = &task.frontmatter;

        match task_info.bucket.as_str() {
            "READY" => {
                // READY task should not have started_at, branch, worktree, or base_sha
                if fm.started_at.is_some()
                    || fm.branch.is_some()
                    || fm.worktree.is_some()
                    || fm.base_sha.is_some()
                {
                    report.issues.push(
                        Issue::new(
                            IssueSeverity::Warning,
                            "bucket_mismatch",
                            &format!(
                                "Task {} in READY has work-in-progress metadata (started_at/branch/worktree/base_sha)",
                                task_info.id
                            ),
                        )
                        .with_path(&task_info.path.display().to_string())
                        .with_remediation("Task should be moved to DOING if work has started")
                        .repairable(),
                    );
                }

                // READY task should not have submitted_at
                if fm.submitted_at.is_some() {
                    report.issues.push(
                        Issue::new(
                            IssueSeverity::Warning,
                            "bucket_mismatch",
                            &format!("Task {} in READY has submitted_at set", task_info.id),
                        )
                        .with_path(&task_info.path.display().to_string())
                        .with_remediation("Task should be moved to QA if already submitted")
                        .repairable(),
                    );
                }
            }
            "DOING" => {
                // DOING task should not have submitted_at
                if fm.submitted_at.is_some() {
                    report.issues.push(
                        Issue::new(
                            IssueSeverity::Warning,
                            "bucket_mismatch",
                            &format!("Task {} in DOING has submitted_at set", task_info.id),
                        )
                        .with_path(&task_info.path.display().to_string())
                        .with_remediation("Task should be moved to QA if already submitted")
                        .repairable(),
                    );
                }

                // DOING task should not have completed_at
                if fm.completed_at.is_some() {
                    report.issues.push(
                        Issue::new(
                            IssueSeverity::Warning,
                            "bucket_mismatch",
                            &format!("Task {} in DOING has completed_at set", task_info.id),
                        )
                        .with_path(&task_info.path.display().to_string())
                        .with_remediation("Task should be moved to DONE if already completed"),
                    );
                }
            }
            "QA" => {
                // QA task should not have completed_at
                if fm.completed_at.is_some() {
                    report.issues.push(
                        Issue::new(
                            IssueSeverity::Warning,
                            "bucket_mismatch",
                            &format!("Task {} in QA has completed_at set", task_info.id),
                        )
                        .with_path(&task_info.path.display().to_string())
                        .with_remediation("Task should be moved to DONE if already completed")
                        .repairable(),
                    );
                }
            }
            "DONE" => {
                // DONE is the terminal state, no further checks needed
            }
            "BLOCKED" => {
                // BLOCKED can have various states, no specific checks
            }
            _ => {}
        }
    }

    Ok(())
}

/// Apply safe repairs based on detected issues.
fn apply_repairs(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    report: &mut DoctorReport,
) -> Result<()> {
    // Collect repairable issues
    let repairable: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.repairable)
        .cloned()
        .collect();

    if repairable.is_empty() {
        return Ok(());
    }

    // Acquire workflow lock for repairs that modify state
    let _workflow_lock = locks::acquire_workflow_lock(ctx, "doctor --repair")?;

    for issue in &repairable {
        match issue.category.as_str() {
            "missing_directory" => {
                if let Some(path) = &issue.path {
                    let path = PathBuf::from(path);
                    if !path.exists() {
                        fs::create_dir_all(&path).map_err(|e| {
                            BurlError::UserError(format!(
                                "failed to create directory '{}': {}",
                                path.display(),
                                e
                            ))
                        })?;
                        report
                            .repairs
                            .push(format!("Created directory: {}", path.display()));
                    }
                }
            }
            "stale_lock" | "orphan_lock" => {
                // Extract lock name from the issue
                if let Some(path) = &issue.path {
                    let path = PathBuf::from(path);
                    if path.exists() {
                        // Extract lock ID from filename
                        if let Some(filename) = path.file_stem() {
                            let lock_id = filename.to_string_lossy().to_string();
                            match locks::clear_lock(ctx, &lock_id, config) {
                                Ok(cleared) => {
                                    report.repairs.push(format!(
                                        "Cleared {} lock: {}",
                                        if cleared.is_stale { "stale" } else { "orphan" },
                                        cleared.name
                                    ));
                                }
                                Err(e) => {
                                    eprintln!("Warning: failed to clear lock '{}': {}", lock_id, e);
                                }
                            }
                        }
                    }
                }
            }
            "bucket_mismatch" => {
                // Handle bucket fixes
                if let Some(path) = &issue.path {
                    let path = PathBuf::from(path);
                    if let Ok(task) = TaskFile::load(&path) {
                        let fm = &task.frontmatter;
                        let current_bucket = get_bucket_from_path(&path);

                        // Determine the correct bucket based on metadata
                        let target_bucket = if fm.completed_at.is_some() {
                            Some("DONE")
                        } else if fm.submitted_at.is_some() {
                            Some("QA")
                        } else if fm.started_at.is_some()
                            || fm.branch.is_some()
                            || fm.worktree.is_some()
                            || fm.base_sha.is_some()
                        {
                            Some("DOING")
                        } else {
                            None
                        };

                        if let (Some(current), Some(target)) = (current_bucket, target_bucket)
                            && current != target
                        {
                            // Move the task to the correct bucket
                            let filename = path.file_name().unwrap().to_string_lossy();
                            let new_path = ctx.bucket_path(target).join(filename.as_ref());

                            if let Err(e) = crate::fs::move_file(&path, &new_path) {
                                eprintln!(
                                    "Warning: failed to move task '{}' from {} to {}: {}",
                                    fm.id, current, target, e
                                );
                            } else {
                                report.repairs.push(format!(
                                    "Moved task {} from {} to {}",
                                    fm.id, current, target
                                ));
                            }
                        }
                    }
                }
            }
            _ => {
                // Other issues are not auto-repairable
            }
        }
    }

    // If any repairs were made and auto-commit is enabled, commit the changes
    if !report.repairs.is_empty() {
        // Log the doctor repair event
        let event = Event::new(EventAction::Clean) // Using Clean as closest action type
            .with_details(json!({
                "action": "doctor_repair",
                "repairs": report.repairs.len(),
                "issues_found": report.issues.len()
            }));

        if let Err(e) = append_event(ctx, &event) {
            eprintln!("Warning: failed to log doctor repair event: {}", e);
        }

        // Commit if auto-commit is enabled
        if config.workflow_auto_commit
            && let Err(e) = commit_repairs(ctx, &report.repairs)
        {
            eprintln!("Warning: failed to commit repairs: {}", e);
        }

        // Push if auto-push is enabled
        if config.workflow_auto_push
            && let Err(e) = push_workflow_branch(ctx, config)
        {
            eprintln!("Warning: failed to push workflow branch: {}", e);
        }
    }

    Ok(())
}

/// Get the bucket name from a task file path.
fn get_bucket_from_path(path: &Path) -> Option<&'static str> {
    let path_str = path.to_string_lossy();
    for bucket in BUCKETS {
        if path_str.contains(&format!("/{}/", bucket))
            || path_str.contains(&format!("\\{}\\", bucket))
        {
            return Some(bucket);
        }
    }
    None
}

/// Commit the repairs to the workflow branch.
fn commit_repairs(ctx: &crate::context::WorkflowContext, repairs: &[String]) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage repair changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let repair_summary = repairs.join("\n  - ");
    let commit_msg = format!(
        "burl doctor: apply {} repair(s)\n\n  - {}",
        repairs.len(),
        repair_summary
    );

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit repairs: {}", e)))?;

    Ok(())
}

/// Push the workflow branch to the remote.
fn push_workflow_branch(ctx: &crate::context::WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}

/// Print the doctor report.
fn print_report(report: &DoctorReport, repair_mode: bool) {
    if !report.has_issues() && report.repairs.is_empty() {
        println!("Workflow is healthy. No issues detected.");
        return;
    }

    // Print issues
    if !report.issues.is_empty() {
        println!("Issues detected ({}):", report.issues.len());
        println!();

        for (i, issue) in report.issues.iter().enumerate() {
            println!(
                "  {}. [{}] {} - {}",
                i + 1,
                issue.severity,
                issue.category,
                issue.description
            );

            if let Some(path) = &issue.path {
                println!("     Path: {}", path);
            }

            if let Some(remediation) = &issue.remediation {
                println!(
                    "     Fix:  {}",
                    remediation.lines().next().unwrap_or(remediation)
                );
                for line in remediation.lines().skip(1) {
                    println!("           {}", line);
                }
            }

            if issue.repairable && !repair_mode {
                println!("     (auto-repairable with --repair --force)");
            }

            println!();
        }
    }

    // Print repairs
    if !report.repairs.is_empty() {
        println!("Repairs applied ({}):", report.repairs.len());
        println!();

        for repair in &report.repairs {
            println!("  - {}", repair);
        }

        println!();
    }

    // Print summary
    let error_count = report
        .issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .count();
    let warning_count = report
        .issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Warning)
        .count();
    let repairable_count = report.issues.iter().filter(|i| i.repairable).count();

    if repair_mode {
        let remaining = report.issues.iter().filter(|i| !i.repairable).count();
        if remaining > 0 {
            println!(
                "Summary: {} issue(s) remain that cannot be auto-repaired ({} errors, {} warnings).",
                remaining, error_count, warning_count
            );
        } else if !report.repairs.is_empty() {
            println!("All repairable issues have been fixed.");
        }
    } else {
        println!(
            "Summary: {} errors, {} warnings, {} auto-repairable.",
            error_count, warning_count, repairable_count
        );

        if repairable_count > 0 {
            println!();
            println!("Run `burl doctor --repair --force` to apply safe repairs.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init::cmd_init;
    use crate::locks::LockMetadata;
    use crate::test_support::{DirGuard, create_test_repo};
    use chrono::{Duration, Utc};
    use serial_test::serial;

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
}
