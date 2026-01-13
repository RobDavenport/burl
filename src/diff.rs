//! Diff parsing primitives for burl.
//!
//! This module provides utilities for parsing git diff output to support:
//! - `submit` validation (scope checking)
//! - `validate` stub detection (added lines only)
//! - `approve` validation after rebase
//!
//! The parsing is deterministic and supports:
//! - Changed files list from `git diff --name-only {base}..HEAD`
//! - Added lines with line numbers from `git diff -U0 {base}..HEAD`
//! - New files (from /dev/null)
//! - File renames (best-effort line mapping)
//! - Proper hunk header parsing for accurate line numbers

use crate::error::Result;
use crate::git::run_git;
use std::path::Path;

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

/// Parse the file path from a "diff --git" line.
///
/// Handles various formats:
/// - "a/path/to/file b/path/to/file" (normal)
/// - "a/path/to/file b/path/to/renamed" (rename)
/// - "a/path b/path" (short paths)
///
/// Returns the "b/" path (new file path), or None if parsing fails.
fn parse_diff_git_line(rest: &str) -> Option<String> {
    // The format is: "a/<path> b/<path>"
    // But paths can contain spaces, so we need to be careful
    // Strategy: find " b/" which separates the two paths

    // Handle the case where the path might contain " b/" as part of the path
    // by looking for the last " b/" occurrence
    if let Some(b_pos) = rest.rfind(" b/") {
        let b_path = &rest[b_pos + 3..]; // Skip " b/"
        return Some(normalize_path(b_path));
    }

    // Fallback: try to split on space and take the second part
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        let b_part = parts[parts.len() - 1];
        if let Some(path) = b_part.strip_prefix("b/") {
            return Some(normalize_path(path));
        }
    }

    None
}

/// Parse a hunk header line.
///
/// Format: "@@ -old_start,old_len +new_start,new_len @@" or "@@ -old_start +new_start @@"
/// Also handles: "@@ -old_start,old_len +new_start,new_len @@ context info"
///
/// Returns (old_start, new_start) or None if parsing fails.
fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    // Remove leading "@@ " and trailing " @@" (with optional context)
    let line = line.strip_prefix("@@ ")?;

    // Find the closing " @@"
    let end_marker = line.find(" @@")?;
    let range_part = &line[..end_marker];

    // Split into old and new ranges
    let parts: Vec<&str> = range_part.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let old_part = parts[0].strip_prefix('-')?;
    let new_part = parts[1].strip_prefix('+')?;

    let old_start = parse_range_start(old_part)?;
    let new_start = parse_range_start(new_part)?;

    Some((old_start, new_start))
}

/// Parse the start line from a range specification.
///
/// Format: "start" or "start,len"
/// Returns the start line number.
fn parse_range_start(range: &str) -> Option<usize> {
    let start_str = if let Some(comma_pos) = range.find(',') {
        &range[..comma_pos]
    } else {
        range
    };

    start_str.parse().ok()
}

