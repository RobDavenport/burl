//! Implementation of the `burl validate` command.
//!
//! This module implements validation checks for tasks in QA without moving them.
//! It runs scope validation, stub validation, and build/test hooks, recording
//! results in the task's QA Report section.
//!
//! # Transaction Steps
//!
//! 1. Acquire per-task lock
//! 2. Verify task is in QA with valid worktree/branch/base_sha
//! 3. Run scope validation
//! 4. Run stub validation
//! 5. Run build/test command (if configured)
//! 6. Acquire workflow.lock for state mutation
//! 7. Write QA Report entry to task file
//! 8. Append validate event and commit
//! 9. Release locks

mod report;

#[cfg(test)]
mod tests;

use crate::cli::ValidateArgs;
use crate::config::Config;
use crate::config::ValidationCommandStep;
use crate::context::require_initialized_workflow;
use crate::diff::{added_lines, changed_files};
use crate::error::{BurlError, Result};
use crate::git_worktree::get_current_branch;
use crate::locks::acquire_task_lock;
use crate::task::TaskFile;
use crate::validate::{ValidationStepResult, ValidationStepStatus, run_command_steps};
use crate::validate::{validate_scope, validate_stubs_with_config};
use crate::workflow::{TaskIndex, validate_task_id};

pub use report::write_qa_report_and_event;

