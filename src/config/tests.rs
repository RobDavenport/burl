//! Tests for config functionality.

use crate::config::types::{default_stub_check_extensions, default_stub_patterns};
use crate::config::{Config, ConflictDetectionMode, ConflictPolicy, MergeStrategy};

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
fn test_parse_conflict_detection_mode() {
    let config = Config::from_yaml("conflict_detection: declared").unwrap();
    assert_eq!(config.conflict_detection, ConflictDetectionMode::Declared);

    let config = Config::from_yaml("conflict_detection: diff").unwrap();
    assert_eq!(config.conflict_detection, ConflictDetectionMode::Diff);

    let config = Config::from_yaml("conflict_detection: hybrid").unwrap();
    assert_eq!(config.conflict_detection, ConflictDetectionMode::Hybrid);
}

#[test]
fn test_parse_validation_profiles() {
    let yaml = r#"
build_command: ""
default_validation_profile: quick
validation_profiles:
  quick:
    steps:
      - name: git-version
        command: "git --version"
      - name: rust-only
        command: "git --version"
        run_if_changed_extensions: [rs]
      - name: docs-only
        command: "git --version"
        run_if_changed_globs: ["docs/**"]
"#;
    let config = Config::from_yaml(yaml).unwrap();

    assert_eq!(config.default_validation_profile.as_deref(), Some("quick"));
    let profile = config.validation_profiles.get("quick").unwrap();
    assert_eq!(profile.steps.len(), 3);
    assert_eq!(profile.steps[0].name, "git-version");
    assert_eq!(profile.steps[0].command, "git --version");
    assert_eq!(profile.steps[1].run_if_changed_extensions, vec!["rs"]);
    assert_eq!(profile.steps[2].run_if_changed_globs, vec!["docs/**"]);
}

#[test]
fn test_validate_default_validation_profile_must_exist() {
    let yaml = r#"
default_validation_profile: quick
validation_profiles: {}
"#;
    let result = Config::from_yaml(yaml);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("default_validation_profile"));
    assert!(err.to_string().contains("not found"));
}

#[test]
fn test_validate_validation_profile_duplicate_step_names_fail() {
    let yaml = r#"
build_command: ""
validation_profiles:
  quick:
    steps:
      - name: lint
        command: "git --version"
      - name: lint
        command: "git --version"
"#;
    let result = Config::from_yaml(yaml);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("duplicate step name"));
}

#[test]
fn test_validate_validation_profile_run_if_changed_extensions_no_leading_dot() {
    let yaml = r#"
build_command: ""
validation_profiles:
  quick:
    steps:
      - name: rust-only
        command: "git --version"
        run_if_changed_extensions: [.rs]
"#;
    let result = Config::from_yaml(yaml);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("run_if_changed_extensions"));
    assert!(err.to_string().contains("leading dots"));
}

#[test]
fn test_validate_validation_profile_run_if_changed_globs_must_be_valid() {
    let yaml = r#"
build_command: ""
validation_profiles:
  quick:
    steps:
      - name: globs
        command: "git --version"
        run_if_changed_globs: ["["]
"#;
    let result = Config::from_yaml(yaml);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("invalid glob"));
}

#[test]
fn test_parse_yaml_with_unknown_fields() {
    // Unknown fields should be silently ignored for forward compatibility
    let yaml = r#"
max_parallel: 5
unknown_field: "some value"
another_unknown:
  nested: true
future_feature_x: enabled
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
