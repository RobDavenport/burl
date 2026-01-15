//! Validation logic for the approve command.
//!
//! This module handles scope validation, stub validation, and build/test execution.

use crate::config::Config;
use crate::config::ValidationCommandStep;
use crate::diff::{added_lines, changed_files};
use crate::error::Result;
use crate::task::TaskFile;
use crate::validate::{ValidationStepResult, ValidationStepStatus, run_command_steps};
use crate::validate::{validate_scope, validate_stubs_with_config};
use chrono::Utc;
use std::path::{Path, PathBuf};

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

    // --- Command validation pipeline ---
    let pipeline_results = run_validation_pipeline(config, task_file, &changed, worktree_path);
    for result in pipeline_results {
        if !result.is_success() {
            all_passed = false;
        }
        results.push(result);
    }

    Ok(ValidationResult {
        all_passed,
        results,
    })
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
        let status = match result.status {
            ValidationStepStatus::Pass => "PASS",
            ValidationStepStatus::Fail => "FAIL",
            ValidationStepStatus::Skip => "SKIP",
        };
        summary.push_str(&format!("- **{}**: {}\n", result.name, status));

        if let Some(msg) = &result.message {
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
    worktree_path: &Path,
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

fn run_legacy_build_command(config: &Config, worktree_path: &Path) -> Vec<ValidationStepResult> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskFrontmatter;
    use tempfile::TempDir;

    #[test]
    fn test_validation_step_result() {
        let pass = ValidationStepResult::pass("scope");
        assert_eq!(pass.status, ValidationStepStatus::Pass);
        assert_eq!(pass.name, "scope");
        assert!(pass.message.is_none());

        let fail = ValidationStepResult::fail("stubs", "Found TODO");
        assert_eq!(fail.status, ValidationStepStatus::Fail);
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

    fn make_task(validation_profile: Option<&str>) -> TaskFile {
        TaskFile {
            frontmatter: TaskFrontmatter {
                id: "TASK-001".to_string(),
                title: "Test".to_string(),
                validation_profile: validation_profile.map(|s| s.to_string()),
                ..Default::default()
            },
            body: String::new(),
        }
    }

    #[test]
    fn test_run_validation_pipeline_uses_default_profile() {
        let config = Config::from_yaml(
            r#"
build_command: ""
default_validation_profile: quick
validation_profiles:
  quick:
    steps:
      - name: git-version
        command: "git --version"
"#,
        )
        .unwrap();

        let task = make_task(None);
        let worktree = TempDir::new().unwrap();
        let results = super::run_validation_pipeline(&config, &task, &[], worktree.path());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "git-version");
        assert_eq!(results[0].status, ValidationStepStatus::Pass);
    }

    #[test]
    fn test_run_validation_pipeline_no_profile_uses_legacy_build_command() {
        let config = Config::from_yaml("build_command: \"git --version\"").unwrap();
        let task = make_task(None);
        let worktree = TempDir::new().unwrap();

        let results = super::run_validation_pipeline(&config, &task, &[], worktree.path());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "build/test");
        assert_eq!(results[0].status, ValidationStepStatus::Pass);
    }
}
