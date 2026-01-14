//! Tests for stub validation.

use crate::config::Config;
use crate::diff::AddedLine;
use crate::error::BurlError;

use super::patterns::CompiledStubPatterns;
use super::types::{StubValidationResult, StubViolation};
use super::validator::{validate_stubs, validate_stubs_with_config};

// =========================================================================
// Helper functions
// =========================================================================

/// Create an AddedLine for testing.
fn make_added_line(file_path: &str, line_number: usize, content: &str) -> AddedLine {
    AddedLine {
        file_path: file_path.to_string(),
        line_number,
        content: content.to_string(),
    }
}

/// Create a config with custom stub patterns and extensions.
fn make_config(patterns: Vec<&str>, extensions: Vec<&str>) -> Config {
    Config {
        stub_patterns: patterns.into_iter().map(String::from).collect(),
        stub_check_extensions: extensions.into_iter().map(String::from).collect(),
        ..Default::default()
    }
}

// =========================================================================
// Acceptance criteria tests
// =========================================================================

/// AC: A pre-existing `TODO` in an unchanged part of a file does not fail validation.
/// This is tested by ensuring we only check added_lines, not existing file content.
#[test]
fn test_preexisting_todo_not_checked() {
    // If a TODO exists in the file but is not in added_lines, it should not be detected.
    // This test verifies that validation only checks the added_lines vector,
    // not the full file content.
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Empty added_lines means no pre-existing code is being checked
    let added_lines: Vec<AddedLine> = vec![];
    let result = validate_stubs(&patterns, &added_lines);

    assert!(result.passed);
    assert!(result.violations.is_empty());
}

/// AC: A newly-added `TODO` line fails validation and reports the exact location.
#[test]
fn test_newly_added_todo_fails() {
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![make_added_line(
        "src/player/jump.rs",
        45,
        "    // TODO: implement cooldown",
    )];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].file_path, "src/player/jump.rs");
    assert_eq!(result.violations[0].line_number, 45);
    assert_eq!(
        result.violations[0].content,
        "    // TODO: implement cooldown"
    );
    assert_eq!(result.violations[0].matched_pattern, "TODO");
}

/// AC: Stub scanning ignores files outside the configured extension list.
#[test]
fn test_ignores_unconfigured_extensions() {
    let config = make_config(vec!["TODO"], vec!["rs", "py"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // A TODO in a .md file should be ignored
    let added_lines = vec![make_added_line(
        "docs/README.md",
        10,
        "TODO: update documentation",
    )];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(result.passed);
    assert!(result.violations.is_empty());
}

// =========================================================================
// Unit test plan tests
// =========================================================================

/// Test Plan: Diff fixture includes an added line: `+ // TODO: implement`
/// Verify the stub is detected.
#[test]
fn test_detect_todo_in_added_line() {
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![make_added_line("src/lib.rs", 67, "// TODO: implement")];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].file_path, "src/lib.rs");
    assert_eq!(result.violations[0].line_number, 67);
}

