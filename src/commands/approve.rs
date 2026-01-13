//! Implementation of the `burl approve` command.
//!
//! This module implements the QA -> DONE transition with rebase, validation, and merge:
//! - Fetch and rebase onto origin/main
//! - Run validation against rebased base (origin/main..HEAD)
//! - Fast-forward merge into local main
//! - Cleanup worktree and branch
//!
//! # Transaction Steps (rebase_ff_only strategy)
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Verify task is in QA with valid worktree/branch
//! 3. Fetch origin/main
//! 4. Rebase task branch onto origin/main (conflict -> reject, move QA -> READY)
//! 5. Run validation against rebased base (origin/main..HEAD)
//! 6. Merge into local main using --ff-only (fail -> reject)
//! 7. Optionally push main if `push_main_on_approve: true`
//! 8. Cleanup worktree and branch (best-effort)
//! 9. Acquire `workflow.lock` for workflow-state mutation
//! 10. Set completed_at, move QA -> DONE
//! 11. Append approve event and commit workflow branch
//! 12. Release locks

use crate::cli::ApproveArgs;
use crate::config::{Config, MergeStrategy};
use crate::context::require_initialized_workflow;
use crate::diff::{added_lines, changed_files};
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::git_worktree::{cleanup_task_worktree, get_current_branch};
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::validate::{validate_scope, validate_stubs_with_config};
use crate::workflow::{TaskIndex, validate_task_id};
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

/// Maximum number of lines to include in QA Report summary.
const QA_REPORT_MAX_LINES: usize = 50;

/// Maximum total characters for QA Report summary.
const QA_REPORT_MAX_CHARS: usize = 4096;

/// Result of a single validation step.
#[derive(Debug, Clone)]
struct ValidationStepResult {
    /// Name of the validation step.
    name: String,
    /// Whether it passed.
    passed: bool,
    /// Error message if failed.
    message: Option<String>,
}

impl ValidationStepResult {
    fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: None,
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: Some(message.into()),
        }
    }
}

/// Result of running the build/test command.
#[derive(Debug)]
struct BuildTestResult {
    /// Whether the command succeeded (exit 0).
    passed: bool,
    /// Exit code of the command.
    exit_code: i32,
    /// Stdout from the command.
    stdout: String,
    /// Stderr from the command.
    stderr: String,
}

/// Execute the `burl approve` command.
///
/// Approves a task in QA by rebasing, validating, merging to main, and moving to DONE.
///
/// # Exit Codes
///
/// - 0: Success
/// - 1: User error (task not in QA, missing state, invalid config)
/// - 2: Validation failure (scope/stub/build-test violations)
/// - 3: Git error (rebase conflict, non-FF merge)
/// - 4: Lock contention
pub fn cmd_approve(args: ApproveArgs) -> Result<()> {
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
             Only tasks in QA can be approved.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "approve")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 3: Verify task has required git state
    // ========================================================================

    let worktree_path = task_file.frontmatter.worktree.as_ref().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded worktree.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    let worktree_path = if PathBuf::from(worktree_path).is_absolute() {
        PathBuf::from(worktree_path)
    } else {
        ctx.repo_root.join(worktree_path)
    };

    if !worktree_path.exists() {
        return Err(BurlError::UserError(format!(
            "task worktree does not exist at '{}'.\n\n\
             Run `burl doctor` to diagnose and repair this inconsistency.",
            worktree_path.display()
        )));
    }

    let expected_branch = task_file.frontmatter.branch.clone().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded branch.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    let current_branch = get_current_branch(&worktree_path)?;
    if current_branch != expected_branch {
        return Err(BurlError::UserError(format!(
            "task worktree is on branch '{}', but task expects branch '{}'.\n\n\
             Remediation: checkout the correct branch with:\n\
             cd {} && git checkout {}",
            current_branch,
            expected_branch,
            worktree_path.display(),
            expected_branch
        )));
    }

    // ========================================================================
    // Phase 4: Strategy-based git operations
    // ========================================================================

    match config.merge_strategy {
        MergeStrategy::RebaseFfOnly => approve_rebase_ff_only(
            &ctx,
            &config,
            &task_info.id,
            &task_info.path,
            &mut task_file,
            &worktree_path,
            &expected_branch,
        ),
        MergeStrategy::FfOnly => approve_ff_only(
            &ctx,
            &config,
            &task_info.id,
            &task_info.path,
            &mut task_file,
            &worktree_path,
            &expected_branch,
        ),
        MergeStrategy::Manual => Err(BurlError::UserError(
            "merge_strategy 'manual' is not implemented in V1.\n\n\
             Use 'rebase_ff_only' (default) or 'ff_only' instead, or perform the merge manually."
                .to_string(),
        )),
    }
}

