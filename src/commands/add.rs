//! Implementation of the `burl add` command.
//!
//! Creates a new task file in the READY bucket with the specified metadata.

use crate::cli::AddArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::fs::atomic_write_file;
use crate::git::run_git;
use crate::locks;
use crate::task::{TaskFile, TaskFrontmatter};
use crate::workflow::{
    TaskIndex, generate_task_filename, generate_task_id, validate_filename_safe,
};
use chrono::Utc;
use serde_json::json;

/// Default task body template.
const TASK_BODY_TEMPLATE: &str = r#"
## Objective
<!-- Single sentence describing what "done" looks like -->

## Acceptance Criteria
- [ ] Criterion 1 (specific, verifiable)
- [ ] Criterion 2
- [ ] Criterion 3

## Context
<!-- Relevant notes/links, constraints, file references -->

## Implementation Notes
<!-- Worker fills -->

## QA Report
<!-- Validator fills (tool can append) -->
"#;

/// Execute the `burl add` command.
///
/// Creates a new task file in the READY bucket with:
/// - Auto-generated numeric ID (monotonic, scanning all buckets)
/// - Slugified title for filename
/// - YAML frontmatter with provided metadata
/// - Standard body template
pub fn cmd_add(args: AddArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;

    // Validate priority
    let priority = validate_priority(&args.priority)?;

    // Acquire workflow lock
    let _workflow_lock = locks::acquire_workflow_lock(&ctx, "add")?;

    // Build task index to find the next available ID
    let index = TaskIndex::build(&ctx)?;
    let task_number = index.next_number();
    let task_id = generate_task_id(task_number);

    // Generate filename
    let filename = generate_task_filename(&task_id, &args.title);

    // Validate filename is safe (no path traversal)
    validate_filename_safe(&filename)?;

    // Build the task file path
    let task_path = ctx.bucket_path("READY").join(&filename);

    // Verify the file doesn't already exist (shouldn't happen with monotonic IDs)
    if task_path.exists() {
        return Err(BurlError::UserError(format!(
            "task file already exists: {}",
            task_path.display()
        )));
    }

    // Create the task frontmatter
    let frontmatter = TaskFrontmatter {
        id: task_id.clone(),
        title: args.title.clone(),
        priority,
        created: Some(Utc::now()),
        assigned_to: None,
        qa_attempts: 0,
        started_at: None,
        submitted_at: None,
        completed_at: None,
        worktree: None,
        branch: None,
        base_sha: None,
        affects: args.affects,
        affects_globs: args.affects_globs,
        must_not_touch: args.must_not_touch,
        depends_on: args.depends_on,
        tags: args.tags,
        agent: None,
        validation_profile: None,
        extra: Default::default(),
    };

    // Create the task file
    let task = TaskFile {
        frontmatter,
        body: TASK_BODY_TEMPLATE.to_string(),
    };

    // Write the task file atomically
    let content = task.to_string()?;
    atomic_write_file(&task_path, &content)?;

    // Load config to check if auto-commit is enabled
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Append event
    let event = Event::new(EventAction::Add)
        .with_task(&task_id)
        .with_details(json!({
            "title": args.title,
            "priority": task.frontmatter.priority,
            "filename": filename,
            "path": task_path.display().to_string()
        }));
    append_event(&ctx, &event)?;

    // Commit if auto-commit is enabled
    if config.workflow_auto_commit {
        commit_task_addition(&ctx, &task_id, &args.title)?;

        // Push if auto-push is enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // Print success message
    println!("Created task: {}", task_id);
    println!();
    println!("  Title:    {}", args.title);
    println!("  Priority: {}", task.frontmatter.priority);
    println!("  Path:     {}", task_path.display());
    println!();
    println!("Next steps:");
    println!("  1. Edit the task file to add objectives and acceptance criteria");
    println!("  2. Run `burl claim {}` to start working on it", task_id);

    Ok(())
}

/// Validate and normalize priority value.
fn validate_priority(priority: &str) -> Result<String> {
    let normalized = priority.to_lowercase();
    match normalized.as_str() {
        "high" | "medium" | "low" => Ok(normalized),
        _ => Err(BurlError::UserError(format!(
            "invalid priority '{}': must be 'high', 'medium', or 'low'",
            priority
        ))),
    }
}

/// Commit the task addition to the workflow branch.
fn commit_task_addition(
    ctx: &crate::context::WorkflowContext,
    task_id: &str,
    title: &str,
) -> Result<()> {
    // Stage the task file
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage task file: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("Add task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit task: {}", e)))?;

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
    use crate::commands::init::cmd_init;
    use crate::test_support::{DirGuard, create_test_repo};
    use serial_test::serial;

    #[test]
    fn test_validate_priority_valid() {
        assert_eq!(validate_priority("high").unwrap(), "high");
        assert_eq!(validate_priority("HIGH").unwrap(), "high");
        assert_eq!(validate_priority("Medium").unwrap(), "medium");
        assert_eq!(validate_priority("LOW").unwrap(), "low");
    }

    #[test]
    fn test_validate_priority_invalid() {
        assert!(validate_priority("urgent").is_err());
        assert!(validate_priority("critical").is_err());
        assert!(validate_priority("").is_err());
    }

    #[test]
    #[serial]
    fn test_add_creates_task_file() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task
        let args = AddArgs {
            title: "Test task".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/lib.rs".to_string()],
            affects_globs: vec!["*.rs".to_string()],
            must_not_touch: vec!["vendor/".to_string()],
            depends_on: vec![],
            tags: vec!["test".to_string()],
        };
        cmd_add(args).unwrap();

        // Verify task file was created
        let task_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-test-task.md");
        assert!(
            task_path.exists(),
            "Task file should exist at {:?}",
            task_path
        );

        // Verify task content
        let task = TaskFile::load(&task_path).unwrap();
        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.title, "Test task");
        assert_eq!(task.frontmatter.priority, "high");
        assert_eq!(task.frontmatter.affects, vec!["src/lib.rs"]);
        assert_eq!(task.frontmatter.affects_globs, vec!["*.rs"]);
        assert_eq!(task.frontmatter.must_not_touch, vec!["vendor/"]);
        assert_eq!(task.frontmatter.tags, vec!["test"]);
    }

    #[test]
    #[serial]
    fn test_add_increments_task_id() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add first task
        let args1 = AddArgs {
            title: "First task".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        cmd_add(args1).unwrap();

        // Add second task
        let args2 = AddArgs {
            title: "Second task".to_string(),
            priority: "medium".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        cmd_add(args2).unwrap();

        // Verify both tasks exist with correct IDs
        let task1_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-001-first-task.md");
        let task2_path = temp_dir
            .path()
            .join(".burl/.workflow/READY/TASK-002-second-task.md");
        assert!(task1_path.exists());
        assert!(task2_path.exists());

        let task1 = TaskFile::load(&task1_path).unwrap();
        let task2 = TaskFile::load(&task2_path).unwrap();
        assert_eq!(task1.frontmatter.id, "TASK-001");
        assert_eq!(task2.frontmatter.id, "TASK-002");
    }
}