/// Test Plan: Diff fixture includes an added line in a `.md` file (should be ignored).
#[test]
fn test_md_file_ignored() {
    let config = make_config(vec!["TODO"], vec!["rs", "py", "ts"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // .md is not in stub_check_extensions
    let added_lines = vec![make_added_line("README.md", 5, "TODO: add more examples")];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(result.passed);
}

// =========================================================================
// Pattern matching tests
// =========================================================================

/// Test all default stub patterns from config.
#[test]
fn test_default_stub_patterns() {
    let config = Config::default();
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Test each default pattern
    let test_cases = vec![
        ("src/lib.rs", "// TODO: fix this"),
        ("src/lib.rs", "// FIXME: broken"),
        ("src/lib.rs", "// XXX: hack"),
        ("src/lib.rs", "// HACK: temporary"),
        ("src/lib.rs", "    unimplemented!()"),
        ("src/lib.rs", "    todo!()"),
        ("src/lib.rs", r#"    panic!("not implemented")"#),
        ("src/lib.py", "raise NotImplementedError"),
        ("src/lib.py", "raise NotImplemented"),
        ("src/lib.py", "    pass"),
        ("src/lib.py", "    ..."),
    ];

    for (file_path, content) in test_cases {
        let added_lines = vec![make_added_line(file_path, 1, content)];
        let result = validate_stubs(&patterns, &added_lines);

        assert!(
            !result.passed,
            "Expected '{}' in {} to be detected as a stub",
            content, file_path
        );
    }
}

/// Test that non-stub code passes validation.
#[test]
fn test_non_stub_code_passes() {
    let config = Config::default();
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![
        make_added_line("src/lib.rs", 1, "fn main() {"),
        make_added_line("src/lib.rs", 2, "    println!(\"Hello, world!\");"),
        make_added_line("src/lib.rs", 3, "}"),
        make_added_line("src/lib.rs", 4, "// This is a normal comment"),
        make_added_line("src/lib.rs", 5, "let x = 42;"),
    ];

    let result = validate_stubs(&patterns, &added_lines);
    assert!(result.passed);
}

/// Test regex pattern matching (not just substring).
#[test]
fn test_regex_pattern_matching() {
    // Use a regex that requires word boundary
    let config = make_config(vec![r"\bTODO\b"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Should match: TODO as a word
    let added_lines = vec![make_added_line("src/lib.rs", 1, "// TODO: fix")];
    let result = validate_stubs(&patterns, &added_lines);
    assert!(!result.passed);

    // Should not match: TODOS (not a word boundary)
    let added_lines = vec![make_added_line("src/lib.rs", 1, "// TODOS list")];
    let result = validate_stubs(&patterns, &added_lines);
    assert!(result.passed);
}

/// Test case sensitivity (default patterns are case-sensitive).
#[test]
fn test_case_sensitivity() {
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Should match: exact case
    let added_lines = vec![make_added_line("src/lib.rs", 1, "// TODO")];
    let result = validate_stubs(&patterns, &added_lines);
    assert!(!result.passed);

    // Should not match: different case
    let added_lines = vec![make_added_line("src/lib.rs", 1, "// todo")];
    let result = validate_stubs(&patterns, &added_lines);
    assert!(result.passed);

    // Test case-insensitive regex
    let config = make_config(vec!["(?i)TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![make_added_line("src/lib.rs", 1, "// todo")];
    let result = validate_stubs(&patterns, &added_lines);
    assert!(!result.passed);
}

// =========================================================================
// Extension filtering tests
// =========================================================================

/// Test should_check_file with various extensions.
#[test]
fn test_should_check_file() {
    let config = make_config(vec!["TODO"], vec!["rs", "py", "ts", "js", "tsx", "jsx"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Should check
    assert!(patterns.should_check_file("src/main.rs"));
    assert!(patterns.should_check_file("src/lib.py"));
    assert!(patterns.should_check_file("src/app.ts"));
    assert!(patterns.should_check_file("src/app.tsx"));
    assert!(patterns.should_check_file("src/app.js"));
    assert!(patterns.should_check_file("src/app.jsx"));

    // Should not check
    assert!(!patterns.should_check_file("README.md"));
    assert!(!patterns.should_check_file("Cargo.toml"));
    assert!(!patterns.should_check_file("config.yaml"));
    assert!(!patterns.should_check_file(".gitignore"));
    assert!(!patterns.should_check_file("no_extension"));
}

/// Test extension matching is case-insensitive.
#[test]
fn test_extension_case_insensitive() {
    let config = make_config(vec!["TODO"], vec!["rs", "py"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Extensions should match case-insensitively
    assert!(patterns.should_check_file("src/main.RS"));
    assert!(patterns.should_check_file("src/lib.Py"));
    assert!(patterns.should_check_file("src/app.RS"));
}

/// Test files without extensions.
#[test]
fn test_no_extension() {
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    assert!(!patterns.should_check_file("Makefile"));
    assert!(!patterns.should_check_file(".gitignore"));
    assert!(!patterns.should_check_file("src/no_ext"));
}

// =========================================================================
// Invalid pattern tests
// =========================================================================

/// Test that invalid regex patterns produce config error (exit 1).
#[test]
fn test_invalid_regex_pattern() {
    let config = make_config(vec!["[invalid"], vec!["rs"]);
    let result = CompiledStubPatterns::from_config(&config);

    assert!(result.is_err());
    let err = result.unwrap_err();

    // Should be a UserError (exit 1), not ValidationError (exit 2)
    match err {
        BurlError::UserError(msg) => {
            assert!(msg.contains("invalid regex pattern"));
            assert!(msg.contains("[invalid"));
            assert!(msg.contains("stub_patterns"));
            assert!(msg.contains("config.yaml"));
        }
        _ => panic!("Expected UserError, got {:?}", err),
    }
}

/// Test that valid regex patterns compile successfully.
#[test]
fn test_valid_regex_patterns() {
    let patterns = vec![
        "TODO",
        r"^\s*pass\s*$",
        r"panic!\s*\(",
        r"\bunimplemented!\b",
        "(?i)fixme",
    ];
    let config = make_config(patterns, vec!["rs"]);

    let result = CompiledStubPatterns::from_config(&config);
    assert!(result.is_ok());
}

// =========================================================================
// Multiple violations tests
// =========================================================================

/// Test multiple violations in the same file.
#[test]
fn test_multiple_violations_same_file() {
    let config = make_config(vec!["TODO", "FIXME", "unimplemented!"], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![
        make_added_line("src/lib.rs", 10, "// TODO: first"),
        make_added_line("src/lib.rs", 20, "// FIXME: second"),
        make_added_line("src/lib.rs", 30, "    unimplemented!()"),
    ];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 3);

    // Check all violations are reported with correct line numbers
    assert_eq!(result.violations[0].line_number, 10);
    assert_eq!(result.violations[1].line_number, 20);
    assert_eq!(result.violations[2].line_number, 30);
}

/// Test violations across multiple files.
#[test]
fn test_violations_across_files() {
    let config = make_config(vec!["TODO"], vec!["rs", "py"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![
        make_added_line("src/main.rs", 5, "// TODO: in rust"),
        make_added_line("src/lib.py", 10, "# TODO: in python"),
        make_added_line("docs/readme.md", 1, "TODO: ignored"), // ignored
    ];

    let result = validate_stubs(&patterns, &added_lines);

    assert!(!result.passed);
    assert_eq!(result.violations.len(), 2);
    assert_eq!(result.violations[0].file_path, "src/main.rs");
    assert_eq!(result.violations[1].file_path, "src/lib.py");
}

// =========================================================================
// Error message formatting tests
// =========================================================================

/// Test error message format matches PRD spec.
#[test]
fn test_error_message_format() {
    let result = StubValidationResult::fail(vec![
        StubViolation::new(
            "src/player/jump.rs",
            67,
            "unimplemented!()",
            "unimplemented!",
        ),
        StubViolation::new(
            "src/player/jump.rs",
            45,
            "// TODO: implement cooldown",
            "TODO",
        ),
    ]);

    let msg = result.format_error();

    assert!(msg.contains("Stub patterns found in added lines"));
    assert!(msg.contains("src/player/jump.rs:67  + unimplemented!()"));
    assert!(msg.contains("src/player/jump.rs:45  + // TODO: implement cooldown"));
}

/// Test passing result has empty error message.
#[test]
fn test_pass_empty_message() {
    let result = StubValidationResult::pass();
    let msg = result.format_error();
    assert!(msg.is_empty());
}

// =========================================================================
// StubViolation tests
// =========================================================================

#[test]
fn test_stub_violation_construction() {
    let v = StubViolation::new("src/lib.rs", 42, "// TODO: test", "TODO");

    assert_eq!(v.file_path, "src/lib.rs");
    assert_eq!(v.line_number, 42);
    assert_eq!(v.content, "// TODO: test");
    assert_eq!(v.matched_pattern, "TODO");
}

#[test]
fn test_stub_violation_equality() {
    let v1 = StubViolation::new("src/lib.rs", 10, "// TODO", "TODO");
    let v2 = StubViolation::new("src/lib.rs", 10, "// TODO", "TODO");
    let v3 = StubViolation::new("src/lib.rs", 11, "// TODO", "TODO");

    assert_eq!(v1, v2);
    assert_ne!(v1, v3);
}

// =========================================================================
// Convenience function tests
// =========================================================================

/// Test validate_stubs_with_config convenience function.
#[test]
fn test_validate_stubs_with_config() {
    let config = make_config(vec!["TODO"], vec!["rs"]);
    let added_lines = vec![make_added_line("src/lib.rs", 1, "// TODO")];

    let result = validate_stubs_with_config(&config, &added_lines).unwrap();
    assert!(!result.passed);
}

/// Test validate_stubs_with_config with invalid patterns.
#[test]
fn test_validate_stubs_with_config_invalid_pattern() {
    let config = make_config(vec!["[invalid"], vec!["rs"]);
    let added_lines = vec![make_added_line("src/lib.rs", 1, "some code")];

    let result = validate_stubs_with_config(&config, &added_lines);
    assert!(result.is_err());
}

// =========================================================================
// Empty input tests
// =========================================================================

/// Test empty added_lines produces pass.
#[test]
fn test_empty_added_lines() {
    let config = Config::default();
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines: Vec<AddedLine> = vec![];
    let result = validate_stubs(&patterns, &added_lines);

    assert!(result.passed);
    assert!(result.violations.is_empty());
}

/// Test empty stub_patterns means nothing is checked.
#[test]
fn test_empty_stub_patterns() {
    let config = make_config(vec![], vec!["rs"]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![
        make_added_line("src/lib.rs", 1, "// TODO: would normally match"),
        make_added_line("src/lib.rs", 2, "unimplemented!()"),
    ];

    let result = validate_stubs(&patterns, &added_lines);
    assert!(result.passed);
}

/// Test empty extensions means no files are checked.
#[test]
fn test_empty_extensions() {
    let config = make_config(vec!["TODO"], vec![]);
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    let added_lines = vec![make_added_line("src/lib.rs", 1, "// TODO")];

    let result = validate_stubs(&patterns, &added_lines);
    assert!(result.passed);
}

// =========================================================================
// Integration-style tests
// =========================================================================

/// Test a realistic diff scenario with mixed changes.
#[test]
fn test_realistic_diff_scenario() {
    let config = Config::default();
    let patterns = CompiledStubPatterns::from_config(&config).unwrap();

    // Simulate a diff with various file types and content
    let added_lines = vec![
        // Clean Rust code
        make_added_line("src/player/jump.rs", 15, "    fn jump(&mut self) {"),
        make_added_line(
            "src/player/jump.rs",
            16,
            "        self.velocity.y = JUMP_FORCE;",
        ),
        make_added_line("src/player/jump.rs", 17, "    }"),
        // Stub in Rust code
        make_added_line("src/player/jump.rs", 30, "    unimplemented!()"),
        // Clean Python code
        make_added_line("scripts/build.py", 5, "def build():"),
        make_added_line(
            "scripts/build.py",
            6,
            "    return subprocess.run(['cargo', 'build'])",
        ),
        // Documentation (ignored)
        make_added_line("docs/README.md", 10, "TODO: document the API"),
        // Config file (ignored)
        make_added_line("config.yaml", 1, "# TODO: add more settings"),
    ];

    let result = validate_stubs(&patterns, &added_lines);

    // Should fail due to unimplemented!() in jump.rs
    assert!(!result.passed);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].file_path, "src/player/jump.rs");
    assert_eq!(result.violations[0].line_number, 30);
}
