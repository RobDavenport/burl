use super::*;
use crate::task::TaskFrontmatter;

/// Helper to create a TaskFrontmatter with specific fields.
fn make_frontmatter(
    affects: Vec<&str>,
    affects_globs: Vec<&str>,
    must_not_touch: Vec<&str>,
) -> TaskFrontmatter {
    TaskFrontmatter {
        id: "TASK-001".to_string(),
        title: "Test task".to_string(),
        affects: affects.into_iter().map(String::from).collect(),
        affects_globs: affects_globs.into_iter().map(String::from).collect(),
        must_not_touch: must_not_touch.into_iter().map(String::from).collect(),
        ..Default::default()
    }
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
    let fm = make_frontmatter(vec!["src/main.rs"], vec!["src/**"], vec!["src/net/**"]);
    let changed = vec!["src/net/client.rs".to_string()];

    let result = validate_scope(&fm, &changed).unwrap();

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].file_path, "src/net/client.rs");
    assert_eq!(
        result.violations[0].violation_type,
        ScopeViolationType::Forbidden
    );
    assert_eq!(
        result.violations[0].matched_pattern,
        Some("src/net/**".to_string())
    );
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
    assert_eq!(
        result.violations[0].violation_type,
        ScopeViolationType::OutOfScope
    );
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
        vec!["src/enemy/boss.rs"], // explicitly allowed
        vec!["src/enemy/**"],      // glob allowed
        vec!["src/enemy/**"],      // but also forbidden!
    );
    let changed = vec!["src/enemy/boss.rs".to_string()];

    let result = validate_scope(&fm, &changed).unwrap();

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(
        result.violations[0].violation_type,
        ScopeViolationType::Forbidden
    );
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
    assert!(
        result
            .violations
            .iter()
            .all(|v| v.violation_type == ScopeViolationType::Forbidden)
    );
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
    let changed = vec!["Cargo.toml".to_string(), "src/player/jump.rs".to_string()];

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
    let fm = make_frontmatter(vec!["src/main.rs"], vec![], vec!["src/secret/**"]);
    let changed = vec![
        "src/secret/keys.rs".to_string(), // forbidden
        "src/other/file.rs".to_string(),  // out of scope
    ];

    let result = validate_scope(&fm, &changed).unwrap();

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 2);

    let forbidden: Vec<_> = result
        .violations
        .iter()
        .filter(|v| v.violation_type == ScopeViolationType::Forbidden)
        .collect();
    let out_of_scope: Vec<_> = result
        .violations
        .iter()
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
    assert_eq!(
        result.violations[0].violation_type,
        ScopeViolationType::OutOfScope
    );
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
    let fm = TaskFrontmatter {
        id: "TASK-001".to_string(),
        title: "Test".to_string(),
        // Windows-style pattern (should be normalized)
        affects_globs: vec!["src\\player\\**".to_string()],
        ..Default::default()
    };

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
    let result = ScopeValidationResult::fail(vec![ScopeViolation::forbidden(
        "src/secret/keys.rs",
        "src/secret/**",
    )]);

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
    let result =
        ScopeValidationResult::fail(vec![ScopeViolation::out_of_scope("src/unauthorized.rs")]);

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
    let fm = TaskFrontmatter {
        id: "TASK-001".to_string(),
        title: "Test".to_string(),
        must_not_touch: vec!["[invalid".to_string()], // Invalid glob
        ..Default::default()
    };

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
    let fm = TaskFrontmatter {
        id: "TASK-001".to_string(),
        title: "Test".to_string(),
        // An unclosed bracket is an invalid glob pattern
        affects_globs: vec!["src/[unclosed".to_string()],
        ..Default::default()
    };

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