/// Approve using rebase_ff_only strategy (default).
fn approve_rebase_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    worktree_path: &PathBuf,
    branch: &str,
) -> Result<()> {
    let remote_main = format!("{}/{}", config.remote, config.main_branch);

    // Step 1: Fetch origin/main
    println!("Fetching {}/{}...", config.remote, config.main_branch);
    run_git(
        &ctx.repo_root,
        &["fetch", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to fetch {}/{}: {}",
            config.remote, config.main_branch, e
        ))
    })?;

    // Step 2: Rebase task branch onto origin/main in worktree
    println!("Rebasing {} onto {}...", branch, remote_main);
    let rebase_result = run_git(worktree_path, &["rebase", &remote_main]);

    if let Err(e) = rebase_result {
        // Abort the rebase to leave the worktree in a clean state
        let _ = run_git(worktree_path, &["rebase", "--abort"]);

        // Reject the task
        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("rebase conflict: {}", e),
        );
    }

    // Step 3: Run validation against rebased base (origin/main..HEAD)
    println!("Running validation...");
    let validation_result = run_validation(ctx, config, task_file, worktree_path, &remote_main)?;

    if !validation_result.all_passed {
        // Append validation report before rejecting
        let summary = format_validation_summary(&validation_result.results, false);
        task_file.append_to_qa_report(&summary);

        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            "validation failed after rebase",
        );
    }

    // Step 4: Merge into local main using --ff-only
    println!("Merging {} into local main...", branch);
    merge_ff_only(ctx, config, task_id, task_path, task_file, branch)?;

    // Step 5: Optional push
    if config.push_main_on_approve {
        println!("Pushing main to {}...", config.remote);
        push_main(ctx, config)?;
    }

    // Step 6: Cleanup worktree and branch (best-effort)
    println!("Cleaning up worktree and branch...");
    let cleanup_result = cleanup_task_worktree(
        &ctx.repo_root,
        branch,
        Some(worktree_path.as_path()),
        true, // Force removal since changes are now merged
    );

    let cleanup_failed = cleanup_result.is_err();
    if let Err(e) = &cleanup_result {
        eprintln!("Warning: cleanup failed (will proceed anyway): {}", e);
    }

    // Step 7: Workflow state mutation
    complete_approval(ctx, config, task_id, task_path, task_file, cleanup_failed)?;

    println!();
    println!("Approved task: {}", task_id);
    println!("  Title:     {}", task_file.frontmatter.title);
    println!("  From:      QA");
    println!("  To:        DONE");
    println!("  Branch:    {} (merged to {})", branch, config.main_branch);
    if cleanup_failed {
        println!("  Cleanup:   Failed (run `burl clean` to remove leftovers)");
    } else {
        println!("  Cleanup:   Complete");
    }
    if config.push_main_on_approve {
        println!(
            "  Pushed:    {} -> {}/{}",
            config.main_branch, config.remote, config.main_branch
        );
    }

    Ok(())
}

