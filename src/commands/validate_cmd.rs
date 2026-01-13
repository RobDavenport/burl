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

use crate::cli::ValidateArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::diff::{added_lines, changed_files};
use crate::error::{BurlError, Result};
use crate::events::{append_event, Event, EventAction};
use crate::git::run_git;
use crate::git_worktree::get_current_branch;
use crate::locks::{acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::validate::{validate_scope, validate_stubs_with_config};
use crate::workflow::{validate_task_id, TaskIndex};
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

    // --- Build/test validation ---
    if !config.build_command.trim().is_empty() {
        let build_result = run_build_command(&config.build_command, &worktree_path)?;
        if build_result.passed {
            validation_results.push(ValidationStepResult::pass("build/test"));
        } else {
            all_passed = false;
            let error_msg = format_build_error(&build_result);
            validation_results.push(ValidationStepResult::fail("build/test", &error_msg));
        }
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
        let status = if result.passed { "PASS" } else { "FAIL" };
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

/// Parse and run the build command in the given worktree directory.
///
/// Uses shell-words to parse the command into an argv array for deterministic
/// execution without invoking a shell.
fn run_build_command(build_command: &str, worktree_path: &PathBuf) -> Result<BuildTestResult> {
    // Parse the command using shell-words
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

    // Run the command
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

    // Combine stdout and stderr, truncate for QA report
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

    // Take last N lines (most likely to contain errors)
    let relevant_lines: Vec<&str> = if lines.len() > max_lines {
        lines[lines.len() - max_lines..].to_vec()
    } else {
        lines
    };

    let mut result = relevant_lines.join("\n");

    // Truncate by character count if still too long
    if result.len() > max_chars {
        result = format!("...(truncated)...\n{}", &result[result.len() - max_chars..]);
    }

    result
}

/// Format the validation summary for the QA Report.
fn format_validation_summary(results: &[ValidationStepResult], all_passed: bool) -> String {
    let now = Utc::now();
    let mut summary = format!(
        "### Validation Run: {}\n\n**Result:** {}\n\n",
        now.format("%Y-%m-%d %H:%M:%S UTC"),
        if all_passed { "PASS" } else { "FAIL" }
    );

    for result in results {
        let status = if result.passed { "PASS" } else { "FAIL" };
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

/// Write QA Report entry to task file and append validate event.
fn write_qa_report_and_event(
    ctx: &crate::context::WorkflowContext,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    task_id: &str,
    passed: bool,
    summary: &str,
    config: &Config,
) -> Result<()> {
    // Verify workflow worktree is clean before acquiring lock
    ctx.ensure_workflow_clean()?;

    // Acquire workflow lock for state mutation
    let _workflow_lock = acquire_workflow_lock(ctx, "validate")?;

    // Append to QA Report in task file
    task_file.append_to_qa_report(summary);

    // Atomically write updated task file
    task_file.save(task_path)?;

    // Append validate event
    let event = Event::new(EventAction::Validate)
        .with_task(task_id)
        .with_details(json!({
            "passed": passed,
            "title": task_file.frontmatter.title
        }));
    append_event(ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_validate(ctx, task_id, passed)?;

        // Push if auto-push enabled
        if config.workflow_auto_push {
            push_workflow_branch(ctx, config)?;
        }
    }

    Ok(())
}

/// Commit the validation result to the workflow branch.
fn commit_validate(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    passed: bool,
) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."]).map_err(|e| {
        BurlError::GitError(format!("failed to stage validate changes: {}", e))
    })?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let status = if passed { "passed" } else { "failed" };
    let commit_msg = format!("Validate task {}: {}", task_id, status);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg]).map_err(|e| {
        BurlError::GitError(format!("failed to commit validate: {}", e))
    })?;

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
    use serial_test::serial;
    use std::path::PathBuf;
    use std::process::Command as ProcessCommand;
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

    /// Create a temporary git repository for testing with remote.
    fn create_test_repo_with_remote() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        // Configure git user for commits
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        ProcessCommand::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        // Rename default branch to main
        let _ = ProcessCommand::new("git")
            .current_dir(path)
            .args(["branch", "-M", "main"])
            .output();

        // Create initial commit
        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        // Add second commit
        std::fs::write(path.join("file2.txt"), "Second file\n").unwrap();
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Second commit"])
            .output()
            .expect("failed to commit");

        // Add remote pointing to itself (simulates remote for fetch)
        let path_str = path.to_string_lossy();
        ProcessCommand::new("git")
            .current_dir(path)
            .args(["remote", "add", "origin", &path_str])
            .output()
            .expect("failed to add remote");

        temp_dir
    }

    /// Helper to create a task in QA state.
    fn setup_task_in_qa(temp_dir: &TempDir) -> PathBuf {
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-validate");

        // Add a task with glob scope
        cmd_add(AddArgs {
            title: "Test validate".to_string(),
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

    #[test]
    fn test_truncate_output_exceeds_chars() {
        let output = "a".repeat(100);
        let result = truncate_output(&output, 1000, 50);
        assert!(result.len() <= 70); // truncation prefix + 50 chars
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_validation_step_result_pass() {
        let result = ValidationStepResult::pass("scope");
        assert!(result.passed);
        assert_eq!(result.name, "scope");
        assert!(result.message.is_none());
    }

    #[test]
    fn test_validation_step_result_fail() {
        let result = ValidationStepResult::fail("stubs", "Found TODO");
        assert!(!result.passed);
        assert_eq!(result.name, "stubs");
        assert_eq!(result.message, Some("Found TODO".to_string()));
    }

    #[test]
    fn test_format_validation_summary_all_pass() {
        let results = vec![
            ValidationStepResult::pass("scope"),
            ValidationStepResult::pass("stubs"),
        ];
        let summary = format_validation_summary(&results, true);

        assert!(summary.contains("**Result:** PASS"));
        assert!(summary.contains("**scope**: PASS"));
        assert!(summary.contains("**stubs**: PASS"));
    }

    #[test]
    fn test_format_validation_summary_with_failure() {
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
    #[serial]
    fn test_validate_task_not_in_qa_fails() {
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

        // Try to validate task in READY - should fail
        let result = cmd_validate(ValidateArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not in QA"));
    }

    #[test]
    #[serial]
    fn test_validate_nonexistent_task_fails() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Try to validate a task that doesn't exist
        let result = cmd_validate(ValidateArgs {
            task_id: "TASK-999".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_validate_passing_task() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command to skip build validation
        let config_path = temp_dir
            .path()
            .join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Validate the task
        let result = cmd_validate(ValidateArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_ok(), "Validate should succeed: {:?}", result);

        // Verify task is still in QA (validate doesn't move)
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-validate.md");
        assert!(qa_path.exists(), "Task should still be in QA");

        // Verify QA Report was written
        let task = TaskFile::load(&qa_path).unwrap();
        assert!(task.body.contains("## QA Report"));
        assert!(task.body.contains("**Result:** PASS"));
    }

    #[test]
    #[serial]
    fn test_validate_with_scope_violation() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir
            .path()
            .join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Add task with restricted scope
        cmd_add(AddArgs {
            title: "Test scope violation".to_string(),
            priority: "high".to_string(),
            affects: vec!["allowed/file.rs".to_string()],
            affects_globs: vec![],
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

        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-scope-violation");

        // Make a change OUTSIDE the allowed scope
        std::fs::create_dir_all(worktree_path.join("not_allowed")).unwrap();
        std::fs::write(
            worktree_path.join("not_allowed/bad.rs"),
            "// Out of scope\n",
        )
        .unwrap();

        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(&worktree_path)
            .args(["commit", "-m", "Out of scope change"])
            .output()
            .expect("failed to commit");

        // Submit to QA (with a permissive submit - this would fail normally, but we need to test validate)
        // Let's adjust scope to allow submission, then reset it
        let task_path = temp_dir
            .path()
            .join(".burl/.workflow/DOING/TASK-001-test-scope-violation.md");
        let mut task = TaskFile::load(&task_path).unwrap();
        task.frontmatter.affects_globs.push("**".to_string());
        task.save(&task_path).unwrap();

        // Commit the scope change before submit
        let workflow_path = temp_dir.path().join(".burl");
        ProcessCommand::new("git")
            .current_dir(&workflow_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(&workflow_path)
            .args(["commit", "-m", "Widen scope for test"])
            .output()
            .expect("failed to commit");

        cmd_submit(SubmitArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Now reset the scope to trigger validation failure
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-scope-violation.md");
        let mut task = TaskFile::load(&qa_path).unwrap();
        task.frontmatter.affects_globs.clear();
        task.save(&qa_path).unwrap();

        // Commit the scope change in workflow worktree to make it clean
        let workflow_path = temp_dir.path().join(".burl");
        ProcessCommand::new("git")
            .current_dir(&workflow_path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        ProcessCommand::new("git")
            .current_dir(&workflow_path)
            .args(["commit", "-m", "Reset scope for test"])
            .output()
            .expect("failed to commit");

        // Validate should fail with scope violation
        let result = cmd_validate(ValidateArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::VALIDATION_FAILURE);

        // Verify QA Report records the failure
        let task = TaskFile::load(&qa_path).unwrap();
        assert!(task.body.contains("**Result:** FAIL"));
        assert!(task.body.contains("**scope**: FAIL"));
    }

    #[test]
    #[serial]
    fn test_validate_with_empty_build_command() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Write config with empty build_command
        let config_path = temp_dir
            .path()
            .join(".burl/.workflow/config.yaml");
        std::fs::write(&config_path, "build_command: \"\"\n").unwrap();

        // Setup task in QA
        setup_task_in_qa(&temp_dir);

        // Validate - should only run scope/stubs, not build
        let result = cmd_validate(ValidateArgs {
            task_id: "TASK-001".to_string(),
        });

        assert!(result.is_ok(), "Validate should succeed: {:?}", result);

        // Verify QA Report doesn't mention build/test
        let qa_path = temp_dir
            .path()
            .join(".burl/.workflow/QA/TASK-001-test-validate.md");
        let task = TaskFile::load(&qa_path).unwrap();
        // Should have scope and stubs, but not build/test
        assert!(task.body.contains("**scope**: PASS"));
        assert!(task.body.contains("**stubs**: PASS"));
    }

    #[test]
    fn test_shell_words_parsing() {
        // Test that shell-words correctly parses commands
        let cmd = "cargo test --features foo";
        let args = shell_words::split(cmd).unwrap();
        assert_eq!(args, vec!["cargo", "test", "--features", "foo"]);

        // Test with quotes
        let cmd = "echo \"hello world\"";
        let args = shell_words::split(cmd).unwrap();
        assert_eq!(args, vec!["echo", "hello world"]);

        // Test invalid (unmatched quote) returns error
        let cmd = "echo \"unmatched";
        let result = shell_words::split(cmd);
        assert!(result.is_err());
    }
}
