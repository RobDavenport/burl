//! Scope validation for burl tasks.
//!
//! This module implements deterministic scope enforcement as defined in the PRD:
//! - Rule S1: If any changed file matches `must_not_touch` -> fail
//! - Rule S2: Every changed file must match `affects` OR `affects_globs` -> fail otherwise
//!
//! New files are allowed if they match an allowed glob or directory pattern.

use crate::error::{BurlError, Result};
use crate::task::TaskFrontmatter;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// Type of scope violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeViolationType {
    /// File matches a `must_not_touch` pattern (Rule S1).
    Forbidden,
    /// File is not in allowed scope (Rule S2).
    OutOfScope,
}

/// A single scope violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeViolation {
    /// The file path that violated scope rules (repo-relative, forward slashes).
    pub file_path: String,
    /// The type of violation.
    pub violation_type: ScopeViolationType,
    /// The glob pattern that matched (for Forbidden violations).
    pub matched_pattern: Option<String>,
}

impl ScopeViolation {
    /// Create a new forbidden violation (S1).
    pub fn forbidden(file_path: impl Into<String>, matched_pattern: impl Into<String>) -> Self {
        Self {
            file_path: file_path.into(),
            violation_type: ScopeViolationType::Forbidden,
            matched_pattern: Some(matched_pattern.into()),
        }
    }

    /// Create a new out-of-scope violation (S2).
    pub fn out_of_scope(file_path: impl Into<String>) -> Self {
        Self {
            file_path: file_path.into(),
            violation_type: ScopeViolationType::OutOfScope,
            matched_pattern: None,
        }
    }
}

/// Result of scope validation.
#[derive(Debug, Clone)]
pub struct ScopeValidationResult {
    /// Whether validation passed.
    pub passed: bool,
    /// List of violations (empty if passed).
    pub violations: Vec<ScopeViolation>,
}

impl ScopeValidationResult {
    /// Create a passing result.
    pub fn pass() -> Self {
        Self {
            passed: true,
            violations: Vec::new(),
        }
    }

    /// Create a failing result with violations.
    pub fn fail(violations: Vec<ScopeViolation>) -> Self {
        Self {
            passed: false,
            violations,
        }
    }

    /// Format the result as a user-friendly error message.
    pub fn format_error(&self, task_id: &str) -> String {
        if self.passed {
            return String::new();
        }

        let mut msg = format!(
            "Scope violation\n\n{} touched files outside allowed scope:\n",
            task_id
        );

        for violation in &self.violations {
            match &violation.violation_type {
                ScopeViolationType::Forbidden => {
                    let pattern = violation.matched_pattern.as_deref().unwrap_or("<unknown>");
                    msg.push_str(&format!(
                        "  x {}  (matches must_not_touch: {})\n",
                        violation.file_path, pattern
                    ));
                }
                ScopeViolationType::OutOfScope => {
                    msg.push_str(&format!(
                        "  x {}  (not in affects/affects_globs)\n",
                        violation.file_path
                    ));
                }
            }
        }

        msg.push_str("\nFix: revert these changes or widen scope in the task file.");

        msg
    }
}