/// Execute the `burl validate` command.
///
/// Validates a task in QA without moving it. Records results in QA Report.
///
/// # Exit Codes
///
/// - 0: All validations passed
/// - 1: User error (task not in QA, missing state, invalid config)
/// - 2: Validation failure (scope/stub/build-test violations)
/// - 3: Git error
/// - 4: Lock contention
pub fn cmd_validate(args: ValidateArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;

    // ========================================================================
    // Phase 1: Task Resolution and Validation
    // ========================================================================

    let task_id = validate_task_id(&args.task_id)?;

    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Use `burl status` to see available tasks.",
            task_id
        ))
    })?;

    // Verify task is in QA bucket
    if task_info.bucket != "QA" {
        return Err(BurlError::UserError(format!(
            "task '{}' is not in QA (currently in {}).\n\n\
             Only tasks in QA can be validated with `burl validate`.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "validate")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 3: Verify task has required git state
    // ========================================================================

    let refs = crate::task_git::require_task_git_refs(
        &ctx,
        &task_id,
        task_file.frontmatter.branch.as_deref(),
        task_file.frontmatter.worktree.as_deref(),
    )?;

    let expected_branch = refs.branch;
    let worktree_path = refs.worktree_path;

    if !worktree_path.exists() {
        return Err(BurlError::UserError(format!(
            "task worktree does not exist at '{}'.\n\n\
             Run `burl doctor` to diagnose and repair this inconsistency.",
            worktree_path.display()
        )));
    }

    let current_branch = get_current_branch(&worktree_path)?;
    if current_branch != expected_branch {
        // Record mismatch in QA report and exit with code 1
        let mismatch_msg = format!(
            "Branch mismatch: worktree is on '{}' but task expects '{}'",
            current_branch, expected_branch
        );

        // We still need to record this, so we do a partial write
        write_qa_report_and_event(
            &ctx,
            &task_info.path,
            &mut task_file,
            &task_id,
            false,
            &mismatch_msg,
            &config,
        )?;

        return Err(BurlError::UserError(format!(
            "task worktree is on branch '{}', but task expects branch '{}'.\n\n\
             Run `burl doctor` to diagnose or re-claim the task.",
            current_branch, expected_branch
        )));
    }

    let base_sha = task_file.frontmatter.base_sha.clone().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded base_sha.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    // ========================================================================
    // Phase 4: Run validations (scope + stubs + build/test)
    // ========================================================================

    let mut validation_results: Vec<ValidationStepResult> = Vec::new();
    let mut all_passed = true;

    // Get changed files and added lines for validation
    let changed = changed_files(&worktree_path, &base_sha)?;
    let added = added_lines(&worktree_path, &base_sha)?;

    // --- Scope validation ---
    let scope_result = validate_scope(&task_file.frontmatter, &changed)?;
    if scope_result.passed {
        validation_results.push(ValidationStepResult::pass("scope"));
    } else {
        all_passed = false;
        let error_msg = scope_result.format_error(&task_id);
        validation_results.push(ValidationStepResult::fail("scope", &error_msg));
    }

    // --- Stub validation ---
    let stub_result = validate_stubs_with_config(&config, &added)?;
    if stub_result.passed {
        validation_results.push(ValidationStepResult::pass("stubs"));
    } else {
        all_passed = false;
        let error_msg = stub_result.format_error();
        validation_results.push(ValidationStepResult::fail("stubs", &error_msg));
    }

    // --- Command validation pipeline ---
    let pipeline_results = run_validation_pipeline(&config, &task_file, &changed, &worktree_path);
    for result in pipeline_results {
        if !result.is_success() {
            all_passed = false;
        }
        validation_results.push(result);
    }

    // ========================================================================
    // Phase 5: Write QA Report and Event
    // ========================================================================

    let summary = format_validation_summary(&validation_results, all_passed);
    write_qa_report_and_event(
        &ctx,
        &task_info.path,
        &mut task_file,
        &task_id,
        all_passed,
        &summary,
        &config,
    )?;

    // ========================================================================
    // Phase 6: Output
    // ========================================================================

    println!("Validated task: {}", task_info.id);
    println!("  Title:  {}", task_file.frontmatter.title);
    println!("  Status: {}", task_info.bucket);
    println!();

    for result in &validation_results {
        let status = match result.status {
            ValidationStepStatus::Pass => "PASS",
            ValidationStepStatus::Fail => "FAIL",
            ValidationStepStatus::Skip => "SKIP",
        };
        println!("  {}: {}", result.name, status);
    }

    println!();
    if all_passed {
        println!("All validations passed.");
        Ok(())
    } else {
        println!("Validation failed. See QA Report in task file for details.");
        Err(BurlError::ValidationError(
            "one or more validation checks failed".to_string(),
        ))
    }
}

/// Format the validation summary for the QA Report.
fn format_validation_summary(results: &[ValidationStepResult], all_passed: bool) -> String {
    use chrono::Utc;

    let now = Utc::now();
    let mut summary = format!(
        "### Validation Run: {}\n\n**Result:** {}\n\n",
        now.format("%Y-%m-%d %H:%M:%S UTC"),
        if all_passed { "PASS" } else { "FAIL" }
    );

    for result in results {
        let status = match result.status {
            ValidationStepStatus::Pass => "PASS",
            ValidationStepStatus::Fail => "FAIL",
            ValidationStepStatus::Skip => "SKIP",
        };
        summary.push_str(&format!("- **{}**: {}\n", result.name, status));

        if let Some(msg) = &result.message {
            // Indent the message
            for line in msg.lines() {
                summary.push_str(&format!("  {}\n", line));
            }
        }
    }

    summary
}

fn run_validation_pipeline(
    config: &Config,
    task_file: &TaskFile,
    changed_files: &[String],
    worktree_path: &std::path::Path,
) -> Vec<ValidationStepResult> {
    let profile_name = task_file
        .frontmatter
        .validation_profile
        .as_deref()
        .or(config.default_validation_profile.as_deref());

    let Some(profile_name) = profile_name else {
        return run_legacy_build_command(config, worktree_path);
    };

    let Some(profile) = config.validation_profiles.get(profile_name) else {
        return vec![ValidationStepResult::fail(
            "validation",
            format!(
                "unknown validation_profile '{}'.\n\
                 Fix: add validation_profiles.{} to config.yaml or unset validation_profile on the task.",
                profile_name, profile_name
            ),
        )];
    };

    if profile.steps.is_empty() {
        return vec![ValidationStepResult::skip(
            "validation",
            format!("validation_profile '{}' has no steps", profile_name),
        )];
    }

    run_command_steps(&profile.steps, changed_files, worktree_path)
}

fn run_legacy_build_command(
    config: &Config,
    worktree_path: &std::path::Path,
) -> Vec<ValidationStepResult> {
    if config.build_command.trim().is_empty() {
        return Vec::new();
    }

    let step = ValidationCommandStep {
        name: "build/test".to_string(),
        command: config.build_command.clone(),
        ..Default::default()
    };

    run_command_steps(std::slice::from_ref(&step), &[], worktree_path)
}
