//! Implementation of the `burl claim` command.
//!
//! This module implements the race-safe, transactional claim operation that:
//! - Selects a claimable READY task (or claims explicit ID)
//! - Creates/reuses branch + worktree at `base_sha`
//! - Updates task metadata and moves READY -> DOING atomically
//!
//! # Transaction Steps
//!
//! 1. Acquire per-task lock (`TASK-XXX.lock`)
//! 2. Resolve `base_sha` (fetch origin/main first)
//! 3. Create/reuse branch and worktree
//! 4. Verify workflow worktree has no unexpected tracked modifications
//! 5. Acquire `workflow.lock` for workflow-state mutation
//! 6. Atomically update task frontmatter and move READY -> DOING
//! 7. Append claim event and commit workflow branch
//! 8. Release locks
//!
//! # Rollback
//!
//! If worktree creation fails after branch creation, delete the branch if it
//! was created in this attempt.

use crate::cli::ClaimArgs;
use crate::config::{Config, ConflictPolicy};
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::git::run_git;
use crate::git_worktree::{WorktreeInfo, branch_exists, delete_branch, setup_task_worktree};
use crate::locks::{LockGuard, acquire_claim_lock, acquire_task_lock, acquire_workflow_lock};
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, TaskInfo, slugify_title, validate_task_id};
use chrono::Utc;
use serde_json::json;

