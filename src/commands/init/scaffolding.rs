//! Directory and file scaffolding for the init command.
//!
//! Creates the workflow state directory structure, config files,
//! and git ignore rules.

use crate::config::Config;
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use crate::fs::atomic_write_file;
use std::fs;
use std::path::Path;

use super::BUCKETS;

const DEFAULT_AGENTS_YAML: &str = r#"agents: {}

defaults:
  timeout_seconds: 600
  prompt_template: default

prompt_templates:
  default: |
    # Task: {title}
    ## Objective
    {objective}
    ## Acceptance Criteria
    {acceptance_criteria}
"#;

/// Create the workflow state directory structure.
pub(super) fn create_workflow_structure(ctx: &WorkflowContext) -> Result<()> {
    // Create .workflow directory
    fs::create_dir_all(&ctx.workflow_state_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create workflow state directory '{}': {}",
            ctx.workflow_state_dir.display(),
            e
        ))
    })?;

    // Create bucket directories with .gitkeep files
    for bucket in BUCKETS {
        let bucket_path = ctx.bucket_path(bucket);
        create_dir_with_gitkeep(&bucket_path)?;
    }

    // Create events directory with .gitkeep
    let events_path = ctx.events_dir();
    create_dir_with_gitkeep(&events_path)?;

    // Create prompts directory with .gitkeep (tracked, durable)
    let prompts_path = ctx.prompts_dir();
    create_dir_with_gitkeep(&prompts_path)?;

    // Create locks directory (no .gitkeep - it's untracked)
    fs::create_dir_all(&ctx.locks_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create locks directory '{}': {}",
            ctx.locks_dir.display(),
            e
        ))
    })?;

    // Create agent-logs directory (untracked)
    let agent_logs_dir = ctx.agent_logs_dir();
    fs::create_dir_all(&agent_logs_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create agent logs directory '{}': {}",
            agent_logs_dir.display(),
            e
        ))
    })?;

    // Create config.yaml if it doesn't exist
    let config_path = ctx.config_path();
    if !config_path.exists() {
        let default_config = Config::default();
        let yaml = default_config.to_yaml()?;
        atomic_write_file(&config_path, &yaml)?;
    }

    // Create agents.yaml template if it doesn't exist
    let agents_path = ctx.agents_config_path();
    if !agents_path.exists() {
        atomic_write_file(&agents_path, DEFAULT_AGENTS_YAML)?;
    }

    // Create .gitignore in .workflow to ignore locks/
    let gitignore_path = ctx.workflow_state_dir.join(".gitignore");
    let required_entries = ["locks/", "agent-logs/"];
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    let mut missing_entries = Vec::new();
    for entry in required_entries {
        if !existing.lines().any(|line| line.trim() == entry) {
            missing_entries.push(entry);
        }
    }
    if gitignore_path.exists() && missing_entries.is_empty() {
        // Nothing to do.
    } else if gitignore_path.exists() {
        let mut new_content = existing;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        if !new_content.contains("# Machine-local files") {
            if !new_content.is_empty() {
                new_content.push('\n');
            }
            new_content.push_str("# Machine-local files (never commit)\n");
        }
        for entry in missing_entries {
            new_content.push_str(entry);
            new_content.push('\n');
        }
        atomic_write_file(&gitignore_path, &new_content)?;
    } else {
        atomic_write_file(
            &gitignore_path,
            "# Machine-local files (never commit)\nlocks/\nagent-logs/\n",
        )?;
    }

    Ok(())
}

/// Create a directory with a .gitkeep file to ensure it's tracked by git.
fn create_dir_with_gitkeep(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create directory '{}': {}",
            path.display(),
            e
        ))
    })?;

    let gitkeep = path.join(".gitkeep");
    if !gitkeep.exists() {
        atomic_write_file(&gitkeep, "")?;
    }

    Ok(())
}

/// Create the .worktrees directory at repo root (untracked).
pub(super) fn create_worktrees_dir(ctx: &WorkflowContext) -> Result<()> {
    if !ctx.worktrees_dir.exists() {
        fs::create_dir_all(&ctx.worktrees_dir).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create worktrees directory '{}': {}",
                ctx.worktrees_dir.display(),
                e
            ))
        })?;
    }

    Ok(())
}

/// Add .burl/ and .worktrees/ to .git/info/exclude.
pub(super) fn add_to_git_exclude(ctx: &WorkflowContext) -> Result<()> {
    let exclude_path = ctx.repo_root.join(".git").join("info").join("exclude");

    // Ensure the info directory exists
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            BurlError::UserError(format!("failed to create git info directory: {}", e))
        })?;
    }

    // Read existing content or start with empty
    let existing_content = fs::read_to_string(&exclude_path).unwrap_or_default();

    // Check what entries need to be added
    let mut entries_to_add = Vec::new();

    if !existing_content.lines().any(|line| line.trim() == ".burl/") {
        entries_to_add.push(".burl/");
    }

    if !existing_content
        .lines()
        .any(|line| line.trim() == ".worktrees/")
    {
        entries_to_add.push(".worktrees/");
    }

    // If entries need to be added, append them
    if !entries_to_add.is_empty() {
        let mut new_content = existing_content;

        // Ensure there's a newline before our additions
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }

        // Add a comment and our entries
        if !new_content.contains("# burl workflow directories") {
            new_content.push_str("\n# burl workflow directories\n");
        }

        for entry in entries_to_add {
            new_content.push_str(entry);
            new_content.push('\n');
        }

        atomic_write_file(&exclude_path, &new_content)?;
    }

    Ok(())
}
