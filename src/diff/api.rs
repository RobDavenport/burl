//! Public API for diff parsing.

use crate::error::Result;
use crate::git::run_git;
use std::path::Path;

use super::helpers::normalize_path;
use super::parser::parse_added_lines_from_diff;

/// Represents a single added line from a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    /// Repository-relative file path (forward slashes).
    pub file_path: String,
    /// Line number in the new file (1-based).
    pub line_number: usize,
    /// The content of the added line (without leading '+').
    pub content: String,
}

/// Get the list of changed files between two commits.
///
/// Runs `git diff --name-only {base}..HEAD` and returns repo-relative
/// file paths with forward slashes.
///
/// # Arguments
///
/// * `cwd` - The working directory (should be the task worktree or repo root)
/// * `base_sha` - The base commit SHA to diff against
///
/// # Returns
///
/// * `Ok(Vec<String>)` - List of changed file paths (repo-relative, forward slashes)
/// * `Err(BurlError::GitError)` - Git command failed
pub fn changed_files<P: AsRef<Path>>(cwd: P, base_sha: &str) -> Result<Vec<String>> {
    let diff_range = format!("{}..HEAD", base_sha);
    let output = run_git(&cwd, &["diff", "--name-only", &diff_range])?;

    if output.is_empty() {
        return Ok(Vec::new());
    }

    // Normalize paths to forward slashes for glob matching
    let files: Vec<String> = output.lines().into_iter().map(normalize_path).collect();

    Ok(files)
}

/// Parse added lines from a unified diff output.
///
/// Parses the output of `git diff -U0 {base}..HEAD` to extract only
/// the lines that were added (+...) with their file path and line number.
///
/// # Arguments
///
/// * `cwd` - The working directory (should be the task worktree or repo root)
/// * `base_sha` - The base commit SHA to diff against
///
/// # Returns
///
/// * `Ok(Vec<AddedLine>)` - List of added lines with file paths and line numbers
/// * `Err(BurlError::GitError)` - Git command failed
pub fn added_lines<P: AsRef<Path>>(cwd: P, base_sha: &str) -> Result<Vec<AddedLine>> {
    let diff_range = format!("{}..HEAD", base_sha);
    let output = run_git(&cwd, &["diff", "-U0", &diff_range])?;

    parse_added_lines_from_diff(&output.stdout)
}
