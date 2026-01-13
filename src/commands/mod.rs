//! Command implementations for burl.
//!
//! This module provides the dispatcher that routes CLI commands to their
//! implementations. The `init` command is fully implemented; other commands
//! are currently stubbed with "not implemented" messages.

mod init;

use crate::cli::{
    AddArgs, ApproveArgs, ClaimArgs, CleanArgs, Command, DoctorArgs, LockAction, LockClearArgs,
    LockCommand, RejectArgs, ShowArgs, SubmitArgs, ValidateArgs, WorktreeArgs,
};
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{append_event, Event, EventAction};
use crate::locks;
use serde_json::json;

/// Dispatch a command to its implementation.
///
/// This is the main entry point for command execution. Each command
/// is routed to its handler function.
pub fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Init => init::cmd_init(),
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
// Most commands below are stubbed and will be implemented in later tasks.
// Each stub returns a NotImplemented error with exit code 1.
// The `init` command is implemented in the `init` module.

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
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    let locks = locks::list_locks(&ctx, &config)?;

    if locks.is_empty() {
        println!("No active locks.");
        return Ok(());
    }

    println!("Active locks ({}):", locks.len());
    println!();

    for lock in &locks {
        println!(
            "  {} ({}):",
            lock.name,
            match lock.lock_type {
                locks::LockType::Workflow => "workflow",
                locks::LockType::Task => "task",
                locks::LockType::Claim => "claim",
            }
        );
        println!("    Owner:      {}", lock.metadata.owner);
        if let Some(pid) = lock.metadata.pid {
            println!("    PID:        {}", pid);
        }
        println!("    Created:    {}", lock.metadata.created_at.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("    Age:        {}", lock.metadata.age_string());
        println!("    Action:     {}", lock.metadata.action);
        if lock.is_stale {
            println!("    Status:     STALE (exceeds {} min threshold)", config.lock_stale_minutes);
        }
        println!("    Path:       {}", lock.path.display());
        println!();
    }

    // Summary
    let stale_count = locks.iter().filter(|l| l.is_stale).count();
    if stale_count > 0 {
        println!(
            "Note: {} lock(s) are stale. Use `burl lock clear <lock-id> --force` to clear.",
            stale_count
        );
    }

    Ok(())
}

fn cmd_lock_clear(args: LockClearArgs) -> Result<()> {
    // Require --force flag
    if !args.force {
        return Err(BurlError::UserError(
            "refusing to clear lock without --force flag.\n\n\
             Clearing locks can cause data corruption if the lock holder is still active.\n\
             Only clear locks if you are certain the lock holder has crashed.\n\n\
             To clear the lock, run:\n  burl lock clear {} --force"
                .replace("{}", &args.lock_id),
        ));
    }

    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    let cleared = locks::clear_lock(&ctx, &args.lock_id, &config)?;

    // Append lock_clear event
    // For workflow.lock, logging is best-effort since we just cleared it and can now acquire it.
    // For other locks, we should be able to log normally.
    let event = Event::new(EventAction::LockClear).with_details(json!({
        "lock_id": cleared.name,
        "lock_type": match cleared.lock_type {
            locks::LockType::Workflow => "workflow",
            locks::LockType::Task => "task",
            locks::LockType::Claim => "claim",
        },
        "age_minutes": cleared.metadata.age().num_minutes(),
        "was_stale": cleared.is_stale,
        "force": args.force,
        "owner": cleared.metadata.owner,
        "original_action": cleared.metadata.action
    }));

    // Best-effort logging: if it fails, print a warning but don't fail the command
    // This is especially important when clearing workflow.lock since we need the lock
    // to be cleared before we can acquire it for logging, but we just cleared it.
    if let Err(e) = append_event(&ctx, &event) {
        eprintln!("Warning: failed to log lock_clear event: {}", e);
    }

    println!("Cleared lock: {}", cleared.name);
    println!();
    println!("Lock details:");
    println!("  Owner:      {}", cleared.metadata.owner);
    if let Some(pid) = cleared.metadata.pid {
        println!("  PID:        {}", pid);
    }
    println!("  Created:    {}", cleared.metadata.created_at.format("%Y-%m-%d %H:%M:%S UTC"));
    println!("  Age:        {}", cleared.metadata.age_string());
    println!("  Action:     {}", cleared.metadata.action);
    if cleared.is_stale {
        println!("  Status:     was STALE");
    }
    println!("  Path:       {}", cleared.path.display());

    Ok(())
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

    // Note: init is now fully implemented and tested in the init module.
    // The dispatch test for init requires a git repo, so it's tested in init module.

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
    fn lock_list_fails_without_initialized_workflow() {
        // lock_list now requires an initialized workflow
        // When run outside a git repo or without workflow, it should fail with UserError
        let result = cmd_lock_list();
        assert!(result.is_err());
        // Either "not inside a git repository" or "not initialized"
        assert_eq!(result.unwrap_err().exit_code(), exit_codes::USER_ERROR);
    }

    #[test]
    fn lock_clear_refuses_without_force() {
        let args = LockClearArgs {
            lock_id: "TASK-001".to_string(),
            force: false,
        };
        let result = cmd_lock_clear(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), exit_codes::USER_ERROR);
        assert!(err.to_string().contains("--force"));
    }

    #[test]
    fn lock_clear_fails_without_initialized_workflow() {
        let args = LockClearArgs {
            lock_id: "TASK-001".to_string(),
            force: true,
        };
        let result = cmd_lock_clear(args);
        assert!(result.is_err());
        // Either "not inside a git repository" or "not initialized"
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
        // Test that dispatch correctly routes stubbed commands
        // (init is tested separately in the init module since it needs a real git repo)
        let result = dispatch(Command::Status);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("burl status"));
    }
}
