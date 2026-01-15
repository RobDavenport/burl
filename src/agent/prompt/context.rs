//! Task context extraction for prompt generation.
//!
//! This module extracts task metadata and markdown body sections into
//! a structured `TaskContext` that can be converted to template variables.
//!
//! # Sections
//!
//! The following markdown sections are recognized:
//!
//! - `## Objective` - The main goal of the task
//! - `## Acceptance Criteria` - Criteria for completion
//! - `## Context` - Additional background information
//! - `## Implementation Notes` - Technical notes for implementation
//! - `## Test Plan` - Testing instructions
//!
//! Sections can use `##` or `###` headings and are case-insensitive.

use crate::task::TaskFile;
use std::collections::HashMap;

/// Extracted context from a task file for prompt generation.
#[derive(Debug, Clone, Default)]
pub struct TaskContext {
    // Frontmatter fields
    /// Task identifier (e.g., "TASK-001").
    pub task_id: String,
    /// Task title.
    pub title: String,
    /// Priority level.
    pub priority: String,
    /// Explicit file paths this task affects.
    pub affects: Vec<String>,
    /// Glob patterns for affected paths.
    pub affects_globs: Vec<String>,
    /// Paths this task must not touch.
    pub must_not_touch: Vec<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Task dependencies.
    pub depends_on: Vec<String>,
    /// Path to the task worktree.
    pub worktree: Option<String>,
    /// Name of the task branch.
    pub branch: Option<String>,
    /// Base SHA for diff validation.
    pub base_sha: Option<String>,

    // Extracted body sections
    /// The Objective section content.
    pub objective: String,
    /// The Acceptance Criteria section content.
    pub acceptance_criteria: String,
    /// The Context section content.
    pub context: String,
    /// The Implementation Notes section content.
    pub implementation_notes: String,
    /// The Test Plan section content.
    pub test_plan: String,

    /// The full body content (for fallback).
    pub full_body: String,
}

impl TaskContext {
    /// Extract context from a task file.
    pub fn from_task(task: &TaskFile) -> Self {
        let fm = &task.frontmatter;

        // Extract sections from the markdown body
        let sections = extract_sections(&task.body);

        Self {
            task_id: fm.id.clone(),
            title: fm.title.clone(),
            priority: fm.priority.clone(),
            affects: fm.affects.clone(),
            affects_globs: fm.affects_globs.clone(),
            must_not_touch: fm.must_not_touch.clone(),
            tags: fm.tags.clone(),
            depends_on: fm.depends_on.clone(),
            worktree: fm.worktree.clone(),
            branch: fm.branch.clone(),
            base_sha: fm.base_sha.clone(),

            objective: sections.get("objective").cloned().unwrap_or_default(),
            acceptance_criteria: sections
                .get("acceptance_criteria")
                .or_else(|| sections.get("acceptance criteria"))
                .cloned()
                .unwrap_or_default(),
            context: sections.get("context").cloned().unwrap_or_default(),
            implementation_notes: sections
                .get("implementation_notes")
                .or_else(|| sections.get("implementation notes"))
                .cloned()
                .unwrap_or_default(),
            test_plan: sections
                .get("test_plan")
                .or_else(|| sections.get("test plan"))
                .cloned()
                .unwrap_or_default(),

            full_body: task.body.clone(),
        }
    }

    /// Convert the context to template variables.
    ///
    /// Returns a HashMap suitable for use with `render_template`.
    pub fn to_template_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Frontmatter fields
        vars.insert("task_id".to_string(), self.task_id.clone());
        vars.insert("title".to_string(), self.title.clone());
        vars.insert("priority".to_string(), self.priority.clone());

        // Lists as comma-separated strings
        vars.insert("affects".to_string(), self.affects.join(", "));
        vars.insert("affects_globs".to_string(), self.affects_globs.join(", "));
        vars.insert("must_not_touch".to_string(), self.must_not_touch.join(", "));
        vars.insert("tags".to_string(), self.tags.join(", "));
        vars.insert("depends_on".to_string(), self.depends_on.join(", "));

        // Git/worktree fields
        vars.insert(
            "worktree".to_string(),
            self.worktree.clone().unwrap_or_default(),
        );
        vars.insert(
            "branch".to_string(),
            self.branch.clone().unwrap_or_default(),
        );
        vars.insert(
            "base_sha".to_string(),
            self.base_sha.clone().unwrap_or_default(),
        );

        // Body sections
        vars.insert("objective".to_string(), self.objective.clone());
        vars.insert(
            "acceptance_criteria".to_string(),
            self.acceptance_criteria.clone(),
        );
        vars.insert("context".to_string(), self.context.clone());
        vars.insert(
            "implementation_notes".to_string(),
            self.implementation_notes.clone(),
        );
        vars.insert("test_plan".to_string(), self.test_plan.clone());
        vars.insert("body".to_string(), self.full_body.clone());

