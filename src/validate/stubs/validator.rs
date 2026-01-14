//! Core validation logic for stub detection.

use crate::config::Config;
use crate::diff::AddedLine;
use crate::error::Result;

use super::patterns::CompiledStubPatterns;
use super::types::{StubValidationResult, StubViolation};

/// Validate that added lines do not contain stub patterns.
///
/// This function implements diff-based stub detection:
/// 1. Filter added_lines to only files with extensions in `stub_check_extensions`
/// 2. For each added line, check against all compiled `stub_patterns`
/// 3. Collect all violations with exact file + line + content
///
/// # Arguments
///
/// * `patterns` - Pre-compiled stub patterns from config
/// * `added_lines` - List of added lines from diff parsing
///
/// # Returns
///
/// * `StubValidationResult` - Result with pass/fail and list of violations
///
/// # Example
///
/// ```
/// use burl::config::Config;
/// use burl::diff::AddedLine;
/// use burl::validate::stubs::{CompiledStubPatterns, validate_stubs};
///
/// let config = Config::default();
/// let patterns = CompiledStubPatterns::from_config(&config).unwrap();
///
/// let added_lines = vec![
///     AddedLine {
///         file_path: "src/lib.rs".to_string(),
///         line_number: 10,
///         content: "// TODO: implement this".to_string(),
///     },
/// ];
///
/// let result = validate_stubs(&patterns, &added_lines);
/// assert!(!result.passed);
/// ```
pub fn validate_stubs(
    patterns: &CompiledStubPatterns,
    added_lines: &[AddedLine],
) -> StubValidationResult {
    let mut violations = Vec::new();

    for line in added_lines {
        // Only check files with configured extensions
        if !patterns.should_check_file(&line.file_path) {
            continue;
        }

        // Check if line matches any stub pattern
        if let Some(matched_pattern) = patterns.matches_stub(&line.content) {
            violations.push(StubViolation::new(
                &line.file_path,
                line.line_number,
                &line.content,
                matched_pattern,
            ));
        }
    }

    if violations.is_empty() {
        StubValidationResult::pass()
    } else {
        StubValidationResult::fail(violations)
    }
}

/// Convenience function to validate stubs directly from config and added lines.
///
/// This compiles the patterns and validates in one call. For repeated validations,
/// prefer compiling patterns once with `CompiledStubPatterns::from_config` and
/// calling `validate_stubs` directly.
///
/// # Arguments
///
/// * `config` - The workflow configuration
/// * `added_lines` - List of added lines from diff parsing
///
/// # Returns
///
/// * `Ok(StubValidationResult)` - Validation result with pass/fail and violations
/// * `Err(BurlError::UserError)` - If any pattern fails to compile (config error)
pub fn validate_stubs_with_config(
    config: &Config,
    added_lines: &[AddedLine],
) -> Result<StubValidationResult> {
    let patterns = CompiledStubPatterns::from_config(config)?;
    Ok(validate_stubs(&patterns, added_lines))
}
