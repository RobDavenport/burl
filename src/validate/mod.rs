//! Validation module for burl.
//!
//! This module provides deterministic validation checks for task submissions:
//! - Scope enforcement: ensures changes are within allowed paths
//! - Stub detection: detects incomplete code patterns in added lines
//! - Build validation: runs build/test commands (future)

pub mod pipeline;
pub mod scope;
pub mod stubs;

pub use pipeline::{
    ValidationStepResult, ValidationStepStatus, run_command_steps, should_run_step,
};
pub use scope::{ScopeValidationResult, ScopeViolation, ScopeViolationType, validate_scope};
pub use stubs::{
    CompiledStubPatterns, StubValidationResult, StubViolation, validate_stubs,
    validate_stubs_with_config,
};
