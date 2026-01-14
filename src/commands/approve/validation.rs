//! Validation logic for the approve command.
//!
//! This module handles scope validation, stub validation, and build/test execution.

use crate::config::Config;
use crate::diff::{added_lines, changed_files};
use crate::error::{BurlError, Result};
use crate::task::TaskFile;
use crate::validate::{validate_scope, validate_stubs_with_config};
use chrono::Utc;
use std::path::PathBuf;
use std::process::Command;

/// Maximum number of lines to include in QA Report summary.
pub const QA_REPORT_MAX_LINES: usize = 50;

/// Maximum total characters for QA Report summary.
pub const QA_REPORT_MAX_CHARS: usize = 4096;

/// Result of a single validation step.
#[derive(Debug, Clone)]
pub struct ValidationStepResult {
    /// Name of the validation step.
    pub name: String,
    /// Whether it passed.
    pub passed: bool,
    /// Error message if failed.
    pub message: Option<String>,
}

impl ValidationStepResult {
    pub fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: None,
        }
    }

    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: Some(message.into()),
        }
    }
}

/// Result of running the build/test command.
#[derive(Debug)]
pub struct BuildTestResult {
    /// Whether the command succeeded (exit 0).
    pub passed: bool,
    /// Exit code of the command.
    pub exit_code: i32,
    /// Stdout from the command.
    pub stdout: String,
    /// Stderr from the command.
    pub stderr: String,
}

/// Validation result with all step results.
pub struct ValidationResult {
    pub all_passed: bool,
    pub results: Vec<ValidationStepResult>,
}

/// Run all validation checks against the given diff base.
pub fn run_validation(
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
pub fn format_validation_summary(results: &[ValidationStepResult], all_passed: bool) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
