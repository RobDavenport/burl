//! Validation command pipeline.
//!
//! Supports ordered command steps with optional conditions based on the set of
//! changed files in the task diff.

use crate::config::ValidationCommandStep;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;
use std::process::Command;

/// Maximum number of lines to include in QA Report summaries.
pub const QA_REPORT_MAX_LINES: usize = 50;

/// Maximum total characters for QA Report summaries.
pub const QA_REPORT_MAX_CHARS: usize = 4096;

/// Status of a validation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStepStatus {
    Pass,
    Fail,
    Skip,
}

/// Result of a single validation step.
#[derive(Debug, Clone)]
pub struct ValidationStepResult {
    pub name: String,
    pub status: ValidationStepStatus,
    pub message: Option<String>,
}

impl ValidationStepResult {
    pub fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ValidationStepStatus::Pass,
            message: None,
        }
    }

    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ValidationStepStatus::Fail,
            message: Some(message.into()),
        }
    }

    pub fn skip(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ValidationStepStatus::Skip,
            message: Some(message.into()),
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self.status,
            ValidationStepStatus::Pass | ValidationStepStatus::Skip
        )
    }
}

/// Determine whether a step should run based on changed files.
pub fn should_run_step(step: &ValidationCommandStep, changed_files: &[String]) -> bool {
    let has_globs = !step.run_if_changed_globs.is_empty();
    let has_exts = !step.run_if_changed_extensions.is_empty();

    if !has_globs && !has_exts {
        return true;
    }

    if changed_files.is_empty() {
        return false;
    }

    let ext_match = if has_exts {
        let exts: Vec<String> = step
            .run_if_changed_extensions
            .iter()
            .map(|s| s.trim().trim_start_matches('.').to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        changed_files.iter().any(|path| {
            let ext = file_extension(path);
            ext.is_some_and(|ext| exts.iter().any(|e| e == ext))
        })
    } else {
        false
    };

    if ext_match {
        return true;
    }

    if has_globs && let Ok(globs) = build_globset(&step.run_if_changed_globs) {
        return changed_files.iter().any(|path| globs.is_match(path));
    }

    false
}

/// Run an ordered list of command steps in the given worktree.
pub fn run_command_steps(
    steps: &[ValidationCommandStep],
    changed_files: &[String],
    worktree_path: &Path,
) -> Vec<ValidationStepResult> {
    let mut results = Vec::new();

    for step in steps {
        if !should_run_step(step, changed_files) {
            results.push(ValidationStepResult::skip(
                &step.name,
                "skipped (no matching changed files)",
            ));
            continue;
        }

        results.push(run_command_step(&step.name, &step.command, worktree_path));
    }

    results
}

fn run_command_step(name: &str, command: &str, worktree_path: &Path) -> ValidationStepResult {
    let command = command.trim();
    if command.is_empty() {
        return ValidationStepResult::fail(name, "command is empty");
    }

    let args = match shell_words::split(command) {
        Ok(args) => args,
        Err(e) => {
            return ValidationStepResult::fail(
                name,
                format!(
                    "failed to parse command: {}\nCommand: {}\nFix: check for unmatched quotes or invalid escape sequences.",
                    e, command
                ),
            );
        }
    };

    if args.is_empty() {
        return ValidationStepResult::fail(
            name,
            format!("command is empty after parsing.\nCommand: {}", command),
        );
    }

    let program = &args[0];
    let cmd_args = &args[1..];

    let output = match Command::new(program)
        .args(cmd_args)
        .current_dir(worktree_path)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return ValidationStepResult::fail(
                name,
                format!(
                    "failed to execute command: {}\nCommand: {}\nFix: ensure the command is installed and in PATH.",
                    e, command
                ),
            );
        }
    };

    if output.status.success() {
        return ValidationStepResult::pass(name);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    let combined = if !stderr.is_empty() {
        format!("{}\n{}", stdout, stderr)
    } else {
        stdout
    };

    let mut msg = format!(
        "Command failed with exit code {}\nCommand: {}\n",
        exit_code, command
    );
    let truncated = truncate_output(&combined, QA_REPORT_MAX_LINES, QA_REPORT_MAX_CHARS);
    if !truncated.is_empty() {
        msg.push_str("\nOutput (truncated):\n```\n");
        msg.push_str(&truncated);
        msg.push_str("\n```\n");
    }

    ValidationStepResult::fail(name, msg)
}

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

fn build_globset(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        let normalized = pattern.trim().replace('\\', "/");
        if normalized.is_empty() {
            continue;
        }
        builder.add(Glob::new(&normalized)?);
    }

    builder.build()
}

fn file_extension(path: &str) -> Option<&str> {
    let (_, ext) = path.rsplit_once('.')?;
    let ext = ext.trim();
    if ext.is_empty() { None } else { Some(ext) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ValidationCommandStep;
    use tempfile::TempDir;

    #[test]
    fn test_should_run_step_unconditional() {
        let step = ValidationCommandStep {
            name: "always".to_string(),
            command: "echo hi".to_string(),
            ..Default::default()
        };
        assert!(should_run_step(&step, &[]));
    }

    #[test]
    fn test_should_run_step_extensions() {
        let step = ValidationCommandStep {
            name: "rust".to_string(),
            command: "cargo test".to_string(),
            run_if_changed_extensions: vec!["rs".to_string()],
            ..Default::default()
        };

        assert!(!should_run_step(&step, &[]));
        assert!(should_run_step(&step, &["src/lib.rs".to_string()]));
        assert!(!should_run_step(&step, &["README.md".to_string()]));
    }

    #[test]
    fn test_should_run_step_globs() {
        let step = ValidationCommandStep {
            name: "cargo".to_string(),
            command: "cargo test".to_string(),
            run_if_changed_globs: vec!["Cargo.toml".to_string(), "src/**/*.rs".to_string()],
            ..Default::default()
        };

        assert!(should_run_step(&step, &["Cargo.toml".to_string()]));
        assert!(should_run_step(&step, &["src/lib.rs".to_string()]));
        assert!(!should_run_step(&step, &["package.json".to_string()]));
    }

    #[test]
    fn test_run_command_step_pass_and_fail() {
        let temp = TempDir::new().unwrap();

        let pass = run_command_step("pass", "git --version", temp.path());
        assert_eq!(pass.status, ValidationStepStatus::Pass);

        let fail = run_command_step("fail", "git definitely-not-a-command", temp.path());
        assert_eq!(fail.status, ValidationStepStatus::Fail);
        assert!(fail.message.unwrap_or_default().contains("exit code"));
    }
}
