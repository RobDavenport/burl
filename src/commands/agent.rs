//! Implementation of the `burl agent` commands.
//!
//! This module provides:
//! - `agent run` - Dispatch an agent to execute a task
//! - `agent list` - List configured agents

use crate::agent::prompt::{TaskContext, generate_and_write_prompt};
use crate::agent::{AgentsConfig, execute_agent, resolve_agent};
use crate::cli::AgentRunArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Execute the `burl agent run` command.
///
/// Dispatches an agent to work on a claimed task:
/// 1. Verifies task is in DOING bucket with worktree
/// 2. Loads agents.yaml and resolves agent profile
/// 3. Generates prompt file
/// 4. Executes agent command in worktree
/// 5. Returns agent's exit code
pub fn cmd_agent_run(args: AgentRunArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let _config = Config::load(ctx.config_path()).unwrap_or_default();

    // Load agents config
    let agents_config = AgentsConfig::load(ctx.agents_config_path())?.ok_or_else(|| {
        BurlError::UserError(format!(
            "agents.yaml not found at '{}'\n\n\
             Create an agents.yaml file to configure agent profiles.\n\n\
             Example agents.yaml:\n\
             agents:\n  \
               claude-code:\n    \
                 name: \"Claude Code CLI\"\n    \
                 command: \"claude -p {{prompt_file}}\"\n    \
                 default: true",
            ctx.agents_config_path().display()
        ))
    })?;

    // Find the task
    let index = TaskIndex::build(&ctx)?;
    let task_entry = index.find(&args.task_id).ok_or_else(|| {
        BurlError::UserError(format!("task '{}' not found in any bucket", args.task_id))
    })?;

    // Verify task is in DOING bucket
    if task_entry.bucket != "DOING" {
        return Err(BurlError::UserError(format!(
            "task '{}' is in {} bucket, but agent can only run on tasks in DOING bucket.\n\n\
             Use `burl claim {}` to claim the task first.",
            args.task_id, task_entry.bucket, args.task_id
        )));
    }

    // Load the task file
    let task_content = std::fs::read_to_string(&task_entry.path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to read task file '{}': {}",
            task_entry.path.display(),
            e
        ))
    })?;
    let task = TaskFile::parse(&task_content)?;

    // Verify task has worktree
    let worktree = task.frontmatter.worktree.as_ref().ok_or_else(|| {
        BurlError::UserError(format!(
            "task '{}' has no worktree recorded.\n\n\
             The task must be properly claimed with `burl claim` before running an agent.",
            args.task_id
        ))
    })?;
    let worktree_path = resolve_worktree_path(&ctx, worktree);
    if !worktree_path.exists() {
        return Err(BurlError::UserError(format!(
            "task '{}' worktree missing at '{}'.\n\n\
             Fix: run `burl doctor` to diagnose/repair worktree metadata.",
            args.task_id,
            worktree_path.display()
        )));
    }
    let worktree_str = worktree_path.to_string_lossy().to_string();

    // Resolve which agent to use
    let binding = if let Some(ref agent_name) = args.agent {
        // Override with explicit agent
        let profile = agents_config.get(agent_name).ok_or_else(|| {
            BurlError::UserError(format!(
                "agent '{}' not found in agents.yaml.\n\
                 Available agents: {}",
                agent_name,
                agents_config
                    .agents
                    .keys()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        crate::agent::AgentBinding {
            profile,
            agent_id: agent_name,
            binding_source: crate::agent::BindingSource::Explicit,
        }
    } else {
        resolve_agent(&task, &agents_config)?
    };

    // Generate prompt file
    let prompt = generate_and_write_prompt(&ctx, &task, binding.profile, &agents_config)?;

    // Build template variables for command
    let task_ctx = TaskContext::from_task(&task);
    let mut vars = task_ctx.to_template_vars();
    // Ensure `{worktree}` is absolute (task frontmatter may contain a relative path).
    vars.insert("worktree".to_string(), worktree_str.clone());
    vars.insert(
        "prompt_file".to_string(),
        prompt.path.to_string_lossy().to_string(),
    );
    vars.insert(
        "task_file".to_string(),
        task_entry.path.to_string_lossy().to_string(),
    );

    // Get timeout
    let timeout = binding.profile.effective_timeout(&agents_config.defaults);

    // Dry run mode - just print what would happen
    if args.dry_run {
        print_dry_run(
            &args.task_id,
            binding.agent_id,
            binding.profile,
            &vars,
            &prompt.path,
            &worktree_str,
            timeout,
        );
        return Ok(());
    }

    // Print execution info
    println!("Dispatching agent for task {}...", args.task_id);
    println!();
    println!(
        "  Agent:     {} ({})",
        binding.agent_id, binding.profile.name
    );
    println!("  Worktree:  {}", worktree_path.display());
    println!("  Prompt:    {}", prompt.path.display());
    println!("  Timeout:   {}s", timeout);
    println!();

    // Log agent dispatch event
    let dispatch_event = Event::new(EventAction::AgentDispatch)
        .with_task(&args.task_id)
        .with_details(json!({
            "agent_id": binding.agent_id,
            "agent_name": binding.profile.name,
            "timeout_seconds": timeout,
            "prompt_file": prompt.path.to_string_lossy(),
        }));
    if let Err(e) = append_event(&ctx, &dispatch_event) {
        eprintln!("Warning: failed to log agent_dispatch event: {}", e);
    }

    // Execute the agent
    let result = execute_agent(
        &ctx,
        binding.profile,
        &args.task_id,
        &vars,
        &worktree_str,
        timeout,
    )?;

    // Log agent complete event
    let complete_event = Event::new(EventAction::AgentComplete)
        .with_task(&args.task_id)
        .with_details(json!({
            "agent_id": binding.agent_id,
            "agent_name": binding.profile.name,
            "exit_code": result.exit_code,
            "duration_ms": result.duration.as_millis() as u64,
            "timed_out": result.timed_out,
            "success": result.is_success(),
        }));
    if let Err(e) = append_event(&ctx, &complete_event) {
        eprintln!("Warning: failed to log agent_complete event: {}", e);
    }

    // Print result
    println!("Agent execution complete.");
    println!();
    println!("  Duration:  {:.2}s", result.duration.as_secs_f64());
    println!("  Exit code: {:?}", result.exit_code);
    println!("  Stdout:    {}", result.stdout_path.display());
    println!("  Stderr:    {}", result.stderr_path.display());

    if result.timed_out {
        println!();
        println!(
            "WARNING: Agent was terminated due to timeout ({}s)",
            timeout
        );
        return Err(BurlError::UserError(format!(
            "agent timed out after {}s. Check {} for output.",
            timeout,
            result.stderr_path.display()
        )));
    }

    if !result.is_success() {
        println!();
        println!("WARNING: Agent exited with non-zero code");
        return Err(BurlError::UserError(format!(
            "agent exited with code {:?}. Check {} for output.",
            result.exit_code,
            result.stderr_path.display()
        )));
    }

    println!();
    println!("Agent completed successfully.");
    println!(
        "Run `burl submit {}` to submit the task for review.",
        args.task_id
    );

    Ok(())
}

/// Print dry run information.
fn print_dry_run(
    task_id: &str,
    agent_id: &str,
    profile: &crate::agent::AgentProfile,
    vars: &HashMap<String, String>,
    prompt_path: &std::path::Path,
    worktree: &str,
    timeout: u64,
) {
    // Render command for display
    let command_display = match crate::agent::prompt::render_template(&profile.command, vars) {
        Ok(cmd) => cmd,
        Err(e) => format!("<error rendering command: {}>", e),
    };

    println!("Dry run - would execute:");
    println!();
    println!("  Task:      {}", task_id);
    println!("  Agent:     {} ({})", agent_id, profile.name);
    println!("  Command:   {}", command_display);
    println!("  Worktree:  {}", worktree);
    println!("  Prompt:    {}", prompt_path.display());
    println!("  Timeout:   {}s", timeout);

    if !profile.environment.is_empty() {
        println!("  Environment:");
        for (key, value) in &profile.environment {
            println!("    {}={}", key, value);
        }
    }
}

fn resolve_worktree_path(ctx: &crate::context::WorkflowContext, worktree: &str) -> PathBuf {
    let path = Path::new(worktree);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.repo_root.join(path)
    }
}

/// Execute the `burl agent list` command.
///
/// Lists all configured agents from agents.yaml.
pub fn cmd_agent_list() -> Result<()> {
    let ctx = require_initialized_workflow()?;

    // Load agents config
    let agents_config = match AgentsConfig::load(ctx.agents_config_path())? {
        Some(config) => config,
        None => {
            println!("No agents configured.");
            println!();
            println!(
                "Create {} to configure agent profiles.",
                ctx.agents_config_path().display()
            );
            println!();
            println!("Example agents.yaml:");
            println!("agents:");
            println!("  claude-code:");
            println!("    name: \"Claude Code CLI\"");
            println!("    command: \"claude -p {{prompt_file}}\"");
            println!("    default: true");
            return Ok(());
        }
    };

    if agents_config.agents.is_empty() {
        println!("No agents configured in agents.yaml.");
        return Ok(());
    }

    println!("Configured agents ({}):", agents_config.agents.len());
    println!();

    for (id, profile) in &agents_config.agents {
        let default_marker = if profile.default { " (default)" } else { "" };
        let name = if profile.name.is_empty() {
            id.as_str()
        } else {
            &profile.name
        };

        println!("  {}{}", id, default_marker);
        println!("    Name:        {}", name);
        println!(
            "    Command:     {}",
            truncate_command(&profile.command, 50)
        );
        println!(
            "    Timeout:     {}s",
            profile.effective_timeout(&agents_config.defaults)
        );

        if !profile.capabilities.is_empty() {
            println!("    Capabilities: {}", profile.capabilities.join(", "));
        }

        if !profile.environment.is_empty() {
            println!("    Environment: {} vars", profile.environment.len());
        }

        println!();
    }

    // Print defaults
    println!("Defaults:");
    println!(
        "  timeout_seconds: {}",
        agents_config.defaults.timeout_seconds
    );
    println!(
        "  prompt_template: {}",
        agents_config.defaults.prompt_template
    );

    if !agents_config.prompt_templates.is_empty() {
        println!();
        println!(
            "Prompt templates: {}",
            agents_config
                .prompt_templates
                .keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}

/// Truncate a command string for display.
fn truncate_command(command: &str, max_len: usize) -> String {
    if command.len() <= max_len {
        command.to_string()
    } else {
        format!("{}...", &command[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn test_truncate_command() {
        assert_eq!(truncate_command("short", 50), "short");
        assert_eq!(
            truncate_command("this is a very long command that exceeds the limit", 30),
            "this is a very long command..."
        );
    }

    /// Create a test git repo with workflow structure.
    fn create_test_workflow() -> (TempDir, crate::context::WorkflowContext) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        let ctx = crate::context::WorkflowContext::resolve_from(path).unwrap();
        std::fs::create_dir_all(ctx.workflow_state_dir.join("DOING")).unwrap();

        (temp_dir, ctx)
    }

    #[test]
    fn test_agents_config_load_missing() {
        let (_temp_dir, ctx) = create_test_workflow();
        let result = AgentsConfig::load(ctx.agents_config_path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_agents_config_load_valid() {
        let (_temp_dir, ctx) = create_test_workflow();

        // Create agents.yaml
        let agents_yaml = r#"
agents:
  test-agent:
    name: "Test Agent"
    command: "echo {task_id}"
    default: true
    timeout_seconds: 60
"#;
        std::fs::write(ctx.agents_config_path(), agents_yaml).unwrap();

        let result = AgentsConfig::load(ctx.agents_config_path()).unwrap();
        assert!(result.is_some());

        let config = result.unwrap();
        assert!(config.agents.contains_key("test-agent"));
        assert_eq!(config.agents["test-agent"].name, "Test Agent");
        assert!(config.agents["test-agent"].default);
    }

    #[test]
    fn test_agent_run_missing_task() {
        let (_temp_dir, _ctx) = create_test_workflow();

        // Set working directory to temp dir
        let result = cmd_agent_run(AgentRunArgs {
            task_id: "TASK-999".to_string(),
            agent: None,
            dry_run: false,
        });

        // Should fail because we're not in the right context
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_profile_effective_timeout() {
        let profile = crate::agent::AgentProfile {
            name: "test".to_string(),
            command: "echo".to_string(),
            timeout_seconds: Some(120),
            ..Default::default()
        };

        let config = AgentsConfig::default();
        assert_eq!(profile.effective_timeout(&config.defaults), 120);

        let profile_no_timeout = crate::agent::AgentProfile {
            name: "test".to_string(),
            command: "echo".to_string(),
            timeout_seconds: None,
            ..Default::default()
        };
        assert_eq!(profile_no_timeout.effective_timeout(&config.defaults), 600); // default
    }
}
