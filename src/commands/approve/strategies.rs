//! Merge strategy implementations for the approve command.
//!
//! This module implements different strategies for merging approved tasks:
//! - rebase_ff_only: Rebase onto origin/main, then fast-forward merge
//! - ff_only: Fast-forward merge without rebasing

use crate::config::Config;
use crate::error::{BurlError, Result};
use crate::git::run_git;
use crate::task::TaskFile;
use std::path::PathBuf;

use super::git_ops::{cleanup_worktree, complete_approval, merge_ff_only, push_main, reject_task};
use super::validation::{format_validation_summary, run_validation};

/// Approve using rebase_ff_only strategy (default).
pub fn approve_rebase_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    worktree_path: &PathBuf,
    branch: &str,
) -> Result<()> {
    let remote_main = format!("{}/{}", config.remote, config.main_branch);

    // Step 1: Fetch origin/main
    println!("Fetching {}/{}...", config.remote, config.main_branch);
    run_git(
        &ctx.repo_root,
        &["fetch", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to fetch {}/{}: {}",
            config.remote, config.main_branch, e
        ))
    })?;

    // Step 2: Rebase task branch onto origin/main in worktree
    println!("Rebasing {} onto {}...", branch, remote_main);
    let rebase_result = run_git(worktree_path, &["rebase", &remote_main]);

    if let Err(e) = rebase_result {
        // Abort the rebase to leave the worktree in a clean state
        let _ = run_git(worktree_path, &["rebase", "--abort"]);

        // Reject the task
        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("rebase conflict: {}", e),
        );
    }

    // Step 3: Run validation against rebased base (origin/main..HEAD)
    println!("Running validation...");
    let validation_result = run_validation(ctx, config, task_file, worktree_path, &remote_main)?;

    if !validation_result.all_passed {
        // Append validation report before rejecting
        let summary = format_validation_summary(&validation_result.results, false);
        task_file.append_to_qa_report(&summary);

        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            "validation failed after rebase",
        );
    }

    // Step 4: Merge into local main using --ff-only
    println!("Merging {} into local main...", branch);
    merge_ff_only(ctx, config, task_id, task_path, task_file, branch)?;

    // Step 5: Optional push
    if config.push_main_on_approve {
        println!("Pushing main to {}...", config.remote);
        push_main(ctx, config)?;
    }

    // Step 6: Cleanup worktree and branch (best-effort)
    println!("Cleaning up worktree and branch...");
    let cleanup_failed = cleanup_worktree(ctx, branch, worktree_path)?;

    // Step 7: Workflow state mutation
    complete_approval(ctx, config, task_id, task_path, task_file, cleanup_failed)?;

    println!();
    println!("Approved task: {}", task_id);
    println!("  Title:     {}", task_file.frontmatter.title);
    println!("  From:      QA");
    println!("  To:        DONE");
    println!("  Branch:    {} (merged to {})", branch, config.main_branch);
    if cleanup_failed {
        println!("  Cleanup:   Failed (run `burl clean` to remove leftovers)");
    } else {
        println!("  Cleanup:   Complete");
    }
    if config.push_main_on_approve {
        println!(
            "  Pushed:    {} -> {}/{}",
            config.main_branch, config.remote, config.main_branch
        );
    }

    Ok(())
}

/// Approve using ff_only strategy (skip rebase).
pub fn approve_ff_only(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    task_id: &str,
    task_path: &std::path::Path,
    task_file: &mut TaskFile,
    worktree_path: &PathBuf,
    branch: &str,
) -> Result<()> {
    let remote_main = format!("{}/{}", config.remote, config.main_branch);

    // Step 1: Fetch origin/main
    println!("Fetching {}/{}...", config.remote, config.main_branch);
    run_git(
        &ctx.repo_root,
        &["fetch", &config.remote, &config.main_branch],
    )
    .map_err(|e| {
        BurlError::GitError(format!(
            "failed to fetch {}/{}: {}",
            config.remote, config.main_branch, e
        ))
    })?;

    // Step 2: Verify task branch is descendant of origin/main
    println!("Verifying branch is up-to-date with {}...", remote_main);
    let is_ancestor = run_git(
        worktree_path,
        &["merge-base", "--is-ancestor", &remote_main, "HEAD"],
    );

    if is_ancestor.is_err() {
        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            &format!("branch behind {}; rebase required", remote_main),
        );
    }

    // Step 3: Run validation against origin/main..HEAD
    println!("Running validation...");
    let validation_result = run_validation(ctx, config, task_file, worktree_path, &remote_main)?;

    if !validation_result.all_passed {
        let summary = format_validation_summary(&validation_result.results, false);
        task_file.append_to_qa_report(&summary);

        return reject_task(
            ctx,
            config,
            task_id,
            task_path,
            task_file,
            "validation failed",
        );
    }

    // Step 4: Optionally fast-forward local main to origin/main first
    // This is recommended to ensure we have the latest main
    let _ = run_git(
        &ctx.repo_root,
        &[
            "fetch",
            &config.remote,
            &format!("{}:{}", config.main_branch, config.main_branch),
        ],
    );

    // Step 5: Merge into local main using --ff-only
    println!("Merging {} into local main...", branch);
    merge_ff_only(ctx, config, task_id, task_path, task_file, branch)?;

    // Step 6: Optional push
    if config.push_main_on_approve {
        println!("Pushing main to {}...", config.remote);
        push_main(ctx, config)?;
    }

    // Step 7: Cleanup worktree and branch (best-effort)
    println!("Cleaning up worktree and branch...");
    let cleanup_failed = cleanup_worktree(ctx, branch, worktree_path)?;

    // Step 8: Workflow state mutation
    complete_approval(ctx, config, task_id, task_path, task_file, cleanup_failed)?;

    println!();
    println!("Approved task: {}", task_id);
    println!("  Title:     {}", task_file.frontmatter.title);
    println!("  From:      QA");
    println!("  To:        DONE");
    println!("  Branch:    {} (merged to {})", branch, config.main_branch);
    if cleanup_failed {
        println!("  Cleanup:   Failed (run `burl clean` to remove leftovers)");
    } else {
        println!("  Cleanup:   Complete");
    }
    if config.push_main_on_approve {
        println!(
            "  Pushed:    {} -> {}/{}",
            config.main_branch, config.remote, config.main_branch
        );
    }

    Ok(())
}
