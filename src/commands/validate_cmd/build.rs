//! Build/test command execution and output formatting.

use crate::error::{BurlError, Result};
use std::path::PathBuf;
use std::process::Command;

/// Maximum number of lines to include in QA Report summary.
const QA_REPORT_MAX_LINES: usize = 50;

/// Maximum total characters for QA Report summary.
const QA_REPORT_MAX_CHARS: usize = 4096;

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

/// Parse and run the build command in the given worktree directory.
///
/// Uses shell-words to parse the command into an argv array for deterministic
/// execution without invoking a shell.
pub fn run_build_command(build_command: &str, worktree_path: &PathBuf) -> Result<BuildTestResult> {
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
pub fn format_build_error(result: &BuildTestResult) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
