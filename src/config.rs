//! Configuration model for burl.
//!
//! This module defines the Config struct that represents `.burl/.workflow/config.yaml`.
//! It supports forward-compatible YAML parsing (unknown fields are ignored),
//! sensible defaults for optional fields, and validation of config values.

use crate::error::{BurlError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

// Default value functions for serde
fn default_max_parallel() -> u32 {
    3
}
fn default_workflow_branch() -> String {
    "burl".to_string()
}
fn default_workflow_worktree() -> String {
    ".burl".to_string()
}
fn default_main_branch() -> String {
    "main".to_string()
}
fn default_remote() -> String {
    "origin".to_string()
}
fn default_lock_stale_minutes() -> u32 {
    120
}
fn default_qa_max_attempts() -> u32 {
    3
}
fn default_build_command() -> String {
    "cargo test".to_string()
}
fn default_true() -> bool {
    true
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

impl Config {
    /// Load config from a YAML file.
    ///
    /// Unknown fields in the YAML are silently ignored for forward compatibility.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the config.yaml file
    ///
    /// # Returns
    ///
    /// * `Ok(Config)` - Successfully loaded and validated config
    /// * `Err(BurlError::UserError)` - Parse error or validation failure
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        let content = std::fs::read_to_string(path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read config file '{}': {}",
                path.display(),
                e
            ))
        })?;

        Self::from_yaml(&content)
    }

    /// Parse config from a YAML string.
    ///
    /// Unknown fields in the YAML are silently ignored for forward compatibility.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let config: Config = serde_yaml::from_str(yaml)
            .map_err(|e| BurlError::UserError(format!("failed to parse config YAML: {}", e)))?;

        config.validate()?;
        Ok(config)
    }

    /// Serialize config to YAML string.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize config to YAML: {}", e)))
    }

    /// Validate config values and return error on invalid values.
    ///
    /// Validation rules:
    /// - `lock_stale_minutes` must be positive
    /// - `qa_max_attempts` must be positive
    /// - `stub_check_extensions` entries must be non-empty and have no leading dots
    pub fn validate(&self) -> Result<()> {
        // Validate lock_stale_minutes
        if self.lock_stale_minutes == 0 {
            return Err(BurlError::UserError(
                "config validation failed: lock_stale_minutes must be greater than 0".to_string(),
            ));
        }

        // Validate qa_max_attempts
        if self.qa_max_attempts == 0 {
            return Err(BurlError::UserError(
                "config validation failed: qa_max_attempts must be greater than 0".to_string(),
            ));
        }

        // Validate stub_check_extensions
        for ext in &self.stub_check_extensions {
            if ext.is_empty() {
                return Err(BurlError::UserError(
                    "config validation failed: stub_check_extensions entries must be non-empty"
                        .to_string(),
                ));
            }
            if ext.starts_with('.') {
                return Err(BurlError::UserError(format!(
                    "config validation failed: stub_check_extensions entries must not have leading dots (found '{}'). Use '{}' instead.",
                    ext,
                    ext.trim_start_matches('.')
                )));
            }
        }

        Ok(())
    }

    /// Get stub_check_extensions normalized to lowercase.
    pub fn normalized_extensions(&self) -> Vec<String> {
        self.stub_check_extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.max_parallel, 3);
        assert_eq!(config.workflow_branch, "burl");
        assert_eq!(config.workflow_worktree, ".burl");
        assert!(config.workflow_auto_commit);
        assert!(!config.workflow_auto_push);
        assert_eq!(config.main_branch, "main");
        assert_eq!(config.remote, "origin");
        assert_eq!(config.merge_strategy, MergeStrategy::RebaseFfOnly);
        assert!(!config.push_main_on_approve);
        assert_eq!(config.lock_stale_minutes, 120);
        assert!(config.use_global_claim_lock);
        assert_eq!(config.qa_max_attempts, 3);
        assert!(config.auto_priority_boost_on_retry);
        assert_eq!(config.build_command, "cargo test");
        assert!(!config.stub_patterns.is_empty());
        assert!(!config.stub_check_extensions.is_empty());
        assert_eq!(config.conflict_policy, ConflictPolicy::Fail);
    }

    #[test]
    fn test_parse_minimal_yaml() {
        let yaml = "";
        let config = Config::from_yaml(yaml).unwrap();

        // Should use all defaults
        assert_eq!(config.max_parallel, 3);
        assert_eq!(config.workflow_branch, "burl");
    }

    #[test]
    fn test_parse_partial_yaml() {
        let yaml = r#"
max_parallel: 5
main_branch: master
"#;
        let config = Config::from_yaml(yaml).unwrap();

        // Specified values should be used
        assert_eq!(config.max_parallel, 5);
        assert_eq!(config.main_branch, "master");

        // Unspecified values should use defaults
        assert_eq!(config.workflow_branch, "burl");
        assert_eq!(config.remote, "origin");
    }

    #[test]
    fn test_parse_full_yaml() {
        let yaml = r#"
max_parallel: 10
workflow_branch: workflow
workflow_worktree: .workflow
workflow_auto_commit: false
workflow_auto_push: true
main_branch: develop
remote: upstream
merge_strategy: ff_only
push_main_on_approve: true
push_task_branch_on_submit: true
lock_stale_minutes: 60
use_global_claim_lock: false
qa_max_attempts: 5
auto_priority_boost_on_retry: false
build_command: "npm test"
stub_patterns:
  - "TODO"
  - "FIXME"
stub_check_extensions:
  - ts
  - js
conflict_policy: warn
"#;
        let config = Config::from_yaml(yaml).unwrap();

        assert_eq!(config.max_parallel, 10);
        assert_eq!(config.workflow_branch, "workflow");
        assert_eq!(config.workflow_worktree, ".workflow");
        assert!(!config.workflow_auto_commit);
        assert!(config.workflow_auto_push);
        assert_eq!(config.main_branch, "develop");
        assert_eq!(config.remote, "upstream");
        assert_eq!(config.merge_strategy, MergeStrategy::FfOnly);
        assert!(config.push_main_on_approve);
        assert!(config.push_task_branch_on_submit);
        assert_eq!(config.lock_stale_minutes, 60);
        assert!(!config.use_global_claim_lock);
        assert_eq!(config.qa_max_attempts, 5);
        assert!(!config.auto_priority_boost_on_retry);
        assert_eq!(config.build_command, "npm test");
        assert_eq!(config.stub_patterns, vec!["TODO", "FIXME"]);
        assert_eq!(config.stub_check_extensions, vec!["ts", "js"]);
        assert_eq!(config.conflict_policy, ConflictPolicy::Warn);
    }

    #[test]
    fn test_parse_yaml_with_unknown_fields() {
        // Unknown fields should be silently ignored for forward compatibility
        let yaml = r#"
max_parallel: 5
unknown_field: "some value"
another_unknown:
  nested: true
future_feature_v2: enabled
"#;
        let config = Config::from_yaml(yaml).unwrap();

        // Known field should be parsed
        assert_eq!(config.max_parallel, 5);

        // Should not fail due to unknown fields
        // and defaults should apply for unspecified known fields
        assert_eq!(config.workflow_branch, "burl");
    }

    #[test]
    fn test_validate_zero_lock_stale_minutes() {
        let yaml = "lock_stale_minutes: 0";
        let result = Config::from_yaml(yaml);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("lock_stale_minutes"));
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn test_validate_zero_qa_max_attempts() {
        let yaml = "qa_max_attempts: 0";
        let result = Config::from_yaml(yaml);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("qa_max_attempts"));
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn test_validate_empty_stub_extension() {
        let yaml = r#"
stub_check_extensions:
  - rs
  - ""
  - py
"#;
        let result = Config::from_yaml(yaml);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("stub_check_extensions"));
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn test_validate_stub_extension_with_leading_dot() {
        let yaml = r#"
stub_check_extensions:
  - .rs
  - py
"#;
        let result = Config::from_yaml(yaml);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("stub_check_extensions"));
        assert!(err.to_string().contains("leading dots"));
        assert!(err.to_string().contains(".rs"));
        assert!(err.to_string().contains("Use 'rs' instead"));
    }

    #[test]
    fn test_merge_strategy_parsing() {
        let yaml = "merge_strategy: rebase_ff_only";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.merge_strategy, MergeStrategy::RebaseFfOnly);

        let yaml = "merge_strategy: ff_only";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.merge_strategy, MergeStrategy::FfOnly);

        let yaml = "merge_strategy: manual";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.merge_strategy, MergeStrategy::Manual);
    }

    #[test]
    fn test_conflict_policy_parsing() {
        let yaml = "conflict_policy: fail";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.conflict_policy, ConflictPolicy::Fail);

        let yaml = "conflict_policy: warn";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.conflict_policy, ConflictPolicy::Warn);

        let yaml = "conflict_policy: ignore";
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.conflict_policy, ConflictPolicy::Ignore);
    }

    #[test]
    fn test_merge_strategy_from_str() {
        assert_eq!(
            MergeStrategy::from_str("rebase_ff_only"),
            Some(MergeStrategy::RebaseFfOnly)
        );
        assert_eq!(
            MergeStrategy::from_str("ff_only"),
            Some(MergeStrategy::FfOnly)
        );
        assert_eq!(
            MergeStrategy::from_str("manual"),
            Some(MergeStrategy::Manual)
        );
        assert_eq!(MergeStrategy::from_str("invalid"), None);
    }

    #[test]
    fn test_conflict_policy_from_str() {
        assert_eq!(ConflictPolicy::from_str("fail"), Some(ConflictPolicy::Fail));
        assert_eq!(ConflictPolicy::from_str("warn"), Some(ConflictPolicy::Warn));
        assert_eq!(
            ConflictPolicy::from_str("ignore"),
            Some(ConflictPolicy::Ignore)
        );
        assert_eq!(ConflictPolicy::from_str("invalid"), None);
    }

    #[test]
    fn test_normalized_extensions() {
        let yaml = r#"
stub_check_extensions:
  - RS
  - Py
  - TsX
"#;
        let config = Config::from_yaml(yaml).unwrap();
        let normalized = config.normalized_extensions();

        assert_eq!(normalized, vec!["rs", "py", "tsx"]);
    }

    #[test]
    fn test_to_yaml() {
        let config = Config::default();
        let yaml = config.to_yaml().unwrap();

        // Should be valid YAML that can be parsed back
        let parsed = Config::from_yaml(&yaml).unwrap();
        assert_eq!(parsed.max_parallel, config.max_parallel);
        assert_eq!(parsed.workflow_branch, config.workflow_branch);
    }

    #[test]
    fn test_config_load_from_file() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "max_parallel: 7").unwrap();
        writeln!(file, "main_branch: trunk").unwrap();

        let config = Config::load(file.path()).unwrap();
        assert_eq!(config.max_parallel, 7);
        assert_eq!(config.main_branch, "trunk");
    }

    #[test]
    fn test_config_load_missing_file() {
        let result = Config::load("/nonexistent/path/config.yaml");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("failed to read config file"));
    }

    #[test]
    fn test_default_stub_patterns_not_empty() {
        let patterns = default_stub_patterns();
        assert!(!patterns.is_empty());
        assert!(patterns.contains(&"TODO".to_string()));
        assert!(patterns.contains(&"FIXME".to_string()));
    }

    #[test]
    fn test_default_stub_check_extensions_not_empty() {
        let extensions = default_stub_check_extensions();
        assert!(!extensions.is_empty());
        assert!(extensions.contains(&"rs".to_string()));
        assert!(extensions.contains(&"py".to_string()));
    }
}
