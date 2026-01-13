//! Task git/worktree invariant checks.
//!
//! Burl treats task files as durable workflow state. For safety, we validate that
//! recorded git metadata (branch/worktree) follows expected conventions before
//! using it for git operations or filesystem access.
//!
//! Invariants (V1):
//! - Task branches are named `task-<numeric>[-<slug>]` and must match the task ID.
//! - Task worktrees live directly under `{repo_root}/.worktrees/` and the directory
//!   name must match the task branch name.

use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TaskGitRefs {
    pub branch: String,
    pub worktree_path: PathBuf,
}

/// Validate that a recorded branch name is a safe task branch for the given task ID.
pub fn validate_task_branch(task_id: &str, branch: &str) -> Result<()> {
    if branch.is_empty() {
        return Err(BurlError::UserError(
            "task branch cannot be empty".to_string(),
        ));
    }

    // Enforce the convention and a restrictive charset to avoid surprising git
    // argument parsing (e.g. branch names that look like flags) and prevent
    // using unrelated branches.
    if branch != branch.to_ascii_lowercase() {
        return Err(BurlError::UserError(format!(
            "invalid task branch '{}': must be lowercase.\n\n\
             Expected format: task-<NNN>[-slug] (e.g., task-001-player-jump).",
            branch
        )));
    }

    if !branch.starts_with("task-") {
        return Err(BurlError::UserError(format!(
            "invalid task branch '{}': must start with 'task-'.\n\n\
             Expected format: task-<NNN>[-slug] (e.g., task-001-player-jump).",
            branch
        )));
    }

    if branch.contains('/') || branch.contains('\\') || branch.contains("..") {
        return Err(BurlError::UserError(format!(
            "invalid task branch '{}': contains disallowed path characters.",
            branch
        )));
    }

    let numeric_expected = task_id
        .strip_prefix("TASK-")
        .unwrap_or(task_id)
        .to_ascii_lowercase();

    let rest = &branch["task-".len()..];
    let (numeric_actual, slug) = match rest.split_once('-') {
        Some((n, s)) => (n, Some(s)),
        None => (rest, None),
    };

    if numeric_actual.is_empty() || !numeric_actual.chars().all(|c| c.is_ascii_digit()) {
        return Err(BurlError::UserError(format!(
            "invalid task branch '{}': numeric portion must be digits.\n\n\
             Expected format: task-<NNN>[-slug] (e.g., task-001-player-jump).",
            branch
        )));
    }

    if numeric_actual != numeric_expected {
        return Err(BurlError::UserError(format!(
            "invalid task branch '{}': does not match task id '{}'.\n\n\
             Expected prefix: task-{}",
            branch, task_id, numeric_expected
        )));
    }

    if let Some(slug) = slug {
        if slug.is_empty() {
            return Err(BurlError::UserError(format!(
                "invalid task branch '{}': slug portion cannot be empty.",
                branch
            )));
        }

        // Slug must be one or more segments of [a-z0-9]+ separated by single hyphens.
        let mut last_was_hyphen = false;
        for c in slug.chars() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                last_was_hyphen = false;
                continue;
            }
            if c == '-' && !last_was_hyphen {
                last_was_hyphen = true;
                continue;
            }
            return Err(BurlError::UserError(format!(
                "invalid task branch '{}': slug contains invalid characters.\n\n\
                 Allowed: lowercase letters, digits, and single hyphens.",
                branch
            )));
        }

        if last_was_hyphen {
            return Err(BurlError::UserError(format!(
                "invalid task branch '{}': slug cannot end with '-'.",
                branch
            )));
        }
    }

    Ok(())
}

/// Resolve and validate a recorded worktree path for a task branch.
///
/// Returns the canonical expected worktree path under `{repo_root}/.worktrees/<branch>`.
pub fn resolve_task_worktree_path(
    ctx: &WorkflowContext,
    recorded_worktree: &str,
    branch: &str,
) -> Result<PathBuf> {
    if recorded_worktree.trim().is_empty() {
        return Err(BurlError::UserError(
            "task worktree path cannot be empty".to_string(),
        ));
    }

    let expected = ctx.worktrees_dir.join(branch);
    let recorded_path = PathBuf::from(recorded_worktree);

    // If the recorded path is absolute but points to a different machine, allow it
    // if it ends with `.worktrees/<branch>` and remap to the current repo root.
    if recorded_path.is_absolute() {
        if paths_equivalent_or_equal(&recorded_path, &expected)
            || is_worktrees_suffix(&recorded_path, branch)
        {
            return Ok(expected);
        }

        return Err(BurlError::UserError(format!(
            "refusing to use recorded worktree path outside the repository worktrees root.\n\n\
             Task branch: {}\n\
             Recorded:    {}\n\
             Expected:    {}\n\n\
             Remediation: recreate the worktree under '.worktrees/' or run `burl doctor`.",
            branch,
            recorded_path.display(),
            expected.display()
        )));
    }

    // Relative paths must not contain traversal.
    if path_contains_parent_dir(&recorded_path) {
        return Err(BurlError::UserError(format!(
            "refusing to use recorded worktree path with traversal: '{}'",
            recorded_worktree
        )));
    }

    let resolved = ctx.repo_root.join(&recorded_path);

    if paths_equivalent_or_equal(&resolved, &expected) {
        return Ok(expected);
    }

    Err(BurlError::UserError(format!(
        "refusing to use recorded worktree path that does not match the task branch.\n\n\
         Task branch: {}\n\
         Recorded:    {}\n\
         Expected:    {}\n\n\
         Remediation: update the task frontmatter 'worktree' to the expected path.",
        branch,
        resolved.display(),
        expected.display()
    )))
}

