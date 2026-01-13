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
                    let pattern = violation
                        .matched_pattern
                        .as_deref()
                        .unwrap_or("<unknown>");
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
        if let Some(pattern) = matches_globset(&forbidden_globs, &normalized_file, &frontmatter.must_not_touch) {
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

    builder.build().map_err(|e| {
        BurlError::UserError(format!("failed to compile {} globs: {}", field_name, e))
    })
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
fn is_under_allowed_directory(file: &str, allowed_paths: &std::collections::HashSet<String>) -> bool {
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
mod tests {
    use super::*;

    /// Helper to create a TaskFrontmatter with specific fields.
    fn make_frontmatter(
        affects: Vec<&str>,
        affects_globs: Vec<&str>,
        must_not_touch: Vec<&str>,
    ) -> TaskFrontmatter {
        let mut fm = TaskFrontmatter::default();
        fm.id = "TASK-001".to_string();
        fm.title = "Test task".to_string();
        fm.affects = affects.into_iter().map(String::from).collect();
        fm.affects_globs = affects_globs.into_iter().map(String::from).collect();
        fm.must_not_touch = must_not_touch.into_iter().map(String::from).collect();
        fm
    }

    // =========================================================================
    // Basic tests from acceptance criteria
    // =========================================================================

    /// Test: Allowed-only - changed file in `src/foo.rs` with `affects: [src/foo.rs]` -> pass
    #[test]
    fn test_allowed_exact_path_pass() {
        let fm = make_frontmatter(vec!["src/foo.rs"], vec![], vec![]);
        let changed = vec!["src/foo.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(result.passed);
        assert!(result.violations.is_empty());
    }

    /// Test: Forbidden - changed file in `src/net/**` with `must_not_touch: [src/net/**]` -> fail
    #[test]
    fn test_forbidden_glob_fail() {
        let fm = make_frontmatter(
            vec!["src/main.rs"],
            vec!["src/**"],
            vec!["src/net/**"],
        );
        let changed = vec!["src/net/client.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].file_path, "src/net/client.rs");
        assert_eq!(result.violations[0].violation_type, ScopeViolationType::Forbidden);
        assert_eq!(result.violations[0].matched_pattern, Some("src/net/**".to_string()));
    }

    /// Test: Out-of-scope - changed file not matching any allow -> fail
    #[test]
    fn test_out_of_scope_fail() {
        let fm = make_frontmatter(vec!["src/main.rs"], vec![], vec![]);
        let changed = vec!["src/other.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].file_path, "src/other.rs");
        assert_eq!(result.violations[0].violation_type, ScopeViolationType::OutOfScope);
    }

    /// Test: New file - `affects_globs: [src/player/**]` and new file `src/player/jump.rs` -> pass
    #[test]
    fn test_new_file_under_allowed_glob_pass() {
        let fm = make_frontmatter(vec![], vec!["src/player/**"], vec![]);
        let changed = vec!["src/player/jump.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(result.passed);
        assert!(result.violations.is_empty());
    }

    // =========================================================================
    // Rule S1 tests: must_not_touch takes priority
    // =========================================================================

    /// A change matching must_not_touch fails even if it is also in allowed scope.
    #[test]
    fn test_forbidden_takes_priority_over_allowed() {
        let fm = make_frontmatter(
            vec!["src/enemy/boss.rs"],  // explicitly allowed
            vec!["src/enemy/**"],       // glob allowed
            vec!["src/enemy/**"],       // but also forbidden!
        );
        let changed = vec!["src/enemy/boss.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].violation_type, ScopeViolationType::Forbidden);
    }

    /// Test multiple forbidden patterns.
    #[test]
    fn test_multiple_forbidden_patterns() {
        let fm = make_frontmatter(
            vec!["src/main.rs"],
            vec!["src/**"],
            vec!["src/net/**", "src/secret/**", "*.lock"],
        );
        let changed = vec![
            "src/net/tcp.rs".to_string(),
            "src/secret/keys.rs".to_string(),
            "Cargo.lock".to_string(),
        ];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations.len(), 3);
        assert!(result.violations.iter().all(|v| v.violation_type == ScopeViolationType::Forbidden));
    }

    // =========================================================================
    // Rule S2 tests: allowed scope
    // =========================================================================

    /// Test multiple explicit affects paths.
    #[test]
    fn test_multiple_explicit_paths() {
        let fm = make_frontmatter(
            vec!["src/main.rs", "src/lib.rs", "Cargo.toml"],
            vec![],
            vec![],
        );
        let changed = vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "Cargo.toml".to_string(),
        ];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test glob patterns for allowed scope.
    #[test]
    fn test_affects_globs() {
        let fm = make_frontmatter(vec![], vec!["src/player/**", "tests/**"], vec![]);
        let changed = vec![
            "src/player/move.rs".to_string(),
            "src/player/nested/deep/file.rs".to_string(),
            "tests/integration.rs".to_string(),
        ];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test combining affects and affects_globs.
    #[test]
    fn test_combined_affects_and_globs() {
        let fm = make_frontmatter(
            vec!["Cargo.toml", "README.md"],
            vec!["src/player/**"],
            vec![],
        );
        let changed = vec![
            "Cargo.toml".to_string(),
            "src/player/jump.rs".to_string(),
        ];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test that files under an allowed directory path are allowed.
    #[test]
    fn test_directory_path_allows_children() {
        let fm = make_frontmatter(vec!["src/player/"], vec![], vec![]);
        let changed = vec!["src/player/jump.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    // =========================================================================
    // Mixed violations tests
    // =========================================================================

    /// Test mixed violations (some forbidden, some out of scope).
    #[test]
    fn test_mixed_violations() {
        let fm = make_frontmatter(
            vec!["src/main.rs"],
            vec![],
            vec!["src/secret/**"],
        );
        let changed = vec![
            "src/secret/keys.rs".to_string(),  // forbidden
            "src/other/file.rs".to_string(),   // out of scope
        ];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations.len(), 2);

        let forbidden: Vec<_> = result.violations.iter()
            .filter(|v| v.violation_type == ScopeViolationType::Forbidden)
            .collect();
        let out_of_scope: Vec<_> = result.violations.iter()
            .filter(|v| v.violation_type == ScopeViolationType::OutOfScope)
            .collect();

        assert_eq!(forbidden.len(), 1);
        assert_eq!(out_of_scope.len(), 1);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    /// Test empty changed files list.
    #[test]
    fn test_empty_changed_files() {
        let fm = make_frontmatter(vec!["src/main.rs"], vec![], vec!["src/secret/**"]);
        let changed: Vec<String> = vec![];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test empty scope (no affects, no affects_globs).
    #[test]
    fn test_empty_scope_fails_any_change() {
        let fm = make_frontmatter(vec![], vec![], vec![]);
        let changed = vec!["src/any.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();

        assert!(!result.passed);
        assert_eq!(result.violations[0].violation_type, ScopeViolationType::OutOfScope);
    }

    /// Test path normalization (backslashes to forward slashes).
    #[test]
    fn test_path_normalization() {
        let fm = make_frontmatter(vec!["src/player/jump.rs"], vec![], vec![]);
        // Windows-style path in changed files
        let changed = vec!["src\\player\\jump.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test glob with backslashes in pattern.
    #[test]
    fn test_pattern_normalization() {
        let mut fm = TaskFrontmatter::default();
        fm.id = "TASK-001".to_string();
        fm.title = "Test".to_string();
        // Windows-style pattern (should be normalized)
        fm.affects_globs = vec!["src\\player\\**".to_string()];

        let changed = vec!["src/player/jump.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test single-star glob (matches within directory).
    /// Note: In globset, `*` matches everything including path separators.
    /// Use `src/*.rs` style patterns for non-recursive matching within a directory.
    #[test]
    fn test_single_star_glob() {
        let fm = make_frontmatter(vec![], vec!["src/*.rs"], vec![]);
        let changed = vec!["src/main.rs".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);

        // Note: globset's `*` matches path separators, so `src/*.rs` WILL match
        // `src/player/jump.rs`. This is the expected behavior of globset.
        // If you need strict non-recursive matching, use more specific patterns.
        let changed_nested = vec!["src/player/jump.rs".to_string()];
        let result_nested = validate_scope(&fm, &changed_nested).unwrap();
        // globset treats `*` as matching any character including `/`
        assert!(result_nested.passed);
    }

    /// Test double-star glob (matches nested directories).
    #[test]
    fn test_double_star_glob() {
        let fm = make_frontmatter(vec![], vec!["src/**/*.rs"], vec![]);
        let changed = vec![
            "src/main.rs".to_string(),
            "src/player/jump.rs".to_string(),
            "src/player/nested/deep.rs".to_string(),
        ];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);
    }

    /// Test extension glob.
    #[test]
    fn test_extension_glob() {
        let fm = make_frontmatter(vec![], vec!["*.toml", "*.lock"], vec![]);
        let changed = vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()];

        let result = validate_scope(&fm, &changed).unwrap();
        assert!(result.passed);

        // .rs should fail
        let changed_rs = vec!["main.rs".to_string()];
        let result_rs = validate_scope(&fm, &changed_rs).unwrap();
        assert!(!result_rs.passed);
    }

    // =========================================================================
    // Error message formatting tests
    // =========================================================================

    /// Test error message format for forbidden violations.
    #[test]
    fn test_error_format_forbidden() {
        let result = ScopeValidationResult::fail(vec![
            ScopeViolation::forbidden("src/secret/keys.rs", "src/secret/**"),
        ]);

        let msg = result.format_error("TASK-001");
        assert!(msg.contains("Scope violation"));
        assert!(msg.contains("TASK-001"));
        assert!(msg.contains("src/secret/keys.rs"));
        assert!(msg.contains("must_not_touch"));
        assert!(msg.contains("src/secret/**"));
    }

    /// Test error message format for out-of-scope violations.
    #[test]
    fn test_error_format_out_of_scope() {
        let result = ScopeValidationResult::fail(vec![
            ScopeViolation::out_of_scope("src/unauthorized.rs"),
        ]);

        let msg = result.format_error("TASK-002");
        assert!(msg.contains("src/unauthorized.rs"));
        assert!(msg.contains("not in affects/affects_globs"));
    }

    /// Test passing result has empty error message.
    #[test]
    fn test_error_format_pass() {
        let result = ScopeValidationResult::pass();
        let msg = result.format_error("TASK-001");
        assert!(msg.is_empty());
    }

    // =========================================================================
    // Invalid glob pattern tests
    // =========================================================================

    /// Test invalid glob pattern returns error.
    #[test]
    fn test_invalid_glob_pattern() {
        let mut fm = TaskFrontmatter::default();
        fm.id = "TASK-001".to_string();
        fm.title = "Test".to_string();
        fm.must_not_touch = vec!["[invalid".to_string()]; // Invalid glob

        let changed = vec!["src/main.rs".to_string()];

        let result = validate_scope(&fm, &changed);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("invalid glob pattern"));
        assert!(err_msg.contains("must_not_touch"));
    }

    /// Test invalid affects_globs pattern returns error.
    #[test]
    fn test_invalid_affects_globs_pattern() {
        let mut fm = TaskFrontmatter::default();
        fm.id = "TASK-001".to_string();
        fm.title = "Test".to_string();
        // An unclosed bracket is an invalid glob pattern
        fm.affects_globs = vec!["src/[unclosed".to_string()];

        let changed = vec!["src/main.rs".to_string()];

        let result = validate_scope(&fm, &changed);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("invalid glob pattern"));
        assert!(err_msg.contains("affects_globs"));
    }

    // =========================================================================
    // ScopeViolation tests
    // =========================================================================

    #[test]
    fn test_scope_violation_forbidden_construction() {
        let v = ScopeViolation::forbidden("path/to/file.rs", "path/**");
        assert_eq!(v.file_path, "path/to/file.rs");
        assert_eq!(v.violation_type, ScopeViolationType::Forbidden);
        assert_eq!(v.matched_pattern, Some("path/**".to_string()));
    }

    #[test]
    fn test_scope_violation_out_of_scope_construction() {
        let v = ScopeViolation::out_of_scope("path/to/file.rs");
        assert_eq!(v.file_path, "path/to/file.rs");
        assert_eq!(v.violation_type, ScopeViolationType::OutOfScope);
        assert_eq!(v.matched_pattern, None);
    }

    #[test]
    fn test_scope_violation_equality() {
        let v1 = ScopeViolation::forbidden("src/a.rs", "src/**");
        let v2 = ScopeViolation::forbidden("src/a.rs", "src/**");
        let v3 = ScopeViolation::out_of_scope("src/a.rs");

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    // =========================================================================
    // ScopeValidationResult tests
    // =========================================================================

    #[test]
    fn test_scope_validation_result_pass() {
        let result = ScopeValidationResult::pass();
        assert!(result.passed);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_scope_validation_result_fail() {
        let violations = vec![ScopeViolation::out_of_scope("file.rs")];
        let result = ScopeValidationResult::fail(violations);
        assert!(!result.passed);
        assert_eq!(result.violations.len(), 1);
    }
}
