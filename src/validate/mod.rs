//! Validation module for burl.
//!
//! This module provides deterministic validation checks for task submissions:
//! - Scope enforcement: ensures changes are within allowed paths
//! - Stub detection: detects incomplete code patterns in added lines (TODO: TASK-010)
//! - Build validation: runs build/test commands (future)

pub mod scope;

pub use scope::{
    validate_scope, ScopeValidationResult, ScopeViolation, ScopeViolationType,
};