/// Approve using ff_only strategy (skip rebase).
fn approve_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    worktree_path: &PathBuf,
    branch: &str,
) -> Result<()> {
    let remote_main = format!("{}/{}", config.remote, config.main_branch);

    // Step 1: Fetch origin/main
    println!("Fetching {}/{}...", config.remote, config.main_branch);
    run_git(
        &ctx.repo_root,
        &["fetch", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to fetch {}/{}: {}",
            config.remote, config.main_branch, e
        ))
    })?;

    // Step 2: Verify task branch is descendant of origin/main
    println!("Verifying branch is up-to-date with {}...", remote_main);
    let is_ancestor = run_git(
        worktree_path,
        &["merge-base", "--is-ancestor", &remote_main, "HEAD"],
    );

    if is_ancestor.is_err() {
        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("branch behind {}; rebase required", remote_main),
        );
    }

    // Step 3: Run validation against origin/main..HEAD
    println!("Running validation...");
    let validation_result = run_validation(ctx, config, task_file, worktree_path, &remote_main)?;

    if !validation_result.all_passed {
        let summary = format_validation_summary(&validation_result.results, false);
        task_file.append_to_qa_report(&summary);

        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            "validation failed",
        );
    }

    // Step 4: Optionally fast-forward local main to origin/main first
    // This is recommended to ensure we have the latest main
    let _ = run_git(
        &ctx.repo_root,
        &[
            "fetch",
            &config.remote,
            &format!("{}:{}", config.main_branch, config.main_branch),
        ],
    );

    // Step 5: Merge into local main using --ff-only
    println!("Merging {} into local main...", branch);
    merge_ff_only(ctx, config, task_id, task_path, task_file, branch)?;

    // Step 6: Optional push
    if config.push_main_on_approve {
        println!("Pushing main to {}...", config.remote);
        push_main(ctx, config)?;
    }

    // Step 7: Cleanup worktree and branch (best-effort)
    println!("Cleaning up worktree and branch...");
    let cleanup_result =
        cleanup_task_worktree(&ctx.repo_root, branch, Some(worktree_path.as_path()), true);

    let cleanup_failed = cleanup_result.is_err();
    if let Err(e) = &cleanup_result {
        eprintln!("Warning: cleanup failed (will proceed anyway): {}", e);
    }

    // Step 8: Workflow state mutation
    complete_approval(ctx, config, task_id, task_path, task_file, cleanup_failed)?;

    println!();
    println!("Approved task: {}", task_id);
    println!("  Title:     {}", task_file.frontmatter.title);
    println!("  From:      QA");
    println!("  To:        DONE");
    println!("  Branch:    {} (merged to {})", branch, config.main_branch);
    if cleanup_failed {
        println!("  Cleanup:   Failed (run `burl clean` to remove leftovers)");
    } else {
        println!("  Cleanup:   Complete");
    }
    if config.push_main_on_approve {
        println!(
            "  Pushed:    {} -> {}/{}",
            config.main_branch, config.remote, config.main_branch
        );
    }

    Ok(())
}

/// Validation result with all step results.
struct ValidationResult {
    all_passed: bool,
    results: Vec<ValidationStepResult>,
}

/// Run all validation checks against the given diff base.
fn run_validation(
    _ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_file: &TaskFile,
    worktree_path: &PathBuf,
    diff_base: &str,
) -> Result<ValidationResult> {
    let mut results: Vec<ValidationStepResult> = Vec::new();
    let mut all_passed = true;

    // Get changed files and added lines for validation
    let changed = changed_files(worktree_path, diff_base)?;
    let added = added_lines(worktree_path, diff_base)?;

    // --- Scope validation ---
    let scope_result = validate_scope(&task_file.frontmatter, &changed)?;
    if scope_result.passed {
        results.push(ValidationStepResult::pass("scope"));
    } else {
        all_passed = false;
        let error_msg = scope_result.format_error(&task_file.frontmatter.id);
        results.push(ValidationStepResult::fail("scope", &error_msg));
    }

    // --- Stub validation ---
    let stub_result = validate_stubs_with_config(config, &added)?;
    if stub_result.passed {
        results.push(ValidationStepResult::pass("stubs"));
    } else {
        all_passed = false;
        let error_msg = stub_result.format_error();
        results.push(ValidationStepResult::fail("stubs", &error_msg));
    }

    // --- Build/test validation ---
    if !config.build_command.trim().is_empty() {
        let build_result = run_build_command(&config.build_command, worktree_path)?;
        if build_result.passed {
            results.push(ValidationStepResult::pass("build/test"));
        } else {
            all_passed = false;
            let error_msg = format_build_error(&build_result);
            results.push(ValidationStepResult::fail("build/test", &error_msg));
        }
    }

    Ok(ValidationResult {
        all_passed,
        results,
    })
}