/// Require and validate both `branch` and `worktree` for a task.
pub fn require_task_git_refs(
    ctx: &WorkflowContext,
    task_id: &str,
    branch: Option<&str>,
    worktree: Option<&str>,
) -> Result<TaskGitRefs> {
    let branch = branch.ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded branch.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    validate_task_branch(task_id, branch)?;

    let worktree = worktree.ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no recorded worktree.\n\n\
             This task may be in an invalid state. Run `burl doctor` to diagnose.",
            task_id
        ))
    })?;

    let worktree_path = resolve_task_worktree_path(ctx, worktree, branch)?;

    Ok(TaskGitRefs {
        branch: branch.to_string(),
        worktree_path,
    })
}

/// Validate task git refs if either is present.
pub fn validate_task_git_refs_if_present(
    ctx: &WorkflowContext,
    task_id: &str,
    branch: Option<&str>,
    worktree: Option<&str>,
) -> Result<Option<TaskGitRefs>> {
    match (branch, worktree) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(BurlError::UserError(format!(
            "task '{}' has partial git state (branch/worktree mismatch).\n\n\
             Run `burl doctor` to diagnose and repair this inconsistency.",
            task_id
        ))),
        (Some(branch), Some(worktree)) => Ok(Some(require_task_git_refs(
            ctx,
            task_id,
            Some(branch),
            Some(worktree),
        )?)),
    }
}

fn path_contains_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn is_worktrees_suffix(path: &Path, branch: &str) -> bool {
    let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if filename != branch {
        return false;
    }

    let Some(parent_name) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    else {
        return false;
    };

    parent_name == ".worktrees"
}

fn paths_equivalent_or_equal(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a_canon), Ok(b_canon)) => a_canon == b_canon,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::WorkflowContext;
    use crate::test_support::create_test_repo;

    #[test]
    fn validate_task_branch_accepts_valid_branch_with_slug() {
        assert!(validate_task_branch("TASK-001", "task-001-player-jump").is_ok());
        assert!(validate_task_branch("TASK-0001", "task-0001-foo").is_ok());
    }

    #[test]
    fn validate_task_branch_accepts_valid_branch_without_slug() {
        assert!(validate_task_branch("TASK-001", "task-001").is_ok());
    }

    #[test]
    fn validate_task_branch_rejects_mismatched_task_id() {
        let err = validate_task_branch("TASK-001", "task-002-foo").unwrap_err();
        assert!(err.to_string().contains("does not match task id"));
    }

    #[test]
    fn validate_task_branch_rejects_uppercase() {
        let err = validate_task_branch("TASK-001", "task-001-Foo").unwrap_err();
        assert!(err.to_string().contains("must be lowercase"));
    }

    #[test]
    fn resolve_task_worktree_path_accepts_relative_expected_path() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();
        let branch = "task-001-test";

        let resolved =
            resolve_task_worktree_path(&ctx, ".worktrees/task-001-test", branch).unwrap();
        assert_eq!(resolved, ctx.worktrees_dir.join(branch));
    }

    #[test]
    fn resolve_task_worktree_path_remaps_absolute_other_machine_path() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();
        let branch = "task-001-test";

        let recorded = PathBuf::from("/some/other/root/.worktrees/task-001-test");
        let resolved =
            resolve_task_worktree_path(&ctx, &recorded.to_string_lossy(), branch).unwrap();
        assert_eq!(resolved, ctx.worktrees_dir.join(branch));
    }

    #[test]
    fn resolve_task_worktree_path_rejects_traversal() {
        let temp_dir = create_test_repo();
        let ctx = WorkflowContext::resolve_from(temp_dir.path()).unwrap();
        let branch = "task-001-test";

        let err =
            resolve_task_worktree_path(&ctx, "../.worktrees/task-001-test", branch).unwrap_err();
        assert!(err.to_string().contains("traversal"));
    }
}
