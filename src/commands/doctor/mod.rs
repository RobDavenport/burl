//! Implementation of the `burl doctor` command.
//!
//! Diagnoses workflow health and optionally repairs detected issues.
//!
//! # Read-only mode (default)
//!
//! Reports:
//! - Stale locks (based on config threshold)
//! - Orphan lock files (lock exists but task doesn't)
//! - Tasks in DOING/QA missing `base_sha`
//! - Tasks in DOING/QA with missing worktree directory
//! - Orphan worktrees under `.worktrees/` not referenced by any task
//! - Tasks that reference a branch that does not exist locally
//! - Bucket/metadata mismatches (e.g., READY task with `started_at` set)
//!
//! # Repair mode (`--repair --force`)
//!
//! Safe repairs only:
//! - Clear stale locks
//! - Recreate missing directories (`locks/`, `events/`)
//! - Fix bucket placement based on metadata

mod checks;
mod display;
mod repairs;

#[cfg(test)]
mod tests;

use crate::cli::DoctorArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};

pub use checks::*;
pub use display::*;
pub use repairs::*;

/// Severity level for issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Warning: potential problem but not critical.
    Warning,
    /// Error: something is wrong and should be fixed.
    Error,
}

impl std::fmt::Display for IssueSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueSeverity::Warning => write!(f, "WARNING"),
            IssueSeverity::Error => write!(f, "ERROR"),
        }
    }
}

/// A detected issue with a recommended fix.
#[derive(Debug, Clone)]
pub struct Issue {
    /// Severity level.
    pub severity: IssueSeverity,
    /// Category of the issue.
    pub category: String,
    /// Description of the issue.
    pub description: String,
    /// Path or identifier involved.
    pub path: Option<String>,
    /// Recommended remediation command or action.
    pub remediation: Option<String>,
    /// Whether this issue can be auto-repaired.
    pub repairable: bool,
}

impl Issue {
    pub fn new(severity: IssueSeverity, category: &str, description: &str) -> Self {
        Self {
            severity,
            category: category.to_string(),
            description: description.to_string(),
            path: None,
            remediation: None,
            repairable: false,
        }
    }

    pub fn with_path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }

    pub fn with_remediation(mut self, remediation: &str) -> Self {
        self.remediation = Some(remediation.to_string());
        self
    }

    pub fn repairable(mut self) -> Self {
        self.repairable = true;
        self
    }
}

/// Result of running the doctor check.
pub struct DoctorReport {
    /// List of detected issues.
    pub issues: Vec<Issue>,
    /// List of repairs that were performed (in repair mode).
    pub repairs: Vec<String>,
}

impl DoctorReport {
    pub fn new() -> Self {
        Self {
            issues: Vec::new(),
            repairs: Vec::new(),
        }
    }

    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    #[allow(dead_code)]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }
}

/// Execute the `burl doctor` command.
pub fn cmd_doctor(args: DoctorArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    // Validate repair args
    if args.repair && !args.force {
        return Err(BurlError::UserError(
            "refusing to repair without --force flag.\n\n\
             Repairs may modify workflow state. Please review the issues first with `burl doctor`,\n\
             then run `burl doctor --repair --force` to apply safe repairs."
                .to_string(),
        ));
    }

    let mut report = DoctorReport::new();

    // Run all checks
    check_missing_directories(&ctx, &mut report)?;
    check_stale_locks(&ctx, &config, &mut report)?;
    check_orphan_locks(&ctx, &config, &mut report)?;
    check_tasks_missing_base_sha(&ctx, &mut report)?;
    check_tasks_missing_worktree(&ctx, &mut report)?;
    check_orphan_worktrees(&ctx, &mut report)?;
    check_tasks_missing_branch(&ctx, &mut report)?;
    check_bucket_metadata_mismatches(&ctx, &mut report)?;

    // If repair mode, apply safe repairs
    if args.repair && args.force {
        apply_repairs(&ctx, &config, &mut report)?;
    }

    // Print report
    print_report(&report, args.repair);

    // Exit code: 0 if healthy (no issues or all repaired), 1 if issues remain
    if !args.repair && report.has_issues() {
        // Return error to indicate issues were found (exit code 1)
        return Err(BurlError::UserError(format!(
            "Found {} issue(s). Run `burl doctor --repair --force` to apply safe repairs.",
            report.issues.len()
        )));
    }

    // In repair mode, check if there are still unrepaired issues
    let remaining_issues: Vec<_> = report.issues.iter().filter(|i| !i.repairable).collect();

    if args.repair && !remaining_issues.is_empty() {
        return Err(BurlError::UserError(format!(
            "Repairs applied, but {} issue(s) remain that cannot be auto-repaired.",
            remaining_issues.len()
        )));
    }

    if report.has_issues() && !args.repair {
        println!();
    }

    Ok(())
}
