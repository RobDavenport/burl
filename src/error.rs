//! Error types for the burl CLI.
//!
//! Uses thiserror for derive macros and provides user-actionable error messages.

use crate::exit_codes;
use thiserror::Error;

/// Main error type for burl operations.
///
/// Each variant maps to a specific exit code as defined in the V1 specification.
/// Some variants are not yet used but are defined for future command implementations.
#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum BurlError {
    /// Command is not yet implemented.
    #[error("{0} is not yet implemented")]
    NotImplemented(String),

    /// User provided invalid arguments or the system is in an invalid state.
    #[error("{0}")]
    UserError(String),

    /// Validation failed (scope, stubs, or build/test).
    #[error("Validation failed: {0}")]
    ValidationError(String),

    /// Git operation failed.
    #[error("Git operation failed: {0}")]
    GitError(String),

    /// Lock could not be acquired.
    #[error("Lock acquisition failed: {0}")]
    LockError(String),
}

impl BurlError {
    /// Returns the appropriate exit code for this error type.
    pub fn exit_code(&self) -> i32 {
        match self {
            BurlError::NotImplemented(_) => exit_codes::USER_ERROR,
            BurlError::UserError(_) => exit_codes::USER_ERROR,
            BurlError::ValidationError(_) => exit_codes::VALIDATION_FAILURE,
            BurlError::GitError(_) => exit_codes::GIT_FAILURE,
            BurlError::LockError(_) => exit_codes::LOCK_FAILURE,
        }
    }
}

/// Result type alias for burl operations.
pub type Result<T> = std::result::Result<T, BurlError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_error_has_correct_exit_code() {
        let err = BurlError::NotImplemented("test command".to_string());
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn user_error_has_correct_exit_code() {
        let err = BurlError::UserError("bad argument".to_string());
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn validation_error_has_correct_exit_code() {
        let err = BurlError::ValidationError("scope violation".to_string());
        assert_eq!(err.exit_code(), exit_codes::VALIDATION_FAILURE);
    }

    #[test]
    fn git_error_has_correct_exit_code() {
        let err = BurlError::GitError("branch creation failed".to_string());
        assert_eq!(err.exit_code(), exit_codes::GIT_FAILURE);
    }

    #[test]
    fn lock_error_has_correct_exit_code() {
        let err = BurlError::LockError("task locked".to_string());
        assert_eq!(err.exit_code(), exit_codes::LOCK_FAILURE);
    }

    #[test]
    fn error_messages_are_descriptive() {
        let err = BurlError::NotImplemented("burl init".to_string());
        assert_eq!(err.to_string(), "burl init is not yet implemented");

        let err = BurlError::ValidationError("stub patterns found".to_string());
        assert_eq!(err.to_string(), "Validation failed: stub patterns found");
    }
}
