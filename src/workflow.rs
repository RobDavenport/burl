//! Workflow operations and task index for burl.
//!
//! This module provides:
//! - Task index: enumerate buckets and map task IDs to file paths
//! - Bucket operations: list tasks, find tasks, move tasks between buckets
//! - Task ID validation and generation
//! - Title slugification for task filenames

use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// All workflow buckets in order.
pub const BUCKETS: &[&str] = &["READY", "DOING", "QA", "DONE", "BLOCKED"];

/// Regex pattern for valid task IDs.
static TASK_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^TASK-\d{3,}$").expect("Invalid task ID regex"));

/// Information about a task in the workflow.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    /// The task ID (e.g., "TASK-001").
    pub id: String,

    /// The bucket the task is in (e.g., "READY", "DOING").
    pub bucket: String,

    /// The full path to the task file.
    pub path: PathBuf,

    /// The numeric part of the task ID.
    pub number: u32,
}

/// Index of all tasks in the workflow.
#[derive(Debug, Default)]
pub struct TaskIndex {
    /// Map of task ID to task info.
    tasks: HashMap<String, TaskInfo>,

    /// Maximum task number seen (for generating new IDs).
    max_number: u32,
}

impl TaskIndex {
    /// Build a task index by scanning all buckets.
    ///
    /// This function scans all bucket directories for task files matching
    /// the pattern `TASK-{id}-{slug}.md`.
    pub fn build(ctx: &WorkflowContext) -> Result<Self> {
        let mut index = TaskIndex::default();

        for bucket in BUCKETS {
            let bucket_path = ctx.bucket_path(bucket);
            if !bucket_path.exists() {
                continue;
            }

            let entries = fs::read_dir(&bucket_path).map_err(|e| {
                BurlError::UserError(format!(
                    "failed to read bucket directory '{}': {}",
                    bucket_path.display(),
                    e
                ))
            })?;

            for entry in entries {
                let entry = entry.map_err(|e| {
                    BurlError::UserError(format!("failed to read directory entry: {}", e))
                })?;

                let path = entry.path();

                // Skip non-markdown files and non-task files
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                // Extract task ID from filename
                if let Some(task_id) = extract_task_id_from_filename(&path)
                    && let Some(number) = extract_task_number(&task_id)
                {
                    index.tasks.insert(
                        task_id.clone(),
                        TaskInfo {
                            id: task_id,
                            bucket: bucket.to_string(),
                            path,
                            number,
                        },
                    );

                    if number > index.max_number {
                        index.max_number = number;
                    }
                }
            }
        }

        Ok(index)
    }

    /// Get the next available task number.
    pub fn next_number(&self) -> u32 {
        self.max_number + 1
    }

    /// Find a task by ID.
    pub fn find(&self, task_id: &str) -> Option<&TaskInfo> {
        // Normalize to uppercase
        let normalized = task_id.to_uppercase();
        self.tasks.get(&normalized)
    }

    /// Get all tasks in a specific bucket.
    pub fn tasks_in_bucket(&self, bucket: &str) -> Vec<&TaskInfo> {
        self.tasks.values().filter(|t| t.bucket == bucket).collect()
    }

    /// Get task counts per bucket.
    pub fn bucket_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for bucket in BUCKETS {
            counts.insert(bucket.to_string(), 0);
        }
        for task in self.tasks.values() {
            *counts.entry(task.bucket.clone()).or_insert(0) += 1;
        }
        counts
    }

    /// Get all tasks.
    pub fn all_tasks(&self) -> impl Iterator<Item = &TaskInfo> {
        self.tasks.values()
    }
}

/// Extract the task ID from a task filename.
///
/// Expected format: `TASK-{id}-{slug}.md`
///
/// Returns the task ID (e.g., "TASK-001") or None if the filename
/// doesn't match the expected pattern.
fn extract_task_id_from_filename(path: &Path) -> Option<String> {
    let filename = path.file_stem()?.to_str()?;

    // Match TASK-NNN at the start
    if let Some(rest) = filename.strip_prefix("TASK-") {
        // Find the second hyphen (after the number)
        if let Some(end) = rest.find('-') {
            let number_part = &rest[..end];
            // Verify it's numeric
            if number_part.chars().all(|c| c.is_ascii_digit()) && !number_part.is_empty() {
                return Some(format!("TASK-{}", number_part));
            }
        } else {
            // No slug, just TASK-NNN
            if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
                return Some(format!("TASK-{}", rest));
            }
        }
    }

    None
}

