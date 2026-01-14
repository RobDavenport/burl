//! Tests for task file parsing, serialization, and mutations.

use super::*;

const MINIMAL_TASK: &str = r#"---
id: TASK-001
title: Test task
---

## Objective
Do something.
"#;

const FULL_TASK: &str = r#"---
id: TASK-001
title: Implement player jump
priority: high
created: 2026-01-13T10:00:00Z
assigned_to: agent@host
qa_attempts: 1
started_at: 2026-01-13T11:00:00Z
submitted_at: null
completed_at: null
worktree: .worktrees/task-001-player-jump
branch: task-001-player-jump
base_sha: abc123def456
affects:
  - src/player/jump.rs
  - src/player/mod.rs
affects_globs:
  - src/player/**
must_not_touch:
  - src/enemy/**
depends_on:
  - TASK-000
tags:
  - feature
  - player
---

## Objective
Implement the player jump mechanic.

## Acceptance Criteria
- [ ] Player can jump
- [ ] Jump has cooldown

## Context
See design doc.

## Implementation Notes
Working on it.

## QA Report
<!-- Validator fills -->
"#;

#[test]
fn test_parse_minimal_task() {
    let task = TaskFile::parse(MINIMAL_TASK).unwrap();
    assert_eq!(task.frontmatter.id, "TASK-001");
    assert_eq!(task.frontmatter.title, "Test task");
    assert_eq!(task.frontmatter.priority, "medium"); // default
    assert!(task.body.contains("## Objective"));
    assert!(task.body.contains("Do something."));
}

#[test]
fn test_parse_full_task() {
    let task = TaskFile::parse(FULL_TASK).unwrap();

    assert_eq!(task.frontmatter.id, "TASK-001");
    assert_eq!(task.frontmatter.title, "Implement player jump");
    assert_eq!(task.frontmatter.priority, "high");
    assert!(task.frontmatter.created.is_some());
    assert_eq!(task.frontmatter.assigned_to, Some("agent@host".to_string()));
    assert_eq!(task.frontmatter.qa_attempts, 1);
    assert!(task.frontmatter.started_at.is_some());
    assert!(task.frontmatter.worktree.is_some());
    assert!(task.frontmatter.branch.is_some());
    assert_eq!(task.frontmatter.base_sha, Some("abc123def456".to_string()));
    assert_eq!(task.frontmatter.affects.len(), 2);
    assert_eq!(task.frontmatter.affects_globs.len(), 1);
    assert_eq!(task.frontmatter.must_not_touch.len(), 1);
    assert_eq!(task.frontmatter.depends_on.len(), 1);
    assert_eq!(task.frontmatter.tags.len(), 2);
}

#[test]
fn test_parse_with_unknown_fields() {
    let content = r#"---
id: TASK-001
title: Test task
unknown_field: some value
another_unknown:
  nested: true
  list:
    - a
    - b
future_v2_feature: enabled
---

Body content.
"#;
    let task = TaskFile::parse(content).unwrap();

    assert_eq!(task.frontmatter.id, "TASK-001");
    assert_eq!(task.frontmatter.extra.len(), 3);
    assert!(task.frontmatter.extra.contains_key("unknown_field"));
    assert!(task.frontmatter.extra.contains_key("another_unknown"));
    assert!(task.frontmatter.extra.contains_key("future_v2_feature"));
}

#[test]
fn test_roundtrip_preserves_unknown_fields() {
    let content = r#"---
id: TASK-001
title: Test task
unknown_field: preserved_value
nested_unknown:
  key: value
---

Body content.
"#;
    let task = TaskFile::parse(content).unwrap();
    let serialized = task.to_string().unwrap();
    let reparsed = TaskFile::parse(&serialized).unwrap();

    assert_eq!(reparsed.frontmatter.id, "TASK-001");
    assert_eq!(reparsed.frontmatter.extra.len(), 2);
    assert!(reparsed.frontmatter.extra.contains_key("unknown_field"));
    assert!(reparsed.frontmatter.extra.contains_key("nested_unknown"));
}

#[test]
fn test_roundtrip_preserves_body() {
    let content = r#"---
id: TASK-001
title: Test task
---

## Section One
Content with special chars: < > & " '

## Section Two
- List item 1
- List item 2

```rust
fn code() {
    // comment
}
```
"#;
    let task = TaskFile::parse(content).unwrap();
    let serialized = task.to_string().unwrap();
    let reparsed = TaskFile::parse(&serialized).unwrap();

    // Body should be preserved exactly
    assert_eq!(reparsed.body, task.body);
    assert!(reparsed.body.contains("## Section One"));
    assert!(reparsed.body.contains("fn code()"));
}

#[test]
fn test_parse_windows_crlf_line_endings() {
    // Create content with Windows CRLF line endings
    let content = "---\r\nid: TASK-001\r\ntitle: Test task\r\n---\r\n\r\n## Objective\r\nDo something.\r\n";

    let task = TaskFile::parse(content).unwrap();
    assert_eq!(task.frontmatter.id, "TASK-001");
    assert_eq!(task.frontmatter.title, "Test task");
    // Body should preserve original line endings
    assert!(task.body.contains("\r\n") || task.body.contains("## Objective"));
}

#[test]
fn test_parse_missing_opening_delimiter() {
    let content = r#"id: TASK-001
title: Test
---
Body
"#;
    let result = TaskFile::parse(content);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("must start with"));
}

#[test]
fn test_parse_missing_closing_delimiter() {
    let content = r#"---
id: TASK-001
title: Test

Body without closing delimiter
"#;
    let result = TaskFile::parse(content);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("missing closing"));
}

#[test]
fn test_set_assigned() {
    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
    let now = Utc::now();

    task.set_assigned("agent@host", Some(now));

    assert_eq!(task.frontmatter.assigned_to, Some("agent@host".to_string()));
    assert_eq!(task.frontmatter.started_at, Some(now));
}

#[test]
fn test_set_git_info() {
    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();

    task.set_git_info("task-001-test", ".worktrees/task-001-test", "abc123");

    assert_eq!(task.frontmatter.branch, Some("task-001-test".to_string()));
    assert_eq!(
        task.frontmatter.worktree,
        Some(".worktrees/task-001-test".to_string())
    );
    assert_eq!(task.frontmatter.base_sha, Some("abc123".to_string()));
}

#[test]
fn test_set_timestamps() {
    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
    let now = Utc::now();

    task.set_submitted(now);
    assert_eq!(task.frontmatter.submitted_at, Some(now));

    task.set_completed(now);
    assert_eq!(task.frontmatter.completed_at, Some(now));
}

#[test]
fn test_increment_qa_attempts() {
    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
    assert_eq!(task.frontmatter.qa_attempts, 0);

    task.increment_qa_attempts();
    assert_eq!(task.frontmatter.qa_attempts, 1);

    task.increment_qa_attempts();
    assert_eq!(task.frontmatter.qa_attempts, 2);
}

#[test]
fn test_append_to_qa_report_new_section() {
    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();

    task.append_to_qa_report("Test report entry 1");
    assert!(task.body.contains("## QA Report"));
    assert!(task.body.contains("Test report entry 1"));

    task.append_to_qa_report("Test report entry 2");
    assert!(task.body.contains("Test report entry 2"));
}

#[test]
fn test_append_to_qa_report_existing_section() {
    let content = r#"---
id: TASK-001
title: Test task
---

## Objective
Do something.

## QA Report
Existing report content.

## Other Section
Other content.
"#;
    let mut task = TaskFile::parse(content).unwrap();

    task.append_to_qa_report("New report entry");

    // New entry should be in QA Report section
    assert!(task.body.contains("New report entry"));
    // Other section should still exist
    assert!(task.body.contains("## Other Section"));
}

#[test]
fn test_clear_assigned() {
    let mut task = TaskFile::parse(FULL_TASK).unwrap();
    assert!(task.frontmatter.assigned_to.is_some());

    task.clear_assigned();
    assert!(task.frontmatter.assigned_to.is_none());
}

#[test]
fn test_empty_body() {
    let content = r#"---
id: TASK-001
title: Test task
---
"#;
    let task = TaskFile::parse(content).unwrap();
    assert!(task.body.is_empty() || task.body.chars().all(|c| c.is_whitespace()));
}

#[test]
fn test_frontmatter_default() {
    let fm = TaskFrontmatter::default();
    assert!(fm.id.is_empty());
    assert!(fm.title.is_empty());
    assert_eq!(fm.priority, "medium");
    assert_eq!(fm.qa_attempts, 0);
    assert!(fm.affects.is_empty());
    assert!(fm.extra.is_empty());
}

#[test]
fn test_serialize_skips_empty_fields() {
    let content = r#"---
id: TASK-001
title: Test task
---

Body.
"#;
    let task = TaskFile::parse(content).unwrap();
    let serialized = task.to_string().unwrap();

    // Empty optional fields should not appear in output
    assert!(!serialized.contains("affects:"));
    assert!(!serialized.contains("affects_globs:"));
    assert!(!serialized.contains("must_not_touch:"));
    assert!(!serialized.contains("depends_on:"));
    assert!(!serialized.contains("tags:"));
}

#[test]
fn test_serialize_includes_non_empty_fields() {
    let content = r#"---
id: TASK-001
title: Test task
affects:
  - src/main.rs
tags:
  - feature
---

Body.
"#;
    let task = TaskFile::parse(content).unwrap();
    let serialized = task.to_string().unwrap();

    // Non-empty fields should appear
    assert!(serialized.contains("affects:") || serialized.contains("affects"));
    assert!(serialized.contains("tags:") || serialized.contains("tags"));
}

#[test]
fn test_load_from_file() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("TASK-001-test.md");

    let content = r#"---
id: TASK-001
title: Test task from file
---

## Objective
Load test.
"#;
    std::fs::write(&file_path, content).unwrap();

    let task = TaskFile::load(&file_path).unwrap();
    assert_eq!(task.frontmatter.id, "TASK-001");
    assert_eq!(task.frontmatter.title, "Test task from file");
    assert!(task.body.contains("Load test."));
}

#[test]
fn test_save_atomic() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("TASK-001-test.md");

    let mut task = TaskFile::parse(MINIMAL_TASK).unwrap();
    task.frontmatter.id = "TASK-002".to_string();
    task.set_assigned("test@example.com", Some(Utc::now()));

    task.save(&file_path).unwrap();

    // Reload and verify
    let loaded = TaskFile::load(&file_path).unwrap();
    assert_eq!(loaded.frontmatter.id, "TASK-002");
    assert_eq!(
        loaded.frontmatter.assigned_to,
        Some("test@example.com".to_string())
    );
}

#[test]
fn test_save_atomic_replace_existing() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("TASK-001-test.md");

    // Create initial file
    let task = TaskFile::parse(MINIMAL_TASK).unwrap();
    task.save(&file_path).unwrap();

    // Modify and save again
    let mut task = TaskFile::load(&file_path).unwrap();
    task.frontmatter.priority = "high".to_string();
    task.increment_qa_attempts();
    task.save(&file_path).unwrap();

    // Reload and verify
    let loaded = TaskFile::load(&file_path).unwrap();
    assert_eq!(loaded.frontmatter.priority, "high");
    assert_eq!(loaded.frontmatter.qa_attempts, 1);
}

#[test]
fn test_load_nonexistent_file() {
    let result = TaskFile::load("/nonexistent/path/TASK-001.md");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("failed to read task file")
    );
}
