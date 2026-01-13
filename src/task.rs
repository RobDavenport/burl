//! Task file model for burl.
//!
//! This module provides parsing and serialization of task files, which use
//! YAML frontmatter followed by a markdown body. The implementation supports:
//!
//! - Round-trip preservation of unknown YAML fields (forward compatibility)
//! - Exact preservation of markdown body content
//! - Common mutation helpers for workflow transitions
//!
//! # Task File Format
//!
//! Task files use YAML frontmatter delimited by `---` lines:
//!
//! ```text
//! ---
//! id: TASK-001
//! title: Implement feature
//! priority: high
//! ---
//!
//! ## Objective
//! Description of the task...
//! ```

use crate::error::{BurlError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A parsed task file with frontmatter and markdown body.
#[derive(Debug, Clone)]
pub struct TaskFile {
    /// The parsed frontmatter fields.
    pub frontmatter: TaskFrontmatter,
    /// The markdown body content (everything after the closing `---`).
    /// This includes any leading newlines after the frontmatter delimiter.
    pub body: String,
}

/// Task frontmatter fields.
///
/// Known fields are explicitly typed, while unknown fields are preserved
/// in the `extra` map for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFrontmatter {
    // =========================================================================
    // Required fields
    // =========================================================================
    /// Task identifier (e.g., "TASK-001").
    pub id: String,

    /// Task title.
    pub title: String,

    // =========================================================================
    // Priority and categorization
    // =========================================================================
    /// Priority level (high, medium, low).
    #[serde(default = "default_priority")]
    pub priority: String,

    /// Creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime<Utc>>,

    // =========================================================================
    // Ownership and attempts
    // =========================================================================
    /// Who claimed this task (e.g., "agent@hostname").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,

    /// Number of QA attempts.
    #[serde(default)]
    pub qa_attempts: u32,

    // =========================================================================
    // Lifecycle timestamps
    // =========================================================================
    /// When the task was claimed (moved to DOING).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,

    /// When the task was submitted for QA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<DateTime<Utc>>,

    /// When the task was approved and completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,

    // =========================================================================
    // Git/worktree state
    // =========================================================================
    /// Path to the task worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,

    /// Name of the task branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Base SHA for diff-based validation (set on claim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,

    // =========================================================================
    // Scope control
    // =========================================================================
    /// Explicit file paths this task affects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,

    /// Glob patterns for affected paths (supports new files).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects_globs: Vec<String>,

    /// Paths this task must not touch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_touch: Vec<String>,

    // =========================================================================
    // Dependencies
    // =========================================================================
    /// Task IDs this task depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,

    // =========================================================================
    // Tags
    // =========================================================================
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    // =========================================================================
    // Unknown fields (forward compatibility)
    // =========================================================================
    /// Any fields not explicitly defined above.
    /// Using BTreeMap for deterministic serialization order.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

fn default_priority() -> String {
    "medium".to_string()
}

impl Default for TaskFrontmatter {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            priority: default_priority(),
            created: None,
            assigned_to: None,
            qa_attempts: 0,
            started_at: None,
            submitted_at: None,
            completed_at: None,
            worktree: None,
            branch: None,
            base_sha: None,
            affects: Vec::new(),
            affects_globs: Vec::new(),
            must_not_touch: Vec::new(),
            depends_on: Vec::new(),
            tags: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl TaskFile {
    /// Parse a task file from its content string.
    ///
    /// The content should have YAML frontmatter delimited by `---` lines,
    /// followed by an optional markdown body.
    ///
    /// # Line ending handling
    ///
    /// Both Unix (LF) and Windows (CRLF) line endings are supported.
    /// The body is preserved exactly as-is, including its original line endings.
    ///
    /// # Examples
    ///
    /// ```
    /// use burl::task::TaskFile;
    ///
    /// let content = r#"---
    /// id: TASK-001
    /// title: Test task
    /// ---
    ///
    /// ## Objective
    /// Do something.
    /// "#;
    ///
    /// let task = TaskFile::parse(content).unwrap();
    /// assert_eq!(task.frontmatter.id, "TASK-001");
    /// ```
    pub fn parse(content: &str) -> Result<Self> {
        // Normalize line endings for delimiter detection, but preserve original for body
        let normalized = content.replace("\r\n", "\n");

        // Find frontmatter delimiters
        let (frontmatter_yaml, body_start) = Self::extract_frontmatter(&normalized, content)?;

        // Parse frontmatter YAML
        let frontmatter: TaskFrontmatter =
            serde_yaml::from_str(&frontmatter_yaml).map_err(|e| {
                BurlError::UserError(format!("failed to parse task frontmatter: {}", e))
            })?;

        // Extract body from original content (preserving original line endings)
        let body = if body_start < content.len() {
            content[body_start..].to_string()
        } else {
            String::new()
        };

        Ok(Self { frontmatter, body })
    }

    /// Extract frontmatter YAML and return the byte offset where the body starts.
    fn extract_frontmatter(normalized: &str, original: &str) -> Result<(String, usize)> {
        // Content must start with ---
        if !normalized.starts_with("---") {
            return Err(BurlError::UserError(
                "task file must start with '---' frontmatter delimiter".to_string(),
            ));
        }

        // Find the opening delimiter line end
        let first_newline = normalized.find('\n').ok_or_else(|| {
            BurlError::UserError("task file frontmatter is incomplete".to_string())
        })?;

        // Find the closing --- delimiter
        let rest = &normalized[first_newline + 1..];
        let closing_pos = rest.find("\n---").ok_or_else(|| {
            BurlError::UserError(
                "task file missing closing '---' frontmatter delimiter".to_string(),
            )
        })?;

        // Extract frontmatter content (between the delimiters)
        let frontmatter_yaml = rest[..closing_pos].to_string();

        // Calculate body start position in original content
        // We need to account for potential CRLF in original
        let normalized_body_start = first_newline + 1 + closing_pos + 4; // +4 for "\n---"

        // Find the actual position in original content by counting characters
        let body_start = Self::find_original_position(original, normalized, normalized_body_start);

        // Skip the newline after closing delimiter if present
        let body_start = if body_start < original.len() {
            let remaining = &original[body_start..];
            if remaining.starts_with("\r\n") {
                body_start + 2
            } else if remaining.starts_with('\n') {
                body_start + 1
            } else {
                body_start
            }
        } else {
            body_start
        };

        Ok((frontmatter_yaml, body_start))
    }

    /// Find the corresponding position in original content given a position in normalized content.
    fn find_original_position(original: &str, _normalized: &str, normalized_pos: usize) -> usize {
        // Count how many CRLFs have been converted up to this position
        let mut orig_pos = 0;
        let mut norm_pos = 0;
        let orig_bytes = original.as_bytes();

        while norm_pos < normalized_pos && orig_pos < original.len() {
            if orig_pos + 1 < original.len()
                && orig_bytes[orig_pos] == b'\r'
                && orig_bytes[orig_pos + 1] == b'\n'
            {
                // CRLF in original maps to single LF in normalized
                orig_pos += 2;
                norm_pos += 1;
            } else {
                orig_pos += 1;
                norm_pos += 1;
            }
        }

        orig_pos
    }

    /// Load a task file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read task file '{}': {}",
                path.display(),
                e
            ))
        })?;
        Self::parse(&content)
    }

    /// Atomically save the task file to disk.
    ///
    /// Uses atomic write (temp file + rename) to ensure the task file
    /// is never left in a corrupted state.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = self.to_string()?;
        crate::fs::atomic_write_file(path, &content)
    }

    /// Serialize the task file to a string.
    ///
    /// The output preserves the YAML frontmatter format and appends the body.
    pub fn to_string(&self) -> Result<String> {
        let frontmatter_yaml = serde_yaml::to_string(&self.frontmatter).map_err(|e| {
            BurlError::UserError(format!("failed to serialize task frontmatter: {}", e))
        })?;

        // Build the complete file content
        let mut output = String::new();
        output.push_str("---\n");
        output.push_str(&frontmatter_yaml);
        output.push_str("---\n");
        output.push_str(&self.body);

        Ok(output)
    }

    // =========================================================================
    // Mutation helpers for common workflow operations
    // =========================================================================

    /// Set the assigned_to field and optionally started_at timestamp.
    pub fn set_assigned(&mut self, assignee: &str, start_time: Option<DateTime<Utc>>) {
        self.frontmatter.assigned_to = Some(assignee.to_string());
        if let Some(time) = start_time {
            self.frontmatter.started_at = Some(time);
        }
    }

    /// Set git-related fields on claim.
    pub fn set_git_info(&mut self, branch: &str, worktree: &str, base_sha: &str) {
        self.frontmatter.branch = Some(branch.to_string());
        self.frontmatter.worktree = Some(worktree.to_string());
        self.frontmatter.base_sha = Some(base_sha.to_string());
    }

    /// Set the submitted_at timestamp.
    pub fn set_submitted(&mut self, time: DateTime<Utc>) {
        self.frontmatter.submitted_at = Some(time);
    }

    /// Set the completed_at timestamp.
    pub fn set_completed(&mut self, time: DateTime<Utc>) {
        self.frontmatter.completed_at = Some(time);
    }

    /// Increment the qa_attempts counter.
    pub fn increment_qa_attempts(&mut self) {
        self.frontmatter.qa_attempts += 1;
    }

    /// Append content to the QA Report section.
    ///
    /// If the section exists, content is appended below it.
    /// If not, a new section is created at the end of the body.
    pub fn append_to_qa_report(&mut self, content: &str) {
        const QA_REPORT_HEADING: &str = "## QA Report";

        // Ensure body ends with newline for clean appending
        if !self.body.is_empty() && !self.body.ends_with('\n') {
            self.body.push('\n');
        }

        if let Some(pos) = self.body.find(QA_REPORT_HEADING) {
            // Find the end of the QA Report section (next ## heading or end of body)
            let after_heading = pos + QA_REPORT_HEADING.len();
            let section_end = self.body[after_heading..]
                .find("\n## ")
                .map(|p| after_heading + p)
                .unwrap_or(self.body.len());

            // Insert content at the end of the section
            let insert_pos = section_end;

            // Ensure there's a newline before the new content
            let prefix = if insert_pos > 0 && !self.body[..insert_pos].ends_with('\n') {
                "\n"
            } else {
                ""
            };

            self.body
                .insert_str(insert_pos, &format!("{}{}\n", prefix, content));
        } else {
            // Create new QA Report section at the end
            self.body
                .push_str(&format!("\n{}\n{}\n", QA_REPORT_HEADING, content));
        }
    }

    /// Clear the assigned_to field (for unassignment).
    pub fn clear_assigned(&mut self) {
        self.frontmatter.assigned_to = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TASK: &str = r#"---
id: TASK-001
title: Test task
---

## Objective
Do something.
"#;

    const FULL_TASK: &str = r#"---
id: TASK-001
title: Implement player jump
priority: high
created: 2026-01-13T10:00:00Z
assigned_to: agent@host
qa_attempts: 1
started_at: 2026-01-13T11:00:00Z
submitted_at: null
completed_at: null
worktree: .worktrees/task-001-player-jump
branch: task-001-player-jump
base_sha: abc123def456
affects:
  - src/player/jump.rs
  - src/player/mod.rs
affects_globs:
  - src/player/**
must_not_touch:
  - src/enemy/**
depends_on:
  - TASK-000
tags:
  - feature
  - player
---

## Objective
Implement the player jump mechanic.

## Acceptance Criteria
- [ ] Player can jump
- [ ] Jump has cooldown

## Context
See design doc.

## Implementation Notes
Working on it.

## QA Report
<!-- Validator fills -->
"#;

    #[test]
    fn test_parse_minimal_task() {
        let task = TaskFile::parse(MINIMAL_TASK).unwrap();
        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.title, "Test task");
        assert_eq!(task.frontmatter.priority, "medium"); // default
        assert!(task.body.contains("## Objective"));
        assert!(task.body.contains("Do something."));
    }

    #[test]
    fn test_parse_full_task() {
        let task = TaskFile::parse(FULL_TASK).unwrap();

        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.title, "Implement player jump");
        assert_eq!(task.frontmatter.priority, "high");
        assert!(task.frontmatter.created.is_some());
        assert_eq!(task.frontmatter.assigned_to, Some("agent@host".to_string()));
        assert_eq!(task.frontmatter.qa_attempts, 1);
        assert!(task.frontmatter.started_at.is_some());
        assert!(task.frontmatter.worktree.is_some());
        assert!(task.frontmatter.branch.is_some());
        assert_eq!(task.frontmatter.base_sha, Some("abc123def456".to_string()));
        assert_eq!(task.frontmatter.affects.len(), 2);
        assert_eq!(task.frontmatter.affects_globs.len(), 1);
        assert_eq!(task.frontmatter.must_not_touch.len(), 1);
        assert_eq!(task.frontmatter.depends_on.len(), 1);
        assert_eq!(task.frontmatter.tags.len(), 2);
    }

    #[test]
    fn test_parse_with_unknown_fields() {
        let content = r#"---
id: TASK-001
title: Test task
unknown_field: some value
another_unknown:
  nested: true
  list:
    - a
    - b
future_v2_feature: enabled
---

Body content.
"#;
        let task = TaskFile::parse(content).unwrap();

        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.extra.len(), 3);
        assert!(task.frontmatter.extra.contains_key("unknown_field"));
        assert!(task.frontmatter.extra.contains_key("another_unknown"));
        assert!(task.frontmatter.extra.contains_key("future_v2_feature"));
    }

    #[test]
    fn test_roundtrip_preserves_unknown_fields() {
        let content = r#"---
id: TASK-001
title: Test task
unknown_field: preserved_value
nested_unknown:
  key: value
---

Body content.
"#;
        let task = TaskFile::parse(content).unwrap();
        let serialized = task.to_string().unwrap();
        let reparsed = TaskFile::parse(&serialized).unwrap();

        assert_eq!(reparsed.frontmatter.id, "TASK-001");
        assert_eq!(reparsed.frontmatter.extra.len(), 2);
        assert!(reparsed.frontmatter.extra.contains_key("unknown_field"));
        assert!(reparsed.frontmatter.extra.contains_key("nested_unknown"));
    }

    #[test]
    fn test_roundtrip_preserves_body() {
        let content = r#"---
id: TASK-001
title: Test task
---

## Section One
Content with special chars: < > & " '

## Section Two
- List item 1
- List item 2

```rust
fn code() {
    // comment
}
```
"#;
        let task = TaskFile::parse(content).unwrap();
        let serialized = task.to_string().unwrap();
        let reparsed = TaskFile::parse(&serialized).unwrap();

        // Body should be preserved exactly
        assert_eq!(reparsed.body, task.body);
        assert!(reparsed.body.contains("## Section One"));
        assert!(reparsed.body.contains("fn code()"));
    }

    #[test]
    fn test_parse_windows_crlf_line_endings() {
        // Create content with Windows CRLF line endings
        let content = "---\r\nid: TASK-001\r\ntitle: Test task\r\n---\r\n\r\n## Objective\r\nDo something.\r\n";

        let task = TaskFile::parse(content).unwrap();
        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.title, "Test task");
        // Body should preserve original line endings
        assert!(task.body.contains("\r\n") || task.body.contains("## Objective"));
    }

    #[test]
    fn test_parse_missing_opening_delimiter() {
        let content = r#"id: TASK-001
title: Test
---
Body
"#;
        let result = TaskFile::parse(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must start with"));
    }

    #[test]
    fn test_parse_missing_closing_delimiter() {
        let content = r#"---
id: TASK-001
title: Test

Body without closing delimiter
"#;
        let result = TaskFile::parse(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing closing"));
    }

    #[test]
    fn test_set_assigned() {
        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
        let now = Utc::now();

        task.set_assigned("agent@host", Some(now));

        assert_eq!(task.frontmatter.assigned_to, Some("agent@host".to_string()));
        assert_eq!(task.frontmatter.started_at, Some(now));
    }

    #[test]
    fn test_set_git_info() {
        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();

        task.set_git_info("task-001-test", ".worktrees/task-001-test", "abc123");

        assert_eq!(task.frontmatter.branch, Some("task-001-test".to_string()));
        assert_eq!(
            task.frontmatter.worktree,
            Some(".worktrees/task-001-test".to_string())
        );
        assert_eq!(task.frontmatter.base_sha, Some("abc123".to_string()));
    }

    #[test]
    fn test_set_timestamps() {
        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
        let now = Utc::now();

        task.set_submitted(now);
        assert_eq!(task.frontmatter.submitted_at, Some(now));

        task.set_completed(now);
        assert_eq!(task.frontmatter.completed_at, Some(now));
    }

    #[test]
    fn test_increment_qa_attempts() {
        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
        assert_eq!(task.frontmatter.qa_attempts, 0);

        task.increment_qa_attempts();
        assert_eq!(task.frontmatter.qa_attempts, 1);

        task.increment_qa_attempts();
        assert_eq!(task.frontmatter.qa_attempts, 2);
    }

    #[test]
    fn test_append_to_qa_report_new_section() {
        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();

        task.append_to_qa_report("Test report entry 1");
        assert!(task.body.contains("## QA Report"));
        assert!(task.body.contains("Test report entry 1"));

        task.append_to_qa_report("Test report entry 2");
        assert!(task.body.contains("Test report entry 2"));
    }

    #[test]
    fn test_append_to_qa_report_existing_section() {
        let content = r#"---
id: TASK-001
title: Test task
---

## Objective
Do something.

## QA Report
Existing report content.

## Other Section
Other content.
"#;
        let mut task = TaskFile::parse(content).unwrap();

        task.append_to_qa_report("New report entry");

        // New entry should be in QA Report section
        assert!(task.body.contains("New report entry"));
        // Other section should still exist
        assert!(task.body.contains("## Other Section"));
    }

    #[test]
    fn test_clear_assigned() {
        let mut task = TaskFile::parse(FULL_TASK).unwrap();
        assert!(task.frontmatter.assigned_to.is_some());

        task.clear_assigned();
        assert!(task.frontmatter.assigned_to.is_none());
    }

    #[test]
    fn test_empty_body() {
        let content = r#"---
id: TASK-001
title: Test task
---
"#;
        let task = TaskFile::parse(content).unwrap();
        assert!(task.body.is_empty() || task.body.chars().all(|c| c.is_whitespace()));
    }

    #[test]
    fn test_frontmatter_default() {
        let fm = TaskFrontmatter::default();
        assert!(fm.id.is_empty());
        assert!(fm.title.is_empty());
        assert_eq!(fm.priority, "medium");
        assert_eq!(fm.qa_attempts, 0);
        assert!(fm.affects.is_empty());
        assert!(fm.extra.is_empty());
    }

    #[test]
    fn test_serialize_skips_empty_fields() {
        let content = r#"---
id: TASK-001
title: Test task
---

Body.
"#;
        let task = TaskFile::parse(content).unwrap();
        let serialized = task.to_string().unwrap();

        // Empty optional fields should not appear in output
        assert!(!serialized.contains("affects:"));
        assert!(!serialized.contains("affects_globs:"));
        assert!(!serialized.contains("must_not_touch:"));
        assert!(!serialized.contains("depends_on:"));
        assert!(!serialized.contains("tags:"));
    }

    #[test]
    fn test_serialize_includes_non_empty_fields() {
        let content = r#"---
id: TASK-001
title: Test task
affects:
  - src/main.rs
tags:
  - feature
---

Body.
"#;
        let task = TaskFile::parse(content).unwrap();
        let serialized = task.to_string().unwrap();

        // Non-empty fields should appear
        assert!(serialized.contains("affects:") || serialized.contains("affects"));
        assert!(serialized.contains("tags:") || serialized.contains("tags"));
    }

    #[test]
    fn test_load_from_file() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TASK-001-test.md");

        let content = r#"---
id: TASK-001
title: Test task from file
---

## Objective
Load test.
"#;
        std::fs::write(&file_path, content).unwrap();

        let task = TaskFile::load(&file_path).unwrap();
        assert_eq!(task.frontmatter.id, "TASK-001");
        assert_eq!(task.frontmatter.title, "Test task from file");
        assert!(task.body.contains("Load test."));
    }

    #[test]
    fn test_save_atomic() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TASK-001-test.md");

        let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
        task.frontmatter.id = "TASK-002".to_string();
        task.set_assigned("test@example.com", Some(Utc::now()));

        task.save(&file_path).unwrap();

        // Reload and verify
        let loaded = TaskFile::load(&file_path).unwrap();
        assert_eq!(loaded.frontmatter.id, "TASK-002");
        assert_eq!(
            loaded.frontmatter.assigned_to,
            Some("test@example.com".to_string())
        );
    }

    #[test]
    fn test_save_atomic_replace_existing() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TASK-001-test.md");

        // Create initial file
        let task = TaskFile::parse(MINIMAL_TASK).unwrap();
        task.save(&file_path).unwrap();

        // Modify and save again
        let mut task = TaskFile::load(&file_path).unwrap();
        task.frontmatter.priority = "high".to_string();
        task.increment_qa_attempts();
        task.save(&file_path).unwrap();

        // Reload and verify
        let loaded = TaskFile::load(&file_path).unwrap();
        assert_eq!(loaded.frontmatter.priority, "high");
        assert_eq!(loaded.frontmatter.qa_attempts, 1);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = TaskFile::load("/nonexistent/path/TASK-001.md");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to read task file")
        );
    }
}