/// Extract the numeric part from a task ID.
fn extract_task_number(task_id: &str) -> Option<u32> {
    if let Some(rest) = task_id.strip_prefix("TASK-") {
        rest.parse().ok()
    } else {
        None
    }
}

/// Validate a task ID format.
///
/// Valid task IDs match the pattern `TASK-NNN` where NNN is at least 3 digits.
/// Also rejects any path traversal attempts.
///
/// # Arguments
///
/// * `task_id` - The task ID to validate
///
/// # Returns
///
/// * `Ok(String)` - The normalized (uppercase) task ID
/// * `Err(BurlError::UserError)` - If the ID is invalid
pub fn validate_task_id(task_id: &str) -> Result<String> {
    // Reject path traversal attempts
    if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
        return Err(BurlError::UserError(format!(
            "invalid task ID '{}': contains path traversal characters.\n\
             Task IDs must be in the format TASK-NNN (e.g., TASK-001).",
            task_id
        )));
    }

    // Normalize to uppercase
    let normalized = task_id.to_uppercase();

    // Validate format
    if !TASK_ID_REGEX.is_match(&normalized) {
        return Err(BurlError::UserError(format!(
            "invalid task ID '{}': must be in the format TASK-NNN (e.g., TASK-001).\n\
             The number must be at least 3 digits.",
            task_id
        )));
    }

    Ok(normalized)
}

/// Generate a task ID from a number.
///
/// # Arguments
///
/// * `number` - The task number
///
/// # Returns
///
/// The task ID in the format `TASK-NNN` (zero-padded to at least 3 digits).
pub fn generate_task_id(number: u32) -> String {
    format!("TASK-{:03}", number)
}

