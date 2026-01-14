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

mod io;
mod mutations;
#[cfg(test)]
mod tests;

// Re-export methods are implemented directly on TaskFile via impl blocks
// in the io and mutations modules, so no explicit re-exports needed

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
    // Agent assignment (V2)
    // =========================================================================
    /// Agent profile to use for this task (V2).
    /// If not set, the default agent from agents.yaml is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,

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
            agent: None,
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
}