/// Normalize a file path to use forward slashes.
///
/// This ensures consistent path format for glob matching,
/// regardless of the platform where the diff was generated.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test parsing a simple diff with one file and added lines.
    #[test]
    fn test_parse_simple_added_lines() {
        let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +11,2 @@ fn existing_function() {
+    let x = 42;
+    println!("Added line");
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].file_path, "src/lib.rs");
        assert_eq!(result[0].line_number, 11);
        assert_eq!(result[0].content, "    let x = 42;");
        assert_eq!(result[1].line_number, 12);
        assert_eq!(result[1].content, "    println!(\"Added line\");");
    }

    /// Test parsing a new file (source is /dev/null).
    #[test]
    fn test_parse_new_file() {
        let diff = r#"diff --git a/src/new_file.rs b/src/new_file.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/src/new_file.rs
@@ -0,0 +1,3 @@
+//! New module
+
+pub fn hello() {}
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].file_path, "src/new_file.rs");
        assert_eq!(result[0].line_number, 1);
        assert_eq!(result[0].content, "//! New module");
        assert_eq!(result[1].line_number, 2);
        assert_eq!(result[1].content, "");
        assert_eq!(result[2].line_number, 3);
        assert_eq!(result[2].content, "pub fn hello() {}");
    }

    /// Test parsing multiple hunks in one file.
    #[test]
    fn test_parse_multiple_hunks() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -5,0 +6,1 @@ fn main() {
+    // First addition at line 6
@@ -20,0 +22,1 @@ fn helper() {
+    // Second addition at line 22
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].line_number, 6);
        assert_eq!(result[0].content, "    // First addition at line 6");
        assert_eq!(result[1].line_number, 22);
        assert_eq!(result[1].content, "    // Second addition at line 22");
    }

    /// Test parsing a hunk with both additions and deletions.
    #[test]
    fn test_parse_mixed_hunk() {
        let diff = r#"diff --git a/src/config.rs b/src/config.rs
index abc1234..def5678 100644
--- a/src/config.rs
+++ b/src/config.rs
@@ -10,2 +10,3 @@ struct Config {
-    old_field: i32,
-    another_old: String,
+    new_field: i64,
+    another_new: String,
+    extra_field: bool,
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // Only added lines should be captured
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].file_path, "src/config.rs");
        assert_eq!(result[0].line_number, 10);
        assert_eq!(result[0].content, "    new_field: i64,");
        assert_eq!(result[1].line_number, 11);
        assert_eq!(result[1].content, "    another_new: String,");
        assert_eq!(result[2].line_number, 12);
        assert_eq!(result[2].content, "    extra_field: bool,");
    }

    /// Test that removed lines are not captured.
    #[test]
    fn test_removed_lines_not_captured() {
        let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5,2 +5,0 @@ fn main() {
-    let x = 1;
-    let y = 2;
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // No added lines
        assert!(result.is_empty());
    }

    /// Test parsing multiple files.
    #[test]
    fn test_parse_multiple_files() {
        let diff = r#"diff --git a/src/first.rs b/src/first.rs
index abc1234..def5678 100644
--- a/src/first.rs
+++ b/src/first.rs
@@ -1,0 +2,1 @@
+// Added to first.rs
diff --git a/src/second.rs b/src/second.rs
index 111111..222222 100644
--- a/src/second.rs
+++ b/src/second.rs
@@ -5,0 +6,1 @@
+// Added to second.rs
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].file_path, "src/first.rs");
        assert_eq!(result[0].line_number, 2);
        assert_eq!(result[0].content, "// Added to first.rs");
        assert_eq!(result[1].file_path, "src/second.rs");
        assert_eq!(result[1].line_number, 6);
        assert_eq!(result[1].content, "// Added to second.rs");
    }

    /// Test parsing a file rename.
    #[test]
    fn test_parse_rename() {
        let diff = r#"diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 95%
rename from src/old_name.rs
rename to src/new_name.rs
index abc1234..def5678 100644
--- a/src/old_name.rs
+++ b/src/new_name.rs
@@ -10,0 +11,1 @@
+// Added line in renamed file
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 1);
        // Should use the new file name
        assert_eq!(result[0].file_path, "src/new_name.rs");
        assert_eq!(result[0].line_number, 11);
    }

    /// Test parsing hunk headers with various formats.
    #[test]
    fn test_parse_hunk_header_formats() {
        // Standard format with lengths
        assert_eq!(parse_hunk_header("@@ -10,5 +20,3 @@"), Some((10, 20)));

        // Without lengths (single line change)
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1)));

        // With context info after @@
        assert_eq!(parse_hunk_header("@@ -10,5 +20,3 @@ fn foo()"), Some((10, 20)));

        // Zero-length addition (insertion)
        assert_eq!(parse_hunk_header("@@ -5,0 +6,2 @@"), Some((5, 6)));

        // Line 0 (new file, no prior content)
        assert_eq!(parse_hunk_header("@@ -0,0 +1,10 @@"), Some((0, 1)));
    }

    /// Test normalize_path converts backslashes.
    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src\\lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src\\nested\\file.rs"), "src/nested/file.rs");
    }

    /// Test empty diff returns empty results.
    #[test]
    fn test_empty_diff() {
        let result = parse_added_lines_from_diff("").unwrap();
        assert!(result.is_empty());
    }

    /// Test diff with only metadata lines (no actual changes).
    #[test]
    fn test_diff_metadata_only() {
        let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();
        assert!(result.is_empty());
    }

    /// Test comprehensive fixture combining multiple scenarios.
    #[test]
    fn test_comprehensive_fixture() {
        let diff = r#"diff --git a/src/player/jump.rs b/src/player/jump.rs
index abc1234..def5678 100644
--- a/src/player/jump.rs
+++ b/src/player/jump.rs
@@ -15,2 +15,3 @@ impl Player {
-    fn old_jump(&mut self) {
-        // old implementation
+    fn jump(&mut self) {
+        self.velocity.y = JUMP_FORCE;
+        // TODO: implement cooldown
@@ -30,0 +32,1 @@ impl Player {
+    unimplemented!()
diff --git a/src/player/mod.rs b/src/player/mod.rs
new file mode 100644
index 0000000..111111
--- /dev/null
+++ b/src/player/mod.rs
@@ -0,0 +1,5 @@
+//! Player module
+
+mod jump;
+
+pub use jump::Player;
diff --git a/src/config.rs b/src/config.rs
index 222222..333333 100644
--- a/src/config.rs
+++ b/src/config.rs
@@ -5,1 +5,1 @@ const MAX_PLAYERS: usize = 4;
-const JUMP_FORCE: f32 = 10.0;
+const JUMP_FORCE: f32 = 15.0;
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // Count added lines per file
        let jump_lines: Vec<_> = result.iter().filter(|l| l.file_path == "src/player/jump.rs").collect();
        let mod_lines: Vec<_> = result.iter().filter(|l| l.file_path == "src/player/mod.rs").collect();
        let config_lines: Vec<_> = result.iter().filter(|l| l.file_path == "src/config.rs").collect();

        // jump.rs: 3 lines in first hunk, 1 in second
        assert_eq!(jump_lines.len(), 4);
        assert_eq!(jump_lines[0].line_number, 15);
        assert_eq!(jump_lines[0].content, "    fn jump(&mut self) {");
        assert_eq!(jump_lines[2].content, "        // TODO: implement cooldown");
        assert_eq!(jump_lines[3].line_number, 32);
        assert_eq!(jump_lines[3].content, "    unimplemented!()");

        // mod.rs: 5 lines (new file)
        assert_eq!(mod_lines.len(), 5);
        assert_eq!(mod_lines[0].line_number, 1);
        assert_eq!(mod_lines[0].content, "//! Player module");

        // config.rs: 1 added line (replacement)
        assert_eq!(config_lines.len(), 1);
        assert_eq!(config_lines[0].line_number, 5);
        assert_eq!(config_lines[0].content, "const JUMP_FORCE: f32 = 15.0;");
    }

    /// Test context lines are handled (rare with -U0, but should work).
    #[test]
    fn test_context_lines() {
        // With -U0 we shouldn't see context, but if present, handle it
        let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5,3 +5,4 @@ fn main() {
 // context line
+// added line
 // another context
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // The added line should be at line 6 (after context at 5)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line_number, 6);
        assert_eq!(result[0].content, "// added line");
    }

    /// Test deleted file (destination is /dev/null) produces no added lines.
    #[test]
    fn test_deleted_file() {
        let diff = r#"diff --git a/src/deleted.rs b/src/deleted.rs
deleted file mode 100644
index abc1234..0000000
--- a/src/deleted.rs
+++ /dev/null
@@ -1,5 +0,0 @@
-//! This file is deleted
-
-pub fn old_function() {
-    // old code
-}
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // No added lines from deleted file
        assert!(result.is_empty());
    }

    /// Test file path with spaces.
    #[test]
    fn test_file_path_with_spaces() {
        let diff = r#"diff --git a/src/my file.rs b/src/my file.rs
index abc1234..def5678 100644
--- a/src/my file.rs
+++ b/src/my file.rs
@@ -1,0 +2,1 @@
+// Added line
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/my file.rs");
    }

    /// Test binary file (should produce no added lines).
    #[test]
    fn test_binary_file() {
        let diff = r#"diff --git a/assets/image.png b/assets/image.png
new file mode 100644
index 0000000..abc1234
Binary files /dev/null and b/assets/image.png differ
"#;

        let result = parse_added_lines_from_diff(diff).unwrap();

        // Binary files don't have text content
        assert!(result.is_empty());
    }

    /// Test AddedLine struct equality.
    #[test]
    fn test_added_line_equality() {
        let line1 = AddedLine {
            file_path: "src/lib.rs".to_string(),
            line_number: 10,
            content: "let x = 1;".to_string(),
        };
        let line2 = AddedLine {
            file_path: "src/lib.rs".to_string(),
            line_number: 10,
            content: "let x = 1;".to_string(),
        };
        let line3 = AddedLine {
            file_path: "src/lib.rs".to_string(),
            line_number: 11,
            content: "let x = 1;".to_string(),
        };

        assert_eq!(line1, line2);
        assert_ne!(line1, line3);
    }

    /// Integration test: parse diff from git command.
    /// This test requires a real git repo, so it uses tempfile.
    #[test]
    fn test_integration_with_git() {
        use std::process::Command;
        use tempfile::TempDir;

        // Create a temporary git repository
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        // Configure git user
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        // Create initial file and commit
        std::fs::write(path.join("test.rs"), "fn main() {}\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        // Get the base SHA
        let base_output = Command::new("git")
            .current_dir(path)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("failed to get HEAD");
        let base_sha = String::from_utf8_lossy(&base_output.stdout).trim().to_string();

        // Modify the file and create new file
        std::fs::write(path.join("test.rs"), "fn main() {\n    let x = 42;\n}\n").unwrap();
        std::fs::write(path.join("new.rs"), "// New file\npub fn hello() {}\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Add changes"])
            .output()
            .expect("failed to commit");

        // Test changed_files
        let files = changed_files(path, &base_sha).unwrap();
        assert!(files.contains(&"test.rs".to_string()));
        assert!(files.contains(&"new.rs".to_string()));

        // Test added_lines
        let lines = added_lines(path, &base_sha).unwrap();

        // Should have lines from both files
        let test_lines: Vec<_> = lines.iter().filter(|l| l.file_path == "test.rs").collect();
        let new_lines: Vec<_> = lines.iter().filter(|l| l.file_path == "new.rs").collect();

        assert!(!test_lines.is_empty(), "Should have added lines in test.rs");
        assert!(!new_lines.is_empty(), "Should have added lines in new.rs");

        // Verify new.rs has expected content
        assert!(new_lines.iter().any(|l| l.content == "// New file"));
        assert!(new_lines.iter().any(|l| l.content == "pub fn hello() {}"));
    }
}