/// Priority ordering for task selection (high > medium > low > none/other)
fn priority_rank(priority: &str) -> u32 {
    match priority.to_lowercase().as_str() {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Select the next claimable task deterministically.
///
/// Selection criteria:
/// 1. Task must be in READY bucket
/// 2. Sort by priority (high > medium > low > none)
/// 3. Then by numeric ID ascending
///
/// Returns the task ID of the selected task.
fn select_next_task_id(
    ctx: &crate::context::WorkflowContext,
    ready_tasks: &[&TaskInfo],
) -> Result<Option<String>> {
    if ready_tasks.is_empty() {
        return Ok(None);
    }

    // Load task files to get priority info - collect into owned data
    let mut tasks_with_priority: Vec<(String, u32, u32)> = Vec::new(); // (id, priority_rank, number)

    for task_info in ready_tasks {
        let task_file = TaskFile::load(&task_info.path)?;
        let rank = priority_rank(&task_file.frontmatter.priority);
        tasks_with_priority.push((task_info.id.clone(), rank, task_info.number));
    }

    // Sort by priority rank (ascending = higher priority first), then by numeric ID
    tasks_with_priority.sort_by(|a, b| {
        let priority_cmp = a.1.cmp(&b.1);
        if priority_cmp == std::cmp::Ordering::Equal {
            a.2.cmp(&b.2)
        } else {
            priority_cmp
        }
    });

    // Filter out tasks with unmet dependencies
    for (task_id, _, _) in tasks_with_priority {
        // Re-fetch task info from a fresh index
        let index = TaskIndex::build(ctx)?;
        if let Some(task_info) = index.find(&task_id) {
            let task_file = TaskFile::load(&task_info.path)?;
            if check_dependencies_satisfied(&task_file, &index).is_ok() {
                return Ok(Some(task_id));
            }
        }
    }

    Ok(None)
}

/// Check if all dependencies of a task are in DONE.
fn check_dependencies_satisfied(task: &TaskFile, index: &TaskIndex) -> Result<()> {
    let mut unmet_deps = Vec::new();

    for dep_id in &task.frontmatter.depends_on {
        match index.find(dep_id) {
            Some(dep_info) => {
                if dep_info.bucket != "DONE" {
                    unmet_deps.push(format!("{} (currently in {})", dep_id, dep_info.bucket));
                }
            }
            None => {
                unmet_deps.push(format!("{} (not found)", dep_id));
            }
        }
    }

    if !unmet_deps.is_empty() {
        return Err(BurlError::UserError(format!(
            "cannot claim task: dependencies not satisfied.\n\n\
             Unmet dependencies:\n  - {}\n\n\
             Complete these tasks first before claiming this one.",
            unmet_deps.join("\n  - ")
        )));
    }

    Ok(())
}

/// Check if two scopes overlap.
///
/// Overlap detection rules:
/// - overlap if any explicit `affects` path in task A matches any `affects_globs` pattern in task B (and vice versa)
/// - overlap if any explicit `affects` path is identical between tasks
/// - overlap if any `affects_globs` pattern is identical between tasks
/// - treat prefix relationships as overlap for directory globs (e.g., `src/**` overlaps `src/foo/**`)
fn scopes_overlap(
    task_a_affects: &[String],
    task_a_globs: &[String],
    task_b_affects: &[String],
    task_b_globs: &[String],
) -> bool {
    // Check identical affects paths
    for path_a in task_a_affects {
        if task_b_affects.contains(path_a) {
            return true;
        }
    }

    // Check identical glob patterns
    for glob_a in task_a_globs {
        if task_b_globs.contains(glob_a) {
            return true;
        }
    }

    // Check if any affects path matches a glob pattern (conservative heuristic)
    for path in task_a_affects {
        for glob in task_b_globs {
            if path_matches_glob_heuristic(path, glob) {
                return true;
            }
        }
    }

    for path in task_b_affects {
        for glob in task_a_globs {
            if path_matches_glob_heuristic(path, glob) {
                return true;
            }
        }
    }

    // Check directory glob prefix relationships
    for glob_a in task_a_globs {
        for glob_b in task_b_globs {
            if globs_overlap_heuristic(glob_a, glob_b) {
                return true;
            }
        }
    }

    false
}

/// Conservative heuristic to check if a path might match a glob pattern.
///
/// This is a simple prefix/suffix check, not a full glob matcher.
fn path_matches_glob_heuristic(path: &str, glob: &str) -> bool {
    // Normalize paths for comparison
    let path_normalized = path.replace('\\', "/");
    let glob_normalized = glob.replace('\\', "/");

    // Handle common glob patterns
    if let Some(prefix) = glob_normalized.strip_suffix("/**") {
        // Directory glob: src/** matches anything under src/
        if path_normalized.starts_with(prefix)
            || path_normalized.starts_with(&format!("{}/", prefix))
        {
            return true;
        }
    }

    if let Some(prefix) = glob_normalized.strip_suffix("/*") {
        // Single-level glob: src/* matches direct children
        if let Some(path_prefix) = path_normalized.rsplit_once('/')
            && path_prefix.0 == prefix
        {
            return true;
        }
    }

    // Exact prefix match for simple cases
    if path_normalized.starts_with(&glob_normalized.replace("**", ""))
        || path_normalized.starts_with(&glob_normalized.replace("*", ""))
    {
        return true;
    }

    false
}

/// Check if two globs have overlapping coverage.
fn globs_overlap_heuristic(glob_a: &str, glob_b: &str) -> bool {
    let a_normalized = glob_a.replace('\\', "/");
    let b_normalized = glob_b.replace('\\', "/");

    // Extract the base directory from globs like "src/foo/**"
    let a_base = a_normalized
        .strip_suffix("/**")
        .or_else(|| a_normalized.strip_suffix("/*"))
        .unwrap_or(&a_normalized);

    let b_base = b_normalized
        .strip_suffix("/**")
        .or_else(|| b_normalized.strip_suffix("/*"))
        .unwrap_or(&b_normalized);

    // Check if one is a prefix of the other
    a_base.starts_with(b_base) || b_base.starts_with(a_base)
}

/// Check for scope conflicts with tasks currently in DOING.
fn check_scope_conflicts(
    _ctx: &crate::context::WorkflowContext,
    task: &TaskFile,
    index: &TaskIndex,
    policy: ConflictPolicy,
) -> Result<()> {
    if policy == ConflictPolicy::Ignore {
        return Ok(());
    }

    let doing_tasks = index.tasks_in_bucket("DOING");
    let mut conflicts: Vec<String> = Vec::new();

    let claiming_affects = &task.frontmatter.affects;
    let claiming_globs = &task.frontmatter.affects_globs;

    for doing_task in doing_tasks {
        let doing_file = TaskFile::load(&doing_task.path)?;

        if scopes_overlap(
            claiming_affects,
            claiming_globs,
            &doing_file.frontmatter.affects,
            &doing_file.frontmatter.affects_globs,
        ) {
            conflicts.push(format!(
                "{} ({})",
                doing_task.id, doing_file.frontmatter.title
            ));
        }
    }

    if conflicts.is_empty() {
        return Ok(());
    }

    let conflict_msg = format!(
        "scope conflict detected with tasks currently in DOING:\n  - {}\n\n\
         The declaring scopes overlap, which may cause merge conflicts.",
        conflicts.join("\n  - ")
    );

    match policy {
        ConflictPolicy::Fail => Err(BurlError::UserError(format!(
            "cannot claim task: {}\n\n\
             To proceed anyway, set `conflict_policy: warn` or `conflict_policy: ignore` in config.yaml.",
            conflict_msg
        ))),
        ConflictPolicy::Warn => {
            eprintln!("Warning: {}", conflict_msg);
            Ok(())
        }
        ConflictPolicy::Ignore => Ok(()),
    }
}

/// Information about a claim operation for rollback purposes.
struct ClaimTransaction {
    /// Whether the branch was created in this transaction (for rollback).
    branch_created: bool,
    /// The branch name.
    branch_name: String,
    /// The worktree info.
    worktree_info: Option<WorktreeInfo>,
}

impl ClaimTransaction {
    fn new() -> Self {
        Self {
            branch_created: false,
            branch_name: String::new(),
            worktree_info: None,
        }
    }

    /// Rollback the transaction by deleting the branch if it was created.
    fn rollback(self, repo_root: &std::path::Path) {
        if self.branch_created && !self.branch_name.is_empty() {
            // Try to delete the branch - ignore errors during rollback
            let _ = delete_branch(repo_root, &self.branch_name, true);
        }
    }
}

/// Execute the `burl claim` command.
///
/// Claims a READY task for work by:
/// 1. Selecting the task (explicit ID or next available)
/// 2. Checking dependencies and scope conflicts
/// 3. Creating/reusing branch and worktree
/// 4. Updating task metadata and moving to DOING
/// 5. Committing workflow state
pub fn cmd_claim(args: ClaimArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Build task index
    let index = TaskIndex::build(&ctx)?;

    // ========================================================================
    // Phase 1: Task Selection
    // ========================================================================

    // Acquire global claim lock if needed (when selecting next task)
    let _claim_lock: Option<LockGuard> = if args.task_id.is_none() && config.use_global_claim_lock {
        Some(acquire_claim_lock(&ctx)?)
    } else {
        None
    };

    let task_id = match &args.task_id {
        Some(id) => {
            // Explicit task ID provided
            validate_task_id(id)?
        }
        None => {
            // Select next available task
            let ready_tasks: Vec<&TaskInfo> = index.tasks_in_bucket("READY");
            select_next_task_id(&ctx, &ready_tasks)?.ok_or_else(|| {
                BurlError::UserError(
                    "no claimable tasks in READY.\n\n\
                     All READY tasks may have unmet dependencies, or there are no READY tasks.\n\
                     Use `burl status` to see the workflow state."
                        .to_string(),
                )
            })?
        }
    };

    // Re-lookup task info now that we have the ID
    let task_info = index.find(&task_id).ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' not found.\n\n\
             Use `burl status` to see available tasks.",
            task_id
        ))
    })?;

    // Verify task is in READY bucket
    if task_info.bucket != "READY" {
        return Err(BurlError::UserError(format!(
            "task '{}' is not in READY (currently in {}).\n\n\
             Only tasks in READY can be claimed.",
            task_info.id, task_info.bucket
        )));
    }

    // ========================================================================
    // Phase 2: Acquire per-task lock and load task file
    // ========================================================================

    let _task_lock = acquire_task_lock(&ctx, &task_info.id, "claim")?;

    let mut task_file = TaskFile::load(&task_info.path)?;

    // ========================================================================
    // Phase 3: Dependency and Scope Checks
    // ========================================================================

    // Rebuild index since we now have the lock
    let index = TaskIndex::build(&ctx)?;

    // Check dependencies
    check_dependencies_satisfied(&task_file, &index)?;

    // Check scope conflicts with DOING tasks
    check_scope_conflicts(&ctx, &task_file, &index, config.conflict_policy)?;

    // ========================================================================
    // Phase 4: Re-claim Check (after reject)
    // ========================================================================

    // If the task already has recorded branch/worktree values (from a prior claim/reject),
    // check if they still exist and are valid
    let existing_branch = task_file.frontmatter.branch.as_deref();
    let existing_worktree = task_file.frontmatter.worktree.as_deref();
    let existing_base_sha = task_file.frontmatter.base_sha.as_deref();

    let existing_git_refs = crate::task_git::validate_task_git_refs_if_present(
        &ctx,
        &task_info.id,
        existing_branch,
        existing_worktree,
    )?;
    let existing_worktree_for_setup = existing_git_refs
        .as_ref()
        .map(|r| r.worktree_path.to_string_lossy().to_string());

    // Check if this is a re-claim with existing state
    if let Some(refs) = &existing_git_refs {
        let branch = refs.branch.as_str();
        let worktree_path = &refs.worktree_path;

        // Check if worktree exists
        if worktree_path.exists() {
            // Verify it's on the correct branch
            if let Ok(current_branch) = crate::git_worktree::get_current_branch(worktree_path)
                && current_branch != branch
            {
                return Err(BurlError::UserError(format!(
                    "task has recorded worktree at '{}' but it's on branch '{}', not '{}'.\n\n\
                     Run `burl doctor` to diagnose and repair this inconsistency.",
                    worktree_path.display(),
                    current_branch,
                    branch
                )));
            }
        } else if branch_exists(&ctx.repo_root, branch)? {
            // Branch exists but worktree is missing
            return Err(BurlError::UserError(format!(
                "task has recorded branch '{}' but worktree at '{}' is missing.\n\n\
                 Run `burl doctor` to diagnose and repair this inconsistency,\n\
                 or manually recreate the worktree:\n  git worktree add {} {}",
                branch,
                worktree_path.display(),
                worktree_path.display(),
                branch
            )));
        }
    }

    // ========================================================================
    // Phase 5: Create/Reuse Branch and Worktree
    // ========================================================================

    let mut transaction = ClaimTransaction::new();

    // Check if branch already existed before setup
    let branch_existed_before = if let Some(branch) = existing_branch {
        branch_exists(&ctx.repo_root, branch)?
    } else {
        false
    };

    // Setup task worktree (handles fetch, base_sha, branch, worktree creation)
    let slug = slugify_title(&task_file.frontmatter.title);
    let worktree_info = match setup_task_worktree(
        &ctx,
        &task_info.id,
        Some(&slug),
        &config.remote,
        &config.main_branch,
        existing_branch,
        existing_worktree_for_setup.as_deref(),
    ) {
        Ok(info) => {
            transaction.branch_name = info.branch.clone();
            transaction.branch_created = !branch_existed_before && !info.reused;
            transaction.worktree_info = Some(info.clone());
            info
        }
        Err(e) => {
            // Rollback not needed if setup_task_worktree handles its own cleanup
            return Err(e);
        }
    };

    // Don't change base_sha on reuse (PRD policy)
    let base_sha = if worktree_info.reused {
        if let Some(sha) = existing_base_sha {
            sha.to_string()
        } else {
            worktree_info.base_sha.clone()
        }
    } else {
        worktree_info.base_sha.clone()
    };

    // ========================================================================
    // Phase 6: Workflow State Mutation (under workflow lock)
    // ========================================================================

    // Verify workflow worktree is clean before acquiring lock
    if let Err(e) = ctx.ensure_workflow_clean() {
        transaction.rollback(&ctx.repo_root);
        return Err(e);
    }

    // Acquire workflow lock
    let _workflow_lock = match acquire_workflow_lock(&ctx, "claim") {
        Ok(lock) => lock,
        Err(e) => {
            transaction.rollback(&ctx.repo_root);
            return Err(e);
        }
    };

    // Update task frontmatter
    let assignee = get_assignee_string();
    let now = Utc::now();

    task_file.set_assigned(&assignee, Some(now));
    task_file.set_git_info(
        &worktree_info.branch,
        &worktree_info.path.to_string_lossy(),
        &base_sha,
    );

    // Atomically write updated task file
    if let Err(e) = task_file.save(&task_info.path) {
        transaction.rollback(&ctx.repo_root);
        return Err(e);
    }

    // Move task file READY -> DOING atomically
    let filename = match task_info.path.file_name() {
        Some(name) => name,
        None => {
            transaction.rollback(&ctx.repo_root);
            return Err(BurlError::UserError("invalid task file path".to_string()));
        }
    };
    let doing_path = ctx.bucket_path("DOING").join(filename);

    // Move task file into DOING.
    if let Err(e) = crate::fs::move_file(&task_info.path, &doing_path) {
        transaction.rollback(&ctx.repo_root);
        return Err(BurlError::UserError(format!(
            "failed to move task from READY to DOING: {}\n\n\
             Task file: {}\n\
             Destination: {}",
            e,
            task_info.path.display(),
            doing_path.display()
        )));
    }

    // ========================================================================
    // Phase 7: Event Logging and Commit
    // ========================================================================

    // Append claim event
    let event = Event::new(EventAction::Claim)
        .with_task(&task_info.id)
        .with_details(json!({
            "title": task_file.frontmatter.title,
            "branch": worktree_info.branch,
            "worktree": worktree_info.path.to_string_lossy(),
            "base_sha": base_sha,
            "reused": worktree_info.reused,
            "assigned_to": assignee
        }));
    append_event(&ctx, &event)?;

    // Commit workflow state if auto-commit enabled
    if config.workflow_auto_commit {
        commit_claim(&ctx, &task_info.id, &task_file.frontmatter.title)?;

        // Push if auto-push enabled
        if config.workflow_auto_push {
            push_workflow_branch(&ctx, &config)?;
        }
    }

    // ========================================================================
    // Phase 8: Output
    // ========================================================================

    // Print worktree path for agents to cd into
    println!("{}", worktree_info.path.display());

    // Print additional info to stderr so it doesn't interfere with scripted use
    eprintln!();
    eprintln!("Claimed task: {}", task_info.id);
    eprintln!("  Title:    {}", task_file.frontmatter.title);
    eprintln!("  Branch:   {}", worktree_info.branch);
    eprintln!("  Base SHA: {}", base_sha);
    if worktree_info.reused {
        eprintln!("  (reused existing worktree)");
    }
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. cd {}", worktree_info.path.display());
    eprintln!("  2. Make your changes");
    eprintln!("  3. Run `burl submit {}` when ready for QA", task_info.id);

    Ok(())
}

