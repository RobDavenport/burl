//! Command implementations for burl.
//!
//! This module provides the dispatcher that routes CLI commands to their
//! implementations.

pub mod add;
pub mod approve;
pub mod claim;
pub mod init;
pub mod reject;
mod show;
mod status;
pub mod submit;
pub mod validate_cmd;
mod worktree;

use crate::cli::{
    ApproveArgs, ClaimArgs, CleanArgs, Command, DoctorArgs, LockAction, LockClearArgs,
    LockCommand, RejectArgs, SubmitArgs, ValidateArgs,
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
        Command::Add(args) => add::cmd_add(args),
        Command::Status => status::cmd_status(),
        Command::Show(args) => show::cmd_show(args),
        Command::Claim(args) => cmd_claim(args),
        Command::Submit(args) => cmd_submit(args),
        Command::Validate(args) => cmd_validate(args),
        Command::Approve(args) => cmd_approve(args),
        Command::Reject(args) => cmd_reject(args),
        Command::Worktree(args) => worktree::cmd_worktree(args),
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
// Commands below are stubbed and will be implemented in later tasks.
// Each stub returns a NotImplemented error with exit code 1.

fn cmd_claim(args: ClaimArgs) -> Result<()> {
    claim::cmd_claim(args)
}

fn cmd_submit(args: SubmitArgs) -> Result<()> {
    submit::cmd_submit(args)
}

fn cmd_validate(args: ValidateArgs) -> Result<()> {
    validate_cmd::cmd_validate(args)
}

fn cmd_approve(args: ApproveArgs) -> Result<()> {
    approve::cmd_approve(args)
}

fn cmd_reject(args: RejectArgs) -> Result<()> {
    reject::cmd_reject(args)
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

    // Note: init, add, status, show, and worktree are now fully implemented.
    // Integration tests for these commands are in their respective modules.
    // Tests that require a git repo are tested in their individual modules.

    // Note: claim is now fully implemented with tests in claim.rs
    // Note: submit is now fully implemented with tests in submit.rs
    // Note: validate is now fully implemented with tests in validate_cmd.rs
    // Note: approve is now fully implemented with tests in approve.rs
    // Note: reject is now fully implemented with tests in reject.rs

    // Note: lock_list_fails_without_initialized_workflow test was removed
    // because it relied on the current directory not having a workflow,
    // which is fragile when tests run in parallel from the burl repo itself.
    // This behavior is adequately tested by context::tests::test_ensure_initialized_fails_when_not_initialized

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

    // Note: lock_clear_fails_without_initialized_workflow test was removed
    // because it relied on the current directory not having a workflow,
    // which is fragile when tests run in parallel from the burl repo itself.

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
}
