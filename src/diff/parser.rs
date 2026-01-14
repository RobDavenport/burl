//! Core diff parsing logic.

use crate::error::Result;

use super::api::AddedLine;
use super::helpers::{normalize_path, parse_diff_git_line, parse_hunk_header};

/// Parse added lines from raw diff output string.
///
/// This is the core parsing function that can be used directly with
/// diff output strings (useful for testing).
///
/// # Arguments
///
/// * `diff_output` - Raw unified diff output with -U0 (no context)
///
/// # Returns
///
/// * `Ok(Vec<AddedLine>)` - List of added lines with file paths and line numbers
/// * `Err(BurlError::UserError)` - Invalid diff format
pub fn parse_added_lines_from_diff(diff_output: &str) -> Result<Vec<AddedLine>> {
    let mut result = Vec::new();
    let mut current_file: Option<String> = None;
    let mut new_line: usize = 0; // Current line number in new file

    for line in diff_output.lines() {
        // Check for diff header to get file path
        // Format: "diff --git a/path/to/file b/path/to/file"
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // Extract the b/ path (the "new" file)
            current_file = parse_diff_git_line(rest);
            new_line = 0;
            continue;
        }

        // Handle new file indicator (source is /dev/null)
        // Format: "--- /dev/null" or "--- a/path/to/file"
        if line.starts_with("--- ") {
            // We already have the file from "diff --git", continue
            continue;
        }

        // Handle destination file indicator
        // Format: "+++ b/path/to/file" or "+++ /dev/null"
        if let Some(rest) = line.strip_prefix("+++ ") {
            if rest == "/dev/null" {
                // File was deleted, skip
                current_file = None;
            } else if let Some(path) = rest.strip_prefix("b/") {
                // Update current file (handles renames where diff --git might be confusing)
                current_file = Some(normalize_path(path));
            }
            continue;
        }

        // Parse hunk header
        // Format: "@@ -old_start,old_len +new_start,new_len @@" or "@@ -old_start +new_start @@"
        // Also: "@@ -old_start,old_len +new_start,new_len @@ optional context"
        if line.starts_with("@@ ") {
            if let Some((_, new_start)) = parse_hunk_header(line) {
                new_line = new_start;
            }
            continue;
        }

        // Parse diff lines
        if let Some(file) = &current_file {
            if let Some(content) = line.strip_prefix('+') {
                // Added line
                result.push(AddedLine {
                    file_path: file.clone(),
                    line_number: new_line,
                    content: content.to_string(),
                });
                new_line += 1;
            } else if line.starts_with('-') {
                // Removed line - don't increment new_line
                // (old_line would increment, but we don't track it)
            } else if line.starts_with(' ') {
                // Context line (rare with -U0, but handle anyway)
                new_line += 1;
            }
            // Ignore other lines (empty lines between hunks, etc.)
        }
    }

    Ok(result)
}
