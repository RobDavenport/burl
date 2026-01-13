//! Exit code constants for the burl CLI.
//!
//! These codes follow the V1 specification:
//! - 0: Success
//! - 1: User error (bad args, invalid state)
//! - 2: Validation failure (scope/stubs/build)
//! - 3: Git operation failure
//! - 4: Lock acquisition failure

/// Successful execution.
pub const SUCCESS: i32 = 0;

/// User error: bad arguments, invalid state, or unimplemented command.
pub const USER_ERROR: i32 = 1;

/// Validation failure: scope violation, stub patterns found, or build/test failure.
pub const VALIDATION_FAILURE: i32 = 2;

/// Git operation failure: branch creation, worktree, merge, rebase errors.
pub const GIT_FAILURE: i32 = 3;

/// Lock acquisition failure: task or workflow lock could not be acquired.
pub const LOCK_FAILURE: i32 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        let codes = [SUCCESS, USER_ERROR, VALIDATION_FAILURE, GIT_FAILURE, LOCK_FAILURE];
        for (i, &a) in codes.iter().enumerate() {
            for (j, &b) in codes.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Exit codes must be distinct");
                }
            }
        }
    }

    #[test]
    fn exit_codes_match_spec() {
        assert_eq!(SUCCESS, 0);
        assert_eq!(USER_ERROR, 1);
        assert_eq!(VALIDATION_FAILURE, 2);
        assert_eq!(GIT_FAILURE, 3);
        assert_eq!(LOCK_FAILURE, 4);
    }
}