/// Get the assignee string for task metadata.
fn get_assignee_string() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!("{}@{}", user, host)
}

/// Commit the claim to the workflow branch.
fn commit_claim(ctx: &crate::context::WorkflowContext, task_id: &str, title: &str) -> Result<()> {
    // Stage all changes in the workflow worktree
    run_git(&ctx.workflow_worktree, &["add", "."])
        .map_err(|e| BurlError::GitError(format!("failed to stage claim changes: {}", e)))?;

    // Check if anything was staged
    let staged = run_git(&ctx.workflow_worktree, &["diff", "--cached", "--name-only"])?;
    if staged.stdout.is_empty() {
        return Ok(());
    }

    // Create commit message
    let commit_msg = format!("Claim task {}: {}", task_id, title);

    run_git(&ctx.workflow_worktree, &["commit", "-m", &commit_msg])
        .map_err(|e| BurlError::GitError(format!("failed to commit claim: {}", e)))?;

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
    use crate::commands::add::cmd_add;
    use crate::commands::init::cmd_init;
    use crate::test_support::{DirGuard, create_test_repo_with_remote};
    use serial_test::serial;

    #[test]
    fn test_priority_rank() {
        assert_eq!(priority_rank("high"), 0);
        assert_eq!(priority_rank("HIGH"), 0);
        assert_eq!(priority_rank("medium"), 1);
        assert_eq!(priority_rank("low"), 2);
        assert_eq!(priority_rank("other"), 3);
        assert_eq!(priority_rank(""), 3);
    }

    #[test]
    fn test_scopes_overlap_identical_affects() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec![];
        let affects_b = vec!["src/lib.rs".to_string()];
        let globs_b = vec![];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_identical_globs() {
        let affects_a = vec![];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec![];
        let globs_b = vec!["src/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_path_matches_glob() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec![];
        let affects_b = vec![];
        let globs_b = vec!["src/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_no_overlap() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec!["tests/test.rs".to_string()];
        let globs_b = vec!["tests/**".to_string()];

        assert!(!scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_nested_globs() {
        let affects_a = vec![];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec![];
        let globs_b = vec!["src/foo/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_path_matches_glob_heuristic() {
        assert!(path_matches_glob_heuristic("src/lib.rs", "src/**"));
        assert!(path_matches_glob_heuristic("src/foo/bar.rs", "src/**"));
        assert!(path_matches_glob_heuristic("src/lib.rs", "src/*"));
        assert!(!path_matches_glob_heuristic("tests/test.rs", "src/**"));
    }

    #[test]
    fn test_globs_overlap_heuristic() {
        assert!(globs_overlap_heuristic("src/**", "src/foo/**"));
        assert!(globs_overlap_heuristic("src/foo/**", "src/**"));
        assert!(!globs_overlap_heuristic("src/**", "tests/**"));
    }

    #[test]
    #[serial]
    fn test_claim_explicit_task() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task
        let add_args = AddArgs {
            title: "Test claim task".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/lib.rs".to_string()],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        cmd_add(add_args).unwrap();

        // Claim the task
        let claim_args = ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        };
        cmd_claim(claim_args).unwrap();

        // Verify task moved to DOING
        let doing_path = temp_dir
            .path()
            .join(".burl/.workflow/DOING/TASK-001-test-claim-task.md");
        assert!(doing_path.exists(), "Task should be in DOING bucket");

        // Verify task file has claim metadata
        let task = TaskFile::load(&doing_path).unwrap();
        assert!(task.frontmatter.assigned_to.is_some());
        assert!(task.frontmatter.started_at.is_some());
        assert!(task.frontmatter.branch.is_some());
        assert!(task.frontmatter.worktree.is_some());
        assert!(task.frontmatter.base_sha.is_some());

        // Verify worktree was created
        let worktree_path = temp_dir.path().join(".worktrees/task-001-test-claim-task");
        assert!(
            worktree_path.exists(),
            "Worktree should exist at {:?}",
            worktree_path
        );
    }

    #[test]
    #[serial]
    fn test_claim_next_task_deterministic() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add tasks with different priorities
        cmd_add(AddArgs {
            title: "Low priority task".to_string(),
            priority: "low".to_string(),
            affects: vec!["tests/a.rs".to_string()],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        cmd_add(AddArgs {
            title: "High priority task".to_string(),
            priority: "high".to_string(),
            affects: vec!["tests/b.rs".to_string()],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        cmd_add(AddArgs {
            title: "Medium priority task".to_string(),
            priority: "medium".to_string(),
            affects: vec!["tests/c.rs".to_string()],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Claim without task ID - should pick high priority first
        let claim_args = ClaimArgs { task_id: None };
        cmd_claim(claim_args).unwrap();

        // TASK-002 (high priority) should be in DOING
        let doing_path = temp_dir
            .path()
            .join(".burl/.workflow/DOING/TASK-002-high-priority-task.md");
        assert!(
            doing_path.exists(),
            "High priority task should be claimed first"
        );
    }

    #[test]
    #[serial]
    fn test_claim_fails_for_non_ready_task() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add and claim a task
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

        // First claim should succeed
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Second claim should fail (task is now in DOING)
        let result = cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in READY"));
    }

    #[test]
    #[serial]
    fn test_claim_fails_with_unmet_dependencies() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add task with dependency
        cmd_add(AddArgs {
            title: "Dependent task".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec!["TASK-002".to_string()], // Depends on non-existent task
            tags: vec![],
        })
        .unwrap();

        // Claim should fail due to unmet dependency
        let result = cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        });

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("dependencies not satisfied")
        );
    }

    #[test]
    #[serial]
    fn test_claim_with_scope_conflict_fails_by_default() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add two tasks with overlapping scope
        cmd_add(AddArgs {
            title: "First task".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/lib.rs".to_string()],
            affects_globs: vec!["src/**".to_string()],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        cmd_add(AddArgs {
            title: "Second task".to_string(),
            priority: "high".to_string(),
            affects: vec!["src/main.rs".to_string()],
            affects_globs: vec!["src/**".to_string()],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        })
        .unwrap();

        // Claim first task
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Claim second task should fail due to scope conflict
        let result = cmd_claim(ClaimArgs {
            task_id: Some("TASK-002".to_string()),
        });

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("scope conflict") || err_msg.contains("conflict"),
            "Error should mention scope conflict: {}",
            err_msg
        );
    }

    #[test]
    #[serial]
    fn test_claim_base_sha_is_set() {
        let temp_dir = create_test_repo_with_remote();
        let _guard = DirGuard::new(temp_dir.path());

        // Initialize workflow
        cmd_init().unwrap();

        // Add a task
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

        // Claim the task
        cmd_claim(ClaimArgs {
            task_id: Some("TASK-001".to_string()),
        })
        .unwrap();

        // Verify base_sha was set
        let doing_path = temp_dir
            .path()
            .join(".burl/.workflow/DOING/TASK-001-test-task.md");
        let task = TaskFile::load(&doing_path).unwrap();

        assert!(task.frontmatter.base_sha.is_some());
        let base_sha = task.frontmatter.base_sha.unwrap();
        assert_eq!(base_sha.len(), 40, "base_sha should be a full SHA");
    }
}
