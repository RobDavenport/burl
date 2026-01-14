//! Config struct definition and default implementation.

use super::types::*;
use serde::{Deserialize, Serialize};

/// Configuration for the burl workflow.
///
/// This struct represents the contents of `.burl/.workflow/config.yaml`.
/// Unknown fields in the YAML are ignored for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // =========================================================================
    // Workflow settings
    // =========================================================================
    /// Maximum parallel tasks (informational, not enforced in V1).
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,

    /// Name of the workflow branch (default: "burl").
    /// Note: In V1, this is read but does not relocate the workflow worktree.
    #[serde(default = "default_workflow_branch")]
    pub workflow_branch: String,

    /// Path to the workflow worktree relative to repo root (default: ".burl").
    /// Note: In V1, this is read but does not relocate the workflow worktree.
    #[serde(default = "default_workflow_worktree")]
    pub workflow_worktree: String,

    /// Whether to auto-commit workflow state changes after transitions.
    #[serde(default = "default_true")]
    pub workflow_auto_commit: bool,

    /// Whether to auto-push workflow branch after commits.
    #[serde(default)]
    pub workflow_auto_push: bool,

    // =========================================================================
    // Git settings
    // =========================================================================
    /// Name of the main branch (default: "main").
    #[serde(default = "default_main_branch")]
    pub main_branch: String,

    /// Name of the remote (default: "origin").
    #[serde(default = "default_remote")]
    pub remote: String,

    /// Merge strategy for task branches.
    #[serde(default)]
    pub merge_strategy: MergeStrategy,

    /// Whether to push main after approving a task.
    #[serde(default)]
    pub push_main_on_approve: bool,

    /// Whether to push task branches on submit.
    #[serde(default)]
    pub push_task_branch_on_submit: bool,

    // =========================================================================
    // Lock settings
    // =========================================================================
    /// Minutes after which a lock is considered stale.
    #[serde(default = "default_lock_stale_minutes")]
    pub lock_stale_minutes: u32,

    /// Whether to use a global claim lock for "claim next" operations.
    #[serde(default = "default_true")]
    pub use_global_claim_lock: bool,

    // =========================================================================
    // QA settings
    // =========================================================================
    /// Maximum QA attempts before moving to BLOCKED.
    #[serde(default = "default_qa_max_attempts")]
    pub qa_max_attempts: u32,

    /// Whether to boost priority on QA retry.
    #[serde(default = "default_true")]
    pub auto_priority_boost_on_retry: bool,

    // =========================================================================
    // Validation settings
    // =========================================================================
    /// Build/test command to run during validation (empty disables).
    #[serde(default = "default_build_command")]
    pub build_command: String,

    /// Regex patterns for detecting stubs in added lines.
    #[serde(default = "default_stub_patterns")]
    pub stub_patterns: Vec<String>,

    /// File extensions to check for stubs (no leading dots).
    #[serde(default = "default_stub_check_extensions")]
    pub stub_check_extensions: Vec<String>,

    // =========================================================================
    // Conflict settings
    // =========================================================================
    /// Policy when declared scopes overlap between tasks.
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_parallel: default_max_parallel(),
            workflow_branch: default_workflow_branch(),
            workflow_worktree: default_workflow_worktree(),
            workflow_auto_commit: default_true(),
            workflow_auto_push: false,
            main_branch: default_main_branch(),
            remote: default_remote(),
            merge_strategy: MergeStrategy::default(),
            push_main_on_approve: false,
            push_task_branch_on_submit: false,
            lock_stale_minutes: default_lock_stale_minutes(),
            use_global_claim_lock: default_true(),
            qa_max_attempts: default_qa_max_attempts(),
            auto_priority_boost_on_retry: default_true(),
            build_command: default_build_command(),
            stub_patterns: default_stub_patterns(),
            stub_check_extensions: default_stub_check_extensions(),
            conflict_policy: ConflictPolicy::default(),
        }
    }
}