/// Parse and run the build command in the given worktree directory.
fn run_build_command(build_command: &str, worktree_path: &PathBuf) -> Result<BuildTestResult> {
    let args = shell_words::split(build_command).map_err(|e| {
        BurlError::UserError(format!(
            "failed to parse build_command '{}': {}\n\n\
             Fix: check for unmatched quotes or invalid escape sequences in config.yaml build_command.",
            build_command, e
        ))
    })?;

    if args.is_empty() {
        return Err(BurlError::UserError(
            "build_command is empty after parsing.\n\n\
             Fix: provide a valid command in config.yaml build_command."
                .to_string(),
        ));
    }

    let program = &args[0];
    let cmd_args = &args[1..];

    let output = Command::new(program)
        .args(cmd_args)
        .current_dir(worktree_path)
        .output()
        .map_err(|e| {
            BurlError::UserError(format!(
                "failed to execute build_command '{}': {}\n\n\
                 Fix: ensure the command is installed and in PATH.",
                build_command, e
            ))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(BuildTestResult {
        passed: output.status.success(),
        exit_code,
        stdout,
        stderr,
    })
}

/// Format the build/test error message for the QA Report.
fn format_build_error(result: &BuildTestResult) -> String {
    let mut msg = format!("Build/test failed with exit code {}\n", result.exit_code);

    let combined = if !result.stderr.is_empty() {
        format!("{}\n{}", result.stdout, result.stderr)
    } else {
        result.stdout.clone()
    };

    let truncated = truncate_output(&combined, QA_REPORT_MAX_LINES, QA_REPORT_MAX_CHARS);
    if !truncated.is_empty() {
        msg.push_str("\nOutput (truncated):\n```\n");
        msg.push_str(&truncated);
        msg.push_str("\n```\n");
    }

    msg
}

/// Truncate output to fit within QA Report limits.
fn truncate_output(output: &str, max_lines: usize, max_chars: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();

    let relevant_lines: Vec<&str> = if lines.len() > max_lines {
        lines[lines.len() - max_lines..].to_vec()
    } else {
        lines
    };

    let mut result = relevant_lines.join("\n");

    if result.len() > max_chars {
        result = format!("...(truncated)...\n{}", &result[result.len() - max_chars..]);
    }

    result
}

/// Format the validation summary for the QA Report.
fn format_validation_summary(results: &[ValidationStepResult], all_passed: bool) -> String {
    let now = Utc::now();
    let mut summary = format!(
        "### Validation Run (approve): {}\n\n**Result:** {}\n\n",
        now.format("%Y-%m-%d %H:%M:%S UTC"),
        if all_passed { "PASS" } else { "FAIL" }
    );

    for result in results {
        let status = if result.passed { "PASS" } else { "FAIL" };
        summary.push_str(&format!("- **{}**: {}\n", result.name, status));

        if let Some(msg) = &result.message {
            for line in msg.lines() {
                summary.push_str(&format!("  {}\n", line));
            }
        }
    }

    summary
}

/// Merge the task branch into local main using --ff-only.
fn merge_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    branch: &str,
) -> Result<()> {
    // Checkout main in the repo root
    run_git(&ctx.repo_root, &["checkout", &config.main_branch]).map_err(|e| {
        BurlError::GitError(format!("failed to checkout {}: {}", config.main_branch, e))
    })?;

    // Attempt fast-forward merge
    let merge_result = run_git(&ctx.repo_root, &["merge", "--ff-only", branch]);

    if let Err(e) = merge_result {
        // Reject the task with non-FF merge reason
        reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("non-FF merge required: {}", e),
        )
    } else {
        Ok(())
    }
}

/// Push main to remote.
fn push_main(ctx: &crate::context::WorkflowContext, config: &Config) -> Result<()> {
    run_git(
        &ctx.repo_root,
        &["push", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to push {} to {}: {}\n\n\
             The merge was successful locally. You can push manually with:\n\
             git push {} {}",
            config.main_branch, config.remote, e, config.remote, config.main_branch
        ))
    })?;
    Ok(())
}

