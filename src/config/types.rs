//! Configuration types and defaults for burl.
//!
//! This module defines enums, constants, and default value functions
//! used by the Config struct.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Merge strategy for task branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Rebase onto main, then fast-forward merge (default, safest).
    #[default]
    RebaseFfOnly,
    /// Fast-forward merge only (no rebase).
    FfOnly,
    /// Manual merge (no automatic merge).
    Manual,
}

impl MergeStrategy {
    /// Parse a merge strategy from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "rebase_ff_only" => Some(Self::RebaseFfOnly),
            "ff_only" => Some(Self::FfOnly),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// Conflict policy when declared scopes overlap between tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Fail when overlaps are detected (default, safest).
    #[default]
    Fail,
    /// Warn but allow overlapping claims.
    Warn,
    /// Ignore overlaps entirely.
    Ignore,
}

impl ConflictPolicy {
    /// Parse a conflict policy from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fail" => Some(Self::Fail),
            "warn" => Some(Self::Warn),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }
}

/// Mode for detecting scope conflicts at claim time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictDetectionMode {
    /// Use declared task scopes only (V1 behavior).
    #[default]
    Declared,
    /// Use only actual changed files in DOING task worktrees (diff vs base_sha).
    Diff,
    /// Prefer actual diffs when available; fallback to declared scopes when a DOING task has no diff.
    Hybrid,
}

/// Default stub patterns for detecting incomplete code.
pub fn default_stub_patterns() -> Vec<String> {
    vec![
        "TODO".to_string(),
        "FIXME".to_string(),
        "XXX".to_string(),
        "HACK".to_string(),
        "unimplemented!".to_string(),
        "todo!".to_string(),
        r#"panic!\s*\(\s*"not implemented"#.to_string(),
        "NotImplementedError".to_string(),
        "raise NotImplemented".to_string(),
        r"^\s*pass\s*$".to_string(),
        r"^\s*\.\.\.\s*$".to_string(),
    ]
}

/// Default file extensions for stub checking.
pub fn default_stub_check_extensions() -> Vec<String> {
    vec![
        "rs".to_string(),
        "py".to_string(),
        "ts".to_string(),
        "js".to_string(),
        "tsx".to_string(),
        "jsx".to_string(),
    ]
}

// Default value functions for serde
pub(crate) fn default_max_parallel() -> u32 {
    3
}
pub(crate) fn default_workflow_branch() -> String {
    "burl".to_string()
}
pub(crate) fn default_workflow_worktree() -> String {
    ".burl".to_string()
}
pub(crate) fn default_main_branch() -> String {
    "main".to_string()
}
pub(crate) fn default_remote() -> String {
    "origin".to_string()
}
pub(crate) fn default_lock_stale_minutes() -> u32 {
    120
}
pub(crate) fn default_qa_max_attempts() -> u32 {
    3
}
pub(crate) fn default_build_command() -> String {
    "cargo test".to_string()
}
pub(crate) fn default_true() -> bool {
    true
}

/// A named validation profile consisting of ordered command steps.
///
/// Profiles are selected by:
/// - Task frontmatter `validation_profile` (if set), otherwise
/// - Config `default_validation_profile` (if set), otherwise
/// - Fallback to legacy `build_command`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidationProfile {
    /// Ordered command steps to run.
    pub steps: Vec<ValidationCommandStep>,

    /// Unknown fields preserved for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

/// A single command step in a validation profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidationCommandStep {
    /// Display name for the step (e.g., "cargo test", "npm test", "fmt").
    pub name: String,

    /// Command to execute (shell-words parsed; no shell).
    pub command: String,

    /// Only run this step if any changed file matches one of these globs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_if_changed_globs: Vec<String>,

    /// Only run this step if any changed file has one of these extensions (no leading dots).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_if_changed_extensions: Vec<String>,

    /// Unknown fields preserved for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}
