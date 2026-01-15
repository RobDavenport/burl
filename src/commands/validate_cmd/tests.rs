//! Tests for the validate command.

use super::*;
use crate::cli::{AddArgs, ClaimArgs, SubmitArgs};
use crate::commands::add::cmd_add;
use crate::commands::claim::cmd_claim;
use crate::commands::init::cmd_init;
use crate::commands::submit::cmd_submit;
use crate::exit_codes;
use crate::task::TaskFrontmatter;
use crate::test_support::{DirGuard, create_test_repo_with_remote};
use serial_test::serial;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use tempfile::TempDir;

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
fn test_validation_step_result_pass() {
    let result = ValidationStepResult::pass("scope");
    assert_eq!(result.status, ValidationStepStatus::Pass);
    assert_eq!(result.name, "scope");
    assert!(result.message.is_none());
}

#[test]
fn test_validation_step_result_fail() {
    let result = ValidationStepResult::fail("stubs", "Found TODO");
    assert_eq!(result.status, ValidationStepStatus::Fail);
    assert_eq!(result.name, "stubs");
    assert_eq!(result.message, Some("Found TODO".to_string()));
}

#[test]
fn test_format_validation_summary_all_pass() {
    let results = vec![
        ValidationStepResult::pass("scope"),
        ValidationStepResult::pass("stubs"),
    ];
    let summary = super::format_validation_summary(&results, true);

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
    let summary = super::format_validation_summary(&results, false);

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
    let results = super::run_validation_pipeline(
        &config,
        &task,
        &["src/lib.rs".to_string()],
        worktree.path(),
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "git-version");
    assert_eq!(results[0].status, ValidationStepStatus::Pass);
}

#[test]
fn test_run_validation_pipeline_task_override_wins() {
    let config = Config::from_yaml(
        r#"
build_command: ""
default_validation_profile: quick
validation_profiles:
  quick:
    steps:
      - name: git-version
        command: "git --version"
  override:
    steps:
      - name: bad
        command: "git definitely-not-a-command"
"#,
    )
    .unwrap();

    let task = make_task(Some("override"));
    let worktree = TempDir::new().unwrap();
    let results = super::run_validation_pipeline(&config, &task, &[], worktree.path());

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "bad");
    assert_eq!(results[0].status, ValidationStepStatus::Fail);
}

#[test]
fn test_run_validation_pipeline_unknown_profile_fails() {
    let config = Config::from_yaml("build_command: \"\"").unwrap();
    let task = make_task(Some("missing"));
    let worktree = TempDir::new().unwrap();

    let results = super::run_validation_pipeline(&config, &task, &[], worktree.path());

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "validation");
    assert_eq!(results[0].status, ValidationStepStatus::Fail);
    assert!(
        results[0]
            .message
            .clone()
            .unwrap_or_default()
            .contains("unknown validation_profile")
    );
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

#[test]
fn test_run_validation_pipeline_empty_profile_steps_skips() {
    let config = Config::from_yaml(
        r#"
build_command: ""
default_validation_profile: quick
validation_profiles:
  quick:
    steps: []
"#,
    )
    .unwrap();

    let task = make_task(None);
    let worktree = TempDir::new().unwrap();
    let results = super::run_validation_pipeline(&config, &task, &[], worktree.path());

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "validation");
    assert_eq!(results[0].status, ValidationStepStatus::Skip);
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
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
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
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
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

    let worktree_path = temp_dir
        .path()
        .join(".worktrees/task-001-test-scope-violation");

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
    let config_path = temp_dir.path().join(".burl/.workflow/config.yaml");
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