/// Slugify a title for use in a task filename.
///
/// Converts the title to lowercase, replaces spaces and special characters
/// with hyphens, removes punctuation, and limits length.
///
/// # Arguments
///
/// * `title` - The task title to slugify
///
/// # Returns
///
/// A safe slug for use in filenames.
pub fn slugify_title(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_hyphen = false;

    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen && !slug.is_empty() {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    // Remove trailing hyphen
    while slug.ends_with('-') {
        slug.pop();
    }

    // Limit length to keep filenames reasonable
    if slug.len() > 50 {
        // Truncate at word boundary if possible
        if let Some(pos) = slug[..50].rfind('-') {
            slug.truncate(pos);
        } else {
            slug.truncate(50);
        }
    }

    // Fallback for empty or all-punctuation titles
    if slug.is_empty() {
        slug = "untitled".to_string();
    }

    slug
}

/// Generate a task filename from ID and title.
///
/// # Arguments
///
/// * `task_id` - The task ID (e.g., "TASK-001")
/// * `title` - The task title
///
/// # Returns
///
/// The filename in the format `TASK-001-slug.md`.
pub fn generate_task_filename(task_id: &str, title: &str) -> String {
    let slug = slugify_title(title);
    format!("{}-{}.md", task_id, slug)
}

/// Validate that a generated filename is safe (no path traversal).
///
/// This is a security check to ensure the filename doesn't contain
/// any path components that could escape the intended directory.
pub fn validate_filename_safe(filename: &str) -> Result<()> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(BurlError::UserError(format!(
            "generated filename '{}' is not safe: contains path traversal characters",
            filename
        )));
    }

    // Check for hidden files (starting with .)
    if filename.starts_with('.') {
        return Err(BurlError::UserError(format!(
            "generated filename '{}' is not safe: starts with '.'",
            filename
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_task_id_valid() {
        assert_eq!(validate_task_id("TASK-001").unwrap(), "TASK-001");
        assert_eq!(validate_task_id("task-001").unwrap(), "TASK-001"); // lowercase
        assert_eq!(validate_task_id("TASK-123").unwrap(), "TASK-123");
        assert_eq!(validate_task_id("TASK-0001").unwrap(), "TASK-0001"); // 4 digits ok
        assert_eq!(validate_task_id("TASK-99999").unwrap(), "TASK-99999");
    }

    #[test]
    fn test_validate_task_id_invalid_format() {
        assert!(validate_task_id("TASK-01").is_err()); // too short
        assert!(validate_task_id("TASK-1").is_err()); // too short
        assert!(validate_task_id("task1").is_err()); // no hyphen
        assert!(validate_task_id("TASK001").is_err()); // no hyphen
        assert!(validate_task_id("001").is_err()); // no prefix
        assert!(validate_task_id("TASK-").is_err()); // no number
        assert!(validate_task_id("").is_err()); // empty
    }

    #[test]
    fn test_validate_task_id_path_traversal() {
        assert!(validate_task_id("../TASK-001").is_err());
        assert!(validate_task_id("TASK-001/..").is_err());
        assert!(validate_task_id("..\\TASK-001").is_err());
        assert!(validate_task_id("TASK-001\\..").is_err());
        assert!(validate_task_id("foo/../TASK-001").is_err());
    }

    #[test]
    fn test_generate_task_id() {
        assert_eq!(generate_task_id(1), "TASK-001");
        assert_eq!(generate_task_id(12), "TASK-012");
        assert_eq!(generate_task_id(123), "TASK-123");
        assert_eq!(generate_task_id(1234), "TASK-1234");
    }

    #[test]
    fn test_slugify_title_basic() {
        assert_eq!(slugify_title("Hello World"), "hello-world");
        assert_eq!(slugify_title("Implement feature"), "implement-feature");
        assert_eq!(slugify_title("Fix bug #123"), "fix-bug-123");
    }

    #[test]
    fn test_slugify_title_special_chars() {
        assert_eq!(slugify_title("Hello, World!"), "hello-world");
        assert_eq!(slugify_title("Test: stuff"), "test-stuff");
        assert_eq!(slugify_title("a@b#c$d"), "a-b-c-d");
        assert_eq!(slugify_title("foo---bar"), "foo-bar");
    }

    #[test]
    fn test_slugify_title_edge_cases() {
        assert_eq!(slugify_title(""), "untitled");
        assert_eq!(slugify_title("!!!"), "untitled");
        assert_eq!(slugify_title("   "), "untitled");
        assert_eq!(slugify_title("-test-"), "test");
    }

    #[test]
    fn test_slugify_title_length_limit() {
        let long_title =
            "This is a very long title that exceeds the maximum allowed length for slugs";
        let slug = slugify_title(long_title);
        assert!(slug.len() <= 50);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn test_generate_task_filename() {
        assert_eq!(
            generate_task_filename("TASK-001", "Hello World"),
            "TASK-001-hello-world.md"
        );
        assert_eq!(
            generate_task_filename("TASK-123", "Fix bug"),
            "TASK-123-fix-bug.md"
        );
    }

    #[test]
    fn test_validate_filename_safe_valid() {
        assert!(validate_filename_safe("TASK-001-hello.md").is_ok());
        assert!(validate_filename_safe("file.txt").is_ok());
    }

    #[test]
    fn test_validate_filename_safe_invalid() {
        assert!(validate_filename_safe("../file.txt").is_err());
        assert!(validate_filename_safe("dir/file.txt").is_err());
        assert!(validate_filename_safe(".hidden").is_err());
        assert!(validate_filename_safe("..\\file.txt").is_err());
    }

    #[test]
    fn test_extract_task_id_from_filename() {
        let path = PathBuf::from("TASK-001-hello-world.md");
        assert_eq!(
            extract_task_id_from_filename(&path),
            Some("TASK-001".to_string())
        );

        let path = PathBuf::from("TASK-123-test.md");
        assert_eq!(
            extract_task_id_from_filename(&path),
            Some("TASK-123".to_string())
        );

        let path = PathBuf::from("TASK-0001-long-slug.md");
        assert_eq!(
            extract_task_id_from_filename(&path),
            Some("TASK-0001".to_string())
        );

        let path = PathBuf::from("not-a-task.md");
        assert_eq!(extract_task_id_from_filename(&path), None);

        let path = PathBuf::from("TASK-abc-invalid.md");
        assert_eq!(extract_task_id_from_filename(&path), None);
    }

    #[test]
    fn test_extract_task_number() {
        assert_eq!(extract_task_number("TASK-001"), Some(1));
        assert_eq!(extract_task_number("TASK-123"), Some(123));
        assert_eq!(extract_task_number("TASK-0001"), Some(1));
        assert_eq!(extract_task_number("INVALID"), None);
    }
}
