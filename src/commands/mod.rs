//! Command implementations for burl.
//!
//! This module provides the dispatcher that routes CLI commands to their
//! implementations. Currently all commands are stubbed with "not implemented"
//! messages and exit code 1.

use crate::cli::{
    AddArgs, ApproveArgs, ClaimArgs, CleanArgs, Command, DoctorArgs, LockAction, LockClearArgs,
    LockCommand, RejectArgs, ShowArgs, SubmitArgs, ValidateArgs, WorktreeArgs,
};
use crate::error::{BurlError, Result};

/// Dispatch a command to its implementation.
///
/// This is the main entry point for command execution. Each command
/// is routed to its handler function.
pub fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Init => cmd_init(),
        Command::Add(args) => cmd_add(args),
        Command::Status => cmd_status(),
        Command::Show(args) => cmd_show(args),
        Command::Claim(args) => cmd_claim(args),
        Command::Submit(args) => cmd_submit(args),
        Command::Validate(args) => cmd_validate(args),
        Command::Approve(args) => cmd_approve(args),
        Command::Reject(args) => cmd_reject(args),
        Command::Worktree(args) => cmd_worktree(args),
        Command::Lock(lock_cmd) => dispatch_lock(lock_cmd),
        Command::Doctor(args) => cmd_doctor(args),
        Command::Clean(args) => cmd_clean(args),
    }
}

/// Dispatch lock subcommands.
fn dispatch_lock(lock_cmd: LockCommand) -> Result<()> {
    match lock_cmd.action {
        LockAction::List => cmd_lock_list(),
        LockAction::Clear(args) => cmd_lock_clear(args),
    }
}

// ============================================================================
// Command Implementations (Stubs)
// ============================================================================
// All commands below are stubbed and will be implemented in later tasks.
// Each stub returns a NotImplemented error with exit code 1.

fn cmd_init() -> Result<()> {
    Err(BurlError::NotImplemented("burl init".to_string()))
}

fn cmd_add(_args: AddArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl add".to_string()))
}

fn cmd_status() -> Result<()> {
    Err(BurlError::NotImplemented("burl status".to_string()))
}

fn cmd_show(_args: ShowArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl show".to_string()))
}

fn cmd_claim(_args: ClaimArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl claim".to_string()))
}

fn cmd_submit(_args: SubmitArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl submit".to_string()))
}

fn cmd_validate(_args: ValidateArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl validate".to_string()))
}

fn cmd_approve(_args: ApproveArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl approve".to_string()))
}

fn cmd_reject(_args: RejectArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl reject".to_string()))
}

fn cmd_worktree(_args: WorktreeArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl worktree".to_string()))
}

fn cmd_lock_list() -> Result<()> {
    Err(BurlError::NotImplemented("burl lock list".to_string()))
}

fn cmd_lock_clear(_args: LockClearArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl lock clear".to_string()))
}

fn cmd_doctor(_args: DoctorArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl doctor".to_string()))
}

fn cmd_clean(_args: CleanArgs) -> Result<()> {
    Err(BurlError::NotImplemented("burl clean".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit_codes;

    #[test]
    fn init_returns_not_implemented() {
        let result = cmd_init();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn add_returns_not_implemented() {
        let args = AddArgs {
            title: "Test".to_string(),
            priority: "high".to_string(),
            affects: vec![],
            affects_globs: vec![],
            must_not_touch: vec![],
            depends_on: vec![],
            tags: vec![],
        };
        let result = cmd_add(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn status_returns_not_implemented() {
        let result = cmd_status();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn show_returns_not_implemented() {
        let args = ShowArgs {
            task_id: "TASK-001".to_string(),
        };
        let result = cmd_show(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn claim_returns_not_implemented() {
        let args = ClaimArgs { task_id: None };
        let result = cmd_claim(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn submit_returns_not_implemented() {
        let args = SubmitArgs { task_id: None };
        let result = cmd_submit(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn validate_returns_not_implemented() {
        let args = ValidateArgs {
            task_id: "TASK-001".to_string(),
        };
        let result = cmd_validate(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn approve_returns_not_implemented() {
        let args = ApproveArgs {
            task_id: "TASK-001".to_string(),
        };
        let result = cmd_approve(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn reject_returns_not_implemented() {
        let args = RejectArgs {
            task_id: "TASK-001".to_string(),
            reason: "Test reason".to_string(),
        };
        let result = cmd_reject(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn worktree_returns_not_implemented() {
        let args = WorktreeArgs { task_id: None };
        let result = cmd_worktree(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn lock_list_returns_not_implemented() {
        let result = cmd_lock_list();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn lock_clear_returns_not_implemented() {
        let args = LockClearArgs {
            lock_id: "TASK-001".to_string(),
            force: true,
        };
        let result = cmd_lock_clear(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn doctor_returns_not_implemented() {
        let args = DoctorArgs {
            repair: false,
            force: false,
        };
        let result = cmd_doctor(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn clean_returns_not_implemented() {
        let args = CleanArgs {
            completed: false,
            orphans: false,
            yes: false,
        };
        let result = cmd_clean(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn dispatch_routes_to_correct_handler() {
        // Test that dispatch correctly routes each command type
        let result = dispatch(Command::Init);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("burl init"));

        let result = dispatch(Command::Status);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("burl status"));
    }
}
