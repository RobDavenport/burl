//! Health check functions for the doctor command.

use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::Result;
use crate::git_worktree::{branch_exists, list_worktrees};
use crate::locks;
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use std::collections::HashSet;
use std::path::PathBuf;

use super::{DoctorReport, Issue, IssueSeverity};

/// Check for missing directories (locks/, events/).
pub fn check_missing_directories(ctx: &WorkflowContext, report: &mut DoctorReport) -> Result<()> {
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
pub fn check_stale_locks(
    ctx: &WorkflowContext,
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
pub fn check_orphan_locks(
    ctx: &WorkflowContext,
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
pub fn check_tasks_missing_base_sha(
    ctx: &WorkflowContext,
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
pub fn check_tasks_missing_worktree(
    ctx: &WorkflowContext,
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
pub fn check_orphan_worktrees(ctx: &WorkflowContext, report: &mut DoctorReport) -> Result<()> {
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
pub fn check_tasks_missing_branch(ctx: &WorkflowContext, report: &mut DoctorReport) -> Result<()> {
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
pub fn check_bucket_metadata_mismatches(
    ctx: &WorkflowContext,
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
