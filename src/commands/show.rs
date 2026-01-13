//! Implementation of the `burl show` command.
//!
//! Displays the content and metadata of a specific task.

use crate::cli::ShowArgs;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::task::TaskFile;
use crate::workflow::{BUCKETS, TaskIndex, validate_task_id};

/// Execute the `burl show` command.
///
/// Locates a task by ID in any bucket and displays its content,
/// including the bucket name in the header.
pub fn cmd_show(args: ShowArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;

    // Validate and normalize task ID
    let task_id = validate_task_id(&args.task_id)?;

    // Build task index and find the task
    let index = TaskIndex::build(&ctx)?;

    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Searched buckets: {}\n\n\
             Use `burl status` to see all tasks.",
            task_id,
            BUCKETS.join(", ")
        ))
    })?;

    // Load the full task file
    let task = TaskFile::load(&task_info.path)?;

    // Print task header
    println!("================================================================================");
    println!("{} [{}]", task_id, task_info.bucket);
    println!("================================================================================");
    println!();

    // Print key metadata
    println!("Title:      {}", task.frontmatter.title);
    println!("Priority:   {}", task.frontmatter.priority);

    if let Some(created) = task.frontmatter.created {
        println!("Created:    {}", created.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if let Some(assigned) = &task.frontmatter.assigned_to {
        println!("Assigned:   {}", assigned);
    }

    if let Some(started) = task.frontmatter.started_at {
        println!("Started:    {}", started.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if let Some(submitted) = task.frontmatter.submitted_at {
        println!("Submitted:  {}", submitted.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if let Some(completed) = task.frontmatter.completed_at {
        println!("Completed:  {}", completed.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if task.frontmatter.qa_attempts > 0 {
        println!("QA Attempts: {}", task.frontmatter.qa_attempts);
    }

    // Print git info if available
    if task.frontmatter.branch.is_some() || task.frontmatter.worktree.is_some() {
        println!();
        println!("Git:");
        if let Some(branch) = &task.frontmatter.branch {
            println!("  Branch:   {}", branch);
        }
        if let Some(worktree) = &task.frontmatter.worktree {
            println!("  Worktree: {}", worktree);
        }
        if let Some(base_sha) = &task.frontmatter.base_sha {
            println!("  Base SHA: {}", base_sha);
        }
    }

    // Print scope if defined
    if !task.frontmatter.affects.is_empty()
        || !task.frontmatter.affects_globs.is_empty()
        || !task.frontmatter.must_not_touch.is_empty()
    {
        println!();
        println!("Scope:");
        if !task.frontmatter.affects.is_empty() {
            println!("  Affects:");
            for path in &task.frontmatter.affects {
                println!("    - {}", path);
            }
        }
        if !task.frontmatter.affects_globs.is_empty() {
            println!("  Affects (globs):");
            for glob in &task.frontmatter.affects_globs {
                println!("    - {}", glob);
            }
        }
        if !task.frontmatter.must_not_touch.is_empty() {
            println!("  Must Not Touch:");
            for path in &task.frontmatter.must_not_touch {
                println!("    - {}", path);
            }
        }
    }

    // Print dependencies if any
    if !task.frontmatter.depends_on.is_empty() {
        println!();
        println!("Dependencies:");
        for dep in &task.frontmatter.depends_on {
            println!("  - {}", dep);
        }
    }

    // Print tags if any
    if !task.frontmatter.tags.is_empty() {
        println!();
        println!("Tags: {}", task.frontmatter.tags.join(", "));
    }

    // Print body
    println!();
    println!("--------------------------------------------------------------------------------");
    println!();

    // Print the body content, trimmed of leading/trailing whitespace
    let body = task.body.trim();
    if !body.is_empty() {
        println!("{}", body);
    } else {
        println!("(No body content)");
    }

    println!();
    println!("--------------------------------------------------------------------------------");
    println!("Path: {}", task_info.path.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AddArgs, ShowArgs};
    use crate::commands::add::cmd_add;
    use crate::commands::init::cmd_init;
    use crate::test_support::{DirGuard, create_test_repo};
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_show_displays_task() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let add_args = AddArgs {
            title: "Test task for show".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/".to_string()],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec!["feature".to_string()],
        };
        cmd_add(add_args).unwrap();

        let show_args = ShowArgs {
            task_id: "TASK-001".to_string(),
        };
        let result = cmd_show(show_args);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_show_normalizes_task_id() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let add_args = AddArgs {
            title: "Test task".to_string(),
            priority: "medium".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        cmd_add(add_args).unwrap();

        let show_args = ShowArgs {
            task_id: "task-001".to_string(),
        };
        let result = cmd_show(show_args);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_show_not_found() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let show_args = ShowArgs {
            task_id: "TASK-999".to_string(),
        };
        let result = cmd_show(show_args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    #[serial]
    fn test_show_invalid_task_id() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let show_args = ShowArgs {
            task_id: "invalid".to_string(),
        };
        let result = cmd_show(show_args);
        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_show_rejects_path_traversal() {
        let temp_dir = create_test_repo();
        let _guard = DirGuard::new(temp_dir.path());

        cmd_init().unwrap();

        let show_args = ShowArgs {
            task_id: "../TASK-001".to_string(),
        };
        let result = cmd_show(show_args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }
}
