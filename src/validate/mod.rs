//! Validation module for burl.
//!
//! This module provides deterministic validation checks for task submissions:
//! - Scope enforcement: ensures changes are within allowed paths
//! - Stub detection: detects incomplete code patterns in added lines
//! - Build validation: runs build/test commands (future)

pub mod scope;
pub mod stubs;

pub use scope::{
    validate_scope, ScopeValidationResult, ScopeViolation, ScopeViolationType,
};
pub use stubs::{
    validate_stubs, validate_stubs_with_config, CompiledStubPatterns, StubValidationResult,
    StubViolation,
};