/// Reject a task by moving it from QA to READY with a reason.
fn reject_task(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    reason: &str,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(ctx, "approve-reject")?;

    // Append rejection reason to QA Report
    let rejection_entry = format!(
        "### Rejection: {}\n\n**Reason:** {}\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        reason
    );
    task_file.append_to_qa_report(&rejection_entry);

    // Increment qa_attempts
    task_file.increment_qa_attempts();

    // Check if max attempts reached
    if task_file.frontmatter.qa_attempts >= config.qa_max_attempts {
        // Boost priority if configured
        if config.auto_priority_boost_on_retry && task_file.frontmatter.priority != "high" {
            task_file.frontmatter.priority = "high".to_string();
        }
    }

    // Clear submitted_at for re-work
    task_file.frontmatter.submitted_at = None;

    // Save the updated task file
    task_file.save(task_path)?;

    // Move task QA -> READY
    let filename = task_path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let ready_path = ctx.bucket_path("READY").join(filename);

    crate::fs::move_file(task_path, &ready_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from QA to READY: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_path.display(),
            ready_path.display()
        ))
    })?;

    // Append reject event
    let event = Event::new(EventAction::Reject)
        .with_task(task_id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "reason": reason,
            "qa_attempts": task_file.frontmatter.qa_attempts,
            "triggered_by": "approve"
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_reject(ctx, task_id, reason)?;

        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    println!();
    println!("Rejected task: {}", task_id);
    println!("  Title:       {}", task_file.frontmatter.title);
    println!("  Reason:      {}", reason);
    println!("  From:        QA");
    println!("  To:          READY");
    println!("  QA Attempts: {}", task_file.frontmatter.qa_attempts);
    println!();
    println!("The task branch and worktree have been preserved for rework.");

    // Return an error to signal that approval failed
    Err(BurlError::GitError(format!(
        "approval rejected: {}",
        reason
    )))
}

/// Complete the approval by updating workflow state.
fn complete_approval(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    cleanup_failed: bool,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock
    let _workflow_lock = acquire_workflow_lock(ctx, "approve")?;

    // Set completed_at
    let now = Utc::now();
    task_file.set_completed(now);

    // Append success to QA Report
    let success_entry = format!(
        "### Approved: {}\n\n**Result:** Merged to main\n",
        now.format("%Y-%m-%d %H:%M:%S UTC")
    );
    task_file.append_to_qa_report(&success_entry);

    // Save the updated task file
    task_file.save(task_path)?;

    // Move task QA -> DONE
    let filename = task_path
        .file_name()
        .ok_or_else(|| BurlError::UserError("invalid task file path".to_string()))?;
    let done_path = ctx.bucket_path("DONE").join(filename);

    crate::fs::move_file(task_path, &done_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to move task from QA to DONE: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_path.display(),
            done_path.display()
        ))
    })?;

    // Append approve event
    let event = Event::new(EventAction::Approve)
        .with_task(task_id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "branch": task_file.frontmatter.branch,
            "cleanup_failed": cleanup_failed,
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_approve(ctx, task_id, &task_file.frontmatter.title)?;

        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the approval to the workflow branch.
fn commit_approve(ctx: &crate::context::WorkflowContext, task_id: &str, title: &str) -> Result<()> {
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage approve changes: {}", e)))?;

    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    let commit_msg = format!("Approve task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit approve: {}", e)))?;

    Ok(())
}