        vars
    }

    /// Add runtime variables (paths resolved at execution time).
    pub fn add_runtime_vars(
        &self,
        vars: &mut HashMap<String, String>,
        task_file_path: Option<&str>,
        prompt_file_path: Option<&str>,
    ) {
        if let Some(path) = task_file_path {
            vars.insert("task_file".to_string(), path.to_string());
        }
        if let Some(path) = prompt_file_path {
            vars.insert("prompt_file".to_string(), path.to_string());
        }
    }
}

/// Extract sections from markdown body.
///
/// Recognizes headings like `## Section Name` or `### Section Name`.
/// Returns a map of lowercase section names to their content.
fn extract_sections(body: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current_section: Option<String> = None;
    let mut current_content = String::new();

    for line in body.lines() {
        if let Some(heading) = parse_heading(line) {
            // Save previous section if any
            if let Some(section_name) = current_section.take() {
                let content = current_content.trim().to_string();
                if !content.is_empty() {
                    sections.insert(section_name, content);
                }
            }

            // Start new section
            current_section = Some(heading);
            current_content = String::new();
        } else if current_section.is_some() {
            // Append to current section
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Save last section
    if let Some(section_name) = current_section {
        let content = current_content.trim().to_string();
        if !content.is_empty() {
            sections.insert(section_name, content);
        }
    }

    sections
}

/// Parse a markdown heading line and return the normalized section name.
///
/// Accepts `## Section Name` or `### Section Name`.
fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // Check for ## or ### prefix
    let content = if let Some(rest) = trimmed.strip_prefix("###") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("##") {
        rest.trim()
    } else {
        return None;
    };

    if content.is_empty() {
        return None;
    }

    // Normalize: lowercase, replace spaces with underscores
    let normalized = content.to_lowercase().replace(' ', "_");
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{TaskFile, TaskFrontmatter};

    fn make_task(body: &str) -> TaskFile {
        TaskFile {
            frontmatter: TaskFrontmatter {
                id: "TASK-001".to_string(),
                title: "Test Task".to_string(),
                priority: "high".to_string(),
                affects: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
                affects_globs: vec!["src/**/*.rs".to_string()],
                must_not_touch: vec!["config.yaml".to_string()],
                tags: vec!["feature".to_string(), "agent".to_string()],
                depends_on: vec!["TASK-000".to_string()],
                worktree: Some("/path/to/worktree".to_string()),
                branch: Some("task-001".to_string()),
                base_sha: Some("abc123".to_string()),
                ..Default::default()
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn test_extract_sections_basic() {
        let body = r#"
## Objective
Do something useful.

## Acceptance Criteria
- Item 1
- Item 2

## Context
Some background info.
"#;
        let sections = extract_sections(body);

        assert_eq!(
            sections.get("objective"),
            Some(&"Do something useful.".to_string())
        );
        assert_eq!(
            sections.get("acceptance_criteria"),
            Some(&"- Item 1\n- Item 2".to_string())
        );
        assert_eq!(
            sections.get("context"),
            Some(&"Some background info.".to_string())
        );
    }

    #[test]
    fn test_extract_sections_h3() {
        let body = r#"
### Objective
Using h3 heading.

### Test Plan
1. Run tests
2. Check output
"#;
        let sections = extract_sections(body);

        assert_eq!(
            sections.get("objective"),
            Some(&"Using h3 heading.".to_string())
        );
        assert_eq!(
            sections.get("test_plan"),
            Some(&"1. Run tests\n2. Check output".to_string())
        );
    }

    #[test]
    fn test_extract_sections_case_insensitive() {
        let body = r#"
## OBJECTIVE
Uppercase heading.

## acceptance criteria
Lowercase heading.
"#;
        let sections = extract_sections(body);

        assert!(sections.contains_key("objective"));
        assert!(sections.contains_key("acceptance_criteria"));
    }

    #[test]
    fn test_extract_sections_empty_body() {
        let sections = extract_sections("");
        assert!(sections.is_empty());
    }

    #[test]
    fn test_extract_sections_no_sections() {
        let body = "Just some text without headings.";
        let sections = extract_sections(body);
        assert!(sections.is_empty());
    }

    #[test]
    fn test_task_context_from_task() {
        let body = r#"
## Objective
Implement the feature.

## Acceptance Criteria
- Tests pass
- No regressions
"#;
        let task = make_task(body);
        let ctx = TaskContext::from_task(&task);

        assert_eq!(ctx.task_id, "TASK-001");
        assert_eq!(ctx.title, "Test Task");
        assert_eq!(ctx.priority, "high");
        assert_eq!(ctx.affects, vec!["src/main.rs", "src/lib.rs"]);
        assert_eq!(ctx.affects_globs, vec!["src/**/*.rs"]);
        assert_eq!(ctx.must_not_touch, vec!["config.yaml"]);
        assert_eq!(ctx.tags, vec!["feature", "agent"]);
        assert_eq!(ctx.depends_on, vec!["TASK-000"]);
        assert_eq!(ctx.worktree, Some("/path/to/worktree".to_string()));
        assert_eq!(ctx.branch, Some("task-001".to_string()));
        assert_eq!(ctx.base_sha, Some("abc123".to_string()));
        assert_eq!(ctx.objective, "Implement the feature.");
        assert_eq!(ctx.acceptance_criteria, "- Tests pass\n- No regressions");
    }

    #[test]
    fn test_to_template_vars() {
        let body = r#"
## Objective
Test objective.
"#;
        let task = make_task(body);
        let ctx = TaskContext::from_task(&task);
        let vars = ctx.to_template_vars();

        assert_eq!(vars.get("task_id"), Some(&"TASK-001".to_string()));
        assert_eq!(vars.get("title"), Some(&"Test Task".to_string()));
        assert_eq!(vars.get("priority"), Some(&"high".to_string()));
        assert_eq!(
            vars.get("affects"),
            Some(&"src/main.rs, src/lib.rs".to_string())
        );
        assert_eq!(vars.get("affects_globs"), Some(&"src/**/*.rs".to_string()));
        assert_eq!(vars.get("must_not_touch"), Some(&"config.yaml".to_string()));
        assert_eq!(vars.get("tags"), Some(&"feature, agent".to_string()));
        assert_eq!(vars.get("depends_on"), Some(&"TASK-000".to_string()));
        assert_eq!(vars.get("worktree"), Some(&"/path/to/worktree".to_string()));
        assert_eq!(vars.get("branch"), Some(&"task-001".to_string()));
        assert_eq!(vars.get("base_sha"), Some(&"abc123".to_string()));
        assert_eq!(vars.get("objective"), Some(&"Test objective.".to_string()));
    }

    #[test]
    fn test_to_template_vars_empty_lists() {
        let task = TaskFile {
            frontmatter: TaskFrontmatter {
                id: "TASK-002".to_string(),
                title: "Empty Task".to_string(),
                ..Default::default()
            },
            body: String::new(),
        };
        let ctx = TaskContext::from_task(&task);
        let vars = ctx.to_template_vars();

        assert_eq!(vars.get("affects"), Some(&"".to_string()));
        assert_eq!(vars.get("tags"), Some(&"".to_string()));
        assert_eq!(vars.get("worktree"), Some(&"".to_string()));
    }

    #[test]
    fn test_add_runtime_vars() {
        let task = make_task("");
        let ctx = TaskContext::from_task(&task);
        let mut vars = ctx.to_template_vars();

        ctx.add_runtime_vars(
            &mut vars,
            Some("/path/to/task.md"),
            Some("/path/to/prompt.md"),
        );

        assert_eq!(vars.get("task_file"), Some(&"/path/to/task.md".to_string()));
        assert_eq!(
            vars.get("prompt_file"),
            Some(&"/path/to/prompt.md".to_string())
        );
    }

    #[test]
    fn test_parse_heading() {
        assert_eq!(parse_heading("## Objective"), Some("objective".to_string()));
        assert_eq!(
            parse_heading("### Test Plan"),
            Some("test_plan".to_string())
        );
        assert_eq!(
            parse_heading("## Acceptance Criteria"),
            Some("acceptance_criteria".to_string())
        );
        assert_eq!(
            parse_heading("##   Spaced  Heading  "),
            Some("spaced__heading".to_string())
        );
        assert_eq!(parse_heading("# Not h2"), None);
        assert_eq!(parse_heading("Just text"), None);
        assert_eq!(parse_heading("##"), None);
        assert_eq!(parse_heading("###"), None);
    }

    #[test]
    fn test_multiline_section_content() {
        let body = r#"
## Objective
Line 1.
Line 2.
Line 3.

## Context
Single line.
"#;
        let sections = extract_sections(body);

        assert_eq!(
            sections.get("objective"),
            Some(&"Line 1.\nLine 2.\nLine 3.".to_string())
        );
        assert_eq!(sections.get("context"), Some(&"Single line.".to_string()));
    }

    #[test]
    fn test_section_with_code_block() {
        let body = r#"
## Implementation Notes
Here's some code:

```rust
fn main() {
    println!("hello");
}
```

End of notes.
"#;
        let sections = extract_sections(body);
        let notes = sections.get("implementation_notes").unwrap();

        assert!(notes.contains("```rust"));
        assert!(notes.contains("fn main()"));
        assert!(notes.contains("End of notes."));
    }

    #[test]
    fn test_full_body_preserved() {
        let body = "This is the full body content.\n\n## Section\nContent here.";
        let task = make_task(body);
        let ctx = TaskContext::from_task(&task);

        assert_eq!(ctx.full_body, body);

        let vars = ctx.to_template_vars();
        assert_eq!(vars.get("body"), Some(&body.to_string()));
    }
}
