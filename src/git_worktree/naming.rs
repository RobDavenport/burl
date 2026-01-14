//! Branch naming and path utilities for task worktrees.

use crate::context::WorkflowContext;
use std::path::PathBuf;

/// Generate the conventional branch name for a task.
///
/// Format: `task-{numeric_id}-{slug}`
/// Example: `task-001-player-jump`
///
/// # Arguments
///
/// * `task_id` - The task ID (e.g., "TASK-001" or "001")
/// * `slug` - Optional slug for the task (derived from title if not provided)
pub fn task_branch_name(task_id: &str, slug: Option<&str>) -> String {
    // Extract numeric part from task ID (e.g., "TASK-001" -> "001", "001" -> "001")
    let numeric = task_id
        .strip_prefix("TASK-")
        .unwrap_or(task_id)
        .to_lowercase();

    match slug {
        Some(s) if !s.is_empty() => format!("task-{}-{}", numeric, sanitize_slug(s)),
        _ => format!("task-{}", numeric),
    }
}

/// Generate the conventional worktree path for a task.
///
/// Format: `.worktrees/task-{numeric_id}-{slug}/`
/// Example: `.worktrees/task-001-player-jump/`
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `task_id` - The task ID (e.g., "TASK-001")
/// * `slug` - Optional slug for the task
pub fn task_worktree_path(ctx: &WorkflowContext, task_id: &str, slug: Option<&str>) -> PathBuf {
    let branch_name = task_branch_name(task_id, slug);
    ctx.worktrees_dir.join(branch_name)
}

/// Sanitize a string for use in branch names.
///
/// Converts to lowercase, replaces spaces and special chars with hyphens,
/// removes consecutive hyphens, and trims leading/trailing hyphens.
pub(crate) fn sanitize_slug(s: &str) -> String {
    let mut result = String::new();
    let mut last_was_hyphen = true; // Start true to avoid leading hyphen

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            result.push('-');
            last_was_hyphen = true;
        }
    }

    // Trim trailing hyphen
    while result.ends_with('-') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_branch_name() {
        assert_eq!(task_branch_name("TASK-001", None), "task-001");
        assert_eq!(task_branch_name("TASK-001", Some("")), "task-001");
        assert_eq!(
            task_branch_name("TASK-001", Some("player-jump")),
            "task-001-player-jump"
        );
        assert_eq!(
            task_branch_name("TASK-001", Some("Player Jump")),
            "task-001-player-jump"
        );
        assert_eq!(task_branch_name("001", Some("feature")), "task-001-feature");
    }

    #[test]
    fn test_sanitize_slug() {
        assert_eq!(sanitize_slug("player-jump"), "player-jump");
        assert_eq!(sanitize_slug("Player Jump"), "player-jump");
        assert_eq!(sanitize_slug("Player  Jump"), "player-jump");
        assert_eq!(sanitize_slug("Feature: New Thing!"), "feature-new-thing");
        assert_eq!(sanitize_slug("  spaces  "), "spaces");
        assert_eq!(sanitize_slug("CamelCase"), "camelcase");
        assert_eq!(sanitize_slug("with_underscores"), "with-underscores");
    }
}
