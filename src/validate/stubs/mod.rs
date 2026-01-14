//! Stub detection validation for burl tasks.
//!
//! This module implements diff-based stub detection as defined in the PRD:
//! - Only scan **added lines** (not whole files) using diff.rs AddedLine
//! - Only files with extensions in `stub_check_extensions` config
//! - Apply regexes from `stub_patterns` config
//! - Fail with exact file + line + matched content
//!
//! Error handling:
//! - Invalid regex patterns are config errors (exit 1), not validation failures (exit 2)

mod patterns;
mod types;
mod validator;

#[cfg(test)]
mod tests;

// Re-export public API
pub use patterns::CompiledStubPatterns;
pub use types::{StubValidationResult, StubViolation};
pub use validator::{validate_stubs, validate_stubs_with_config};