/// Commit the rejection to the workflow branch.
fn commit_reject(ctx: &crate::context::WorkflowContext, task_id: &str, reason: &str) -> Result<()> {
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage reject changes: {}", e)))?;

    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Truncate reason for commit message
    let short_reason = if reason.len() > 50 {
        format!("{}...", &reason[..47])
    } else {
        reason.to_string()
    };

    let commit_msg = format!("Reject task {} (approve): {}", task_id, short_reason);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit reject: {}", e)))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::AddArgs;
    use crate::cli::ClaimArgs;
    use crate::cli::SubmitArgs;
    use crate::commands::add::cmd_add;
    use crate::commands::claim::cmd_claim;
    use crate::commands::init::cmd_init;
    use crate::commands::submit::cmd_submit;
    use crate::exit_codes;
    use crate::test_support::{DirGuard, create_test_repo_with_remote};
    use serial_test::serial;
    use std::process::Command as ProcessCommand;
    use tempfile::TempDir;

    /// Helper to create a task in QA state with valid changes.
    fn setup_task_in_qa(temp_dir: &TempDir) -> PathBuf {
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-approve");

        // Add a task with glob scope
        cmd_add(AddArgs {
            title: "Test approve".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec!["src/**".to_string()],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Claim the task
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Make a valid change
        std::fs::create_dir_all(worktree_path.join("src")).unwrap();
        std::fs::write(
            worktree_path.join("src/lib.rs"),
            "fn main() {\n    println!(\"Hello, world!\");\n}\n",
        )
        .unwrap();

        // Commit the change
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Add valid implementation"])
            .output()
            .expect("failed to commit");

        // Submit to QA
        cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        worktree_path
    }

    #[test]
    #[serial]
    fn test_approve_task_not_in_qa_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task (stays in READY)
        cmd_add(AddArgs {
            title: "Test task".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Try to approve task in READY - should fail
        let result = cmd_approve(ApproveArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not in QA"));
    }

    #[test]
    #[serial]
    fn test_approve_nonexistent_task_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Try to approve a task that doesn't exist
        let result = cmd_approve(ApproveArgs {
            task_id: "TASK-999".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_approve_happy_path() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command to skip build validation
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Approve the task
        let result = cmd_approve(ApproveArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_ok(), "Approve should succeed: {:?}", result);

        // Verify task moved to DONE
        let done_path = temp_dir
            .path()
            .join(".burl/.workflow/DONE/TASK-001-test-approve.md");
        assert!(done_path.exists(), "Task should be in DONE bucket");

        // Verify QA is empty
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-approve.md");
        assert!(!qa_path.exists(), "Task should no longer be in QA");

        // Verify completed_at was set
        let task = TaskFile::load(&done_path).unwrap();
        assert!(task.frontmatter.completed_at.is_some());

        // Verify worktree was cleaned up
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-approve");
        assert!(!worktree_path.exists(), "Worktree should be removed");

        // Verify main branch contains the changes
        let main_log = ProcessCommand::new("git")
            .current_dir(temp_dir.path())
            .args(["log", "--oneline", "main"])
            .output()
            .expect("failed to get git log");
        let log_output = String::from_utf8_lossy(&main_log.stdout);
        assert!(
            log_output.contains("Add valid implementation"),
            "Main should contain the task commit"
        );
    }

    #[test]
    #[serial]
    fn test_approve_with_rebase_conflict_rejects() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        let worktree_path = setup_task_in_qa(&temp_dir);

        // Create a conflicting commit on main AFTER claiming the task
        // First, checkout main in the repo root
        ProcessCommand::new("git")
            .current_dir(temp_dir.path())
            .args(["checkout", "main"])
            .output()
            .expect("failed to checkout main");

        // Create the same file with different content
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::write(
            temp_dir.path().join("src/lib.rs"),
            "fn main() {\n    println!(\"Conflict!\");\n}\n",
        )
        .unwrap();

        ProcessCommand::new("git")
            .current_dir(temp_dir.path())
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(temp_dir.path())
            .args(["commit", "-m", "Conflicting commit on main"])
            .output()
            .expect("failed to commit");

        // Try to approve - should fail with rebase conflict
        let result = cmd_approve(ApproveArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should be a GitError for rebase conflict
        assert_eq!(err.exit_code(), exit_codes::GIT_FAILURE);
        assert!(
            err.to_string().contains("rebase conflict")
                || err.to_string().contains("approval rejected")
        );

        // Verify task moved to READY (rejected)
        let ready_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-approve.md");
        assert!(
            ready_path.exists(),
            "Task should be in READY bucket after rejection"
        );

        // Verify QA is empty
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-approve.md");
        assert!(!qa_path.exists(), "Task should no longer be in QA");

        // Verify worktree was preserved (for rework)
        assert!(
            worktree_path.exists(),
            "Worktree should be preserved for rework"
        );

        // Verify qa_attempts was incremented
        let task = TaskFile::load(&ready_path).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 1);
    }

    #[test]
    fn test_validation_step_result() {
        let pass = ValidationStepResult::pass("scope");
        assert!(pass.passed);
        assert_eq!(pass.name, "scope");
        assert!(pass.message.is_none());

        let fail = ValidationStepResult::fail("stubs", "Found TODO");
        assert!(!fail.passed);
        assert_eq!(fail.name, "stubs");
        assert_eq!(fail.message, Some("Found TODO".to_string()));
    }

    #[test]
    fn test_format_validation_summary() {
        let results = vec![
            ValidationStepResult::pass("scope"),
            ValidationStepResult::fail("stubs", "Found TODO in src/lib.rs"),
        ];
        let summary = format_validation_summary(&results, false);

        assert!(summary.contains("**Result:** FAIL"));
        assert!(summary.contains("**scope**: PASS"));
        assert!(summary.contains("**stubs**: FAIL"));
        assert!(summary.contains("Found TODO"));
    }

    #[test]
    fn test_truncate_output_within_limits() {
        let output = "line1\nline2\nline3";
        let result = truncate_output(output, 10, 1000);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_truncate_output_exceeds_lines() {
        let output = "line1\nline2\nline3\nline4\nline5";
        let result = truncate_output(output, 3, 1000);
        assert_eq!(result, "line3\nline4\nline5");
    }
}