/// Validate that changed files are within the allowed scope.
///
/// Implements the scope enforcement rules from the PRD:
/// - Rule S1: If any changed file matches `must_not_touch` -> fail
/// - Rule S2: Every changed file must match at least one allowed path/glob -> fail otherwise
///
/// # Arguments
///
/// * `frontmatter` - The task frontmatter containing scope configuration
/// * `changed_files` - List of changed file paths (repo-relative, forward slashes)
///
/// # Returns
///
/// * `Ok(ScopeValidationResult)` - Validation result with pass/fail and violations
/// * `Err(BurlError)` - If glob patterns are invalid
///
/// # Examples
///
/// ```
/// use burl::task::TaskFrontmatter;
/// use burl::validate::validate_scope;
///
/// let mut frontmatter = TaskFrontmatter::default();
/// frontmatter.affects = vec!["src/main.rs".to_string()];
/// frontmatter.affects_globs = vec!["src/player/**".to_string()];
/// frontmatter.must_not_touch = vec!["src/enemy/**".to_string()];
///
/// let changed_files = vec!["src/main.rs".to_string(), "src/player/jump.rs".to_string()];
/// let result = validate_scope(&frontmatter, &changed_files).unwrap();
/// assert!(result.passed);
/// ```
pub fn validate_scope(
    frontmatter: &TaskFrontmatter,
    changed_files: &[String],
) -> Result<ScopeValidationResult> {
    // Early return if no files changed
    if changed_files.is_empty() {
        return Ok(ScopeValidationResult::pass());
    }

    // Build the forbidden glob set from must_not_touch
    let forbidden_globs = build_globset(&frontmatter.must_not_touch, "must_not_touch")?;

    // Build the allowed glob set from affects_globs
    let allowed_globs = build_globset(&frontmatter.affects_globs, "affects_globs")?;

    // Build a set of explicit allowed paths (normalized)
    let allowed_paths: std::collections::HashSet<String> = frontmatter
        .affects
        .iter()
        .map(|p| normalize_path(p))
        .collect();

    // Check each changed file
    let mut violations = Vec::new();

    for file in changed_files {
        let normalized_file = normalize_path(file);

        // Rule S1: Check against must_not_touch (takes priority)
        if let Some(pattern) = matches_globset(
            &forbidden_globs,
            &normalized_file,
            &frontmatter.must_not_touch,
        ) {
            violations.push(ScopeViolation::forbidden(&normalized_file, pattern));
            continue; // S1 takes priority, don't check S2
        }

        // Rule S2: Check if file is in allowed scope
        let in_explicit_paths = allowed_paths.contains(&normalized_file);
        let matches_allowed_glob = allowed_globs.is_match(&normalized_file);
        // Also check if file is under an allowed directory
        let in_allowed_directory = is_under_allowed_directory(&normalized_file, &allowed_paths);

        if !in_explicit_paths && !matches_allowed_glob && !in_allowed_directory {
            violations.push(ScopeViolation::out_of_scope(&normalized_file));
        }
    }

    if violations.is_empty() {
        Ok(ScopeValidationResult::pass())
    } else {
        Ok(ScopeValidationResult::fail(violations))
    }
}

/// Build a GlobSet from a list of glob patterns.
fn build_globset(patterns: &[String], field_name: &str) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        let normalized_pattern = normalize_path(pattern);
        let glob = Glob::new(&normalized_pattern).map_err(|e| {
            BurlError::UserError(format!(
                "invalid glob pattern in {}: '{}' - {}",
                field_name, pattern, e
            ))
        })?;
        builder.add(glob);
    }

    builder
        .build()
        .map_err(|e| BurlError::UserError(format!("failed to compile {} globs: {}", field_name, e)))
}

/// Check if a file matches any pattern in the globset, returning the matched pattern.
fn matches_globset(globset: &GlobSet, file: &str, patterns: &[String]) -> Option<String> {
    let matches = globset.matches(file);
    if matches.is_empty() {
        None
    } else {
        // Return the first matching pattern
        patterns.get(matches[0]).cloned()
    }
}

/// Check if a file is under an allowed directory.
///
/// For example, if `affects` contains `src/player/`, then
/// `src/player/jump.rs` is allowed.
fn is_under_allowed_directory(
    file: &str,
    allowed_paths: &std::collections::HashSet<String>,
) -> bool {
    for allowed in allowed_paths {
        // Check if allowed path is a directory (ends with /) or if file starts with allowed path
        if allowed.ends_with('/') && file.starts_with(allowed) {
            return true;
        }
        // Also check if allowed path (without trailing slash) is a prefix directory
        let dir_prefix = if allowed.ends_with('/') {
            allowed.to_string()
        } else {
            format!("{}/", allowed)
        };
        if file.starts_with(&dir_prefix) {
            return true;
        }
    }
    false
}

/// Normalize a file path to use forward slashes.
///
/// This ensures consistent path format for glob matching,
/// regardless of the platform.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests;
