//! Repair functions for the doctor command.

use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::locks;
use crate::task::TaskFile;
use crate::workflow::BUCKETS;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use super::DoctorReport;

/// Apply safe repairs based on detected issues.
pub fn apply_repairs(
    ctx: &WorkflowContext,
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
pub fn get_bucket_from_path(path: &Path) -> Option<&'static str> {
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
fn commit_repairs(ctx: &WorkflowContext, repairs: &[String]) -> Result<()> {
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
fn push_workflow_branch(ctx: &WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.workflow_worktree,
        &["push", &config.remote, &config.workflow_branch],
    )
    .map_err(|e| BurlError::GitError(format!("failed to push workflow branch: {}", e)))?;

    Ok(())
}
