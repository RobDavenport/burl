//! Agent subprocess executor.
//!
//! Executes agent commands with timeout, output capture, and error handling.

use crate::agent::config::AgentProfile;
use crate::agent::prompt::{TemplateError, render_template};
use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Result of executing an agent command.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// Exit code of the process (None if killed or didn't exit normally).
    pub exit_code: Option<i32>,
    /// Path to the stdout log file.
    pub stdout_path: PathBuf,
    /// Path to the stderr log file.
    pub stderr_path: PathBuf,
    /// Duration of execution.
    pub duration: Duration,
    /// Whether the process was killed due to timeout.
    pub timed_out: bool,
    /// The command that was executed (for logging).
    pub command: String,
}

impl AgentResult {
    /// Check if the agent execution was successful.
    pub fn is_success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

/// Execute an agent command for a task.
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `agent` - The agent profile containing command template and settings
/// * `task_id` - The task identifier
/// * `variables` - Template variables for command substitution
/// * `worktree` - Path to the task worktree (working directory for the command)
/// * `timeout_seconds` - Maximum execution time before killing the process
///
/// # Returns
///
/// An `AgentResult` containing execution details and output paths.
pub fn execute_agent(
    ctx: &WorkflowContext,
    agent: &AgentProfile,
    task_id: &str,
    variables: &HashMap<String, String>,
    worktree: &str,
    timeout_seconds: u64,
) -> Result<AgentResult> {
    // Render the command template
    let command_str = render_template(&agent.command, variables).map_err(|e| match e {
        TemplateError::UndefinedVariable { name, .. } => BurlError::UserError(format!(
            "agent command template references undefined variable '{}'\n\
             Command: {}\n\
             Available variables: {}",
            name,
            agent.command,
            format_vars(variables)
        )),
        TemplateError::UnmatchedBrace { position } => BurlError::UserError(format!(
            "agent command template has unmatched '{{' at position {}",
            position
        )),
        TemplateError::EmptyVariableName { position } => BurlError::UserError(format!(
            "agent command template has empty variable name at position {}",
            position
        )),
    })?;

    // Parse the command using shell-words
    let args = shell_words::split(&command_str).map_err(|e| {
        BurlError::UserError(format!(
            "failed to parse agent command '{}': {}\n\
             Fix: check for unmatched quotes or invalid escape sequences.",
            command_str, e
        ))
    })?;

    if args.is_empty() {
        return Err(BurlError::UserError(format!(
            "agent command is empty after parsing: '{}'",
            command_str
        )));
    }

    // Create log directory
    let logs_dir = ctx.task_agent_logs_dir(task_id);
    std::fs::create_dir_all(&logs_dir).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create agent logs directory '{}': {}",
            logs_dir.display(),
            e
        ))
    })?;

    let stdout_path = logs_dir.join("stdout.log");
    let stderr_path = logs_dir.join("stderr.log");

    // Open log files
    let stdout_file = std::fs::File::create(&stdout_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create stdout log '{}': {}",
            stdout_path.display(),
            e
        ))
    })?;

    let stderr_file = std::fs::File::create(&stderr_path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create stderr log '{}': {}",
            stderr_path.display(),
            e
        ))
    })?;

    // Build the command
    let program = &args[0];
    let cmd_args = &args[1..];

    let mut command = Command::new(program);
    command
        .args(cmd_args)
        .current_dir(worktree)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    // Merge environment variables
    for (key, value) in &agent.environment {
        command.env(key, value);
    }

    // Spawn the process
    let start_time = Instant::now();
    let mut child = command.spawn().map_err(|e| {
        BurlError::UserError(format!(
            "failed to execute agent command '{}': {}\n\
             Fix: ensure the command is installed and in PATH.",
            program, e
        ))
    })?;

    // Wait with timeout
    let timeout = Duration::from_secs(timeout_seconds);
    let (exit_code, timed_out) = wait_with_timeout(&mut child, timeout)?;
    let duration = start_time.elapsed();

    Ok(AgentResult {
        exit_code,
        stdout_path,
        stderr_path,
        duration,
        timed_out,
        command: command_str,
    })
}

/// Wait for a child process with timeout.
///
/// Returns (exit_code, timed_out).
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<(Option<i32>, bool)> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(100);

    loop {
        // Check if process has exited
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok((status.code(), false));
            }
            Ok(None) => {
                // Still running
                if start.elapsed() >= timeout {
                    // Timeout - kill the process
                    kill_process(child)?;
                    return Ok((None, true));
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return Err(BurlError::UserError(format!(
                    "failed to check process status: {}",
                    e
                )));
            }
        }
    }
}

/// Kill a process and wait for it to terminate.
fn kill_process(child: &mut Child) -> Result<()> {
    // On Unix this is SIGKILL; on Windows it is TerminateProcess.
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

/// Format variables for error messages.
fn format_vars(vars: &HashMap<String, String>) -> String {
    let mut keys: Vec<_> = vars.keys().collect();
    keys.sort();
    keys.iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Append a message to a log file (for recording execution details).
#[allow(dead_code)]
pub fn append_to_log(path: &PathBuf, message: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| {
            BurlError::UserError(format!(
                "failed to open log file '{}': {}",
                path.display(),
                e
            ))
        })?;

    writeln!(file, "{}", message).map_err(|e| {
        BurlError::UserError(format!(
            "failed to write to log file '{}': {}",
            path.display(),
            e
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_context(temp_dir: &TempDir) -> WorkflowContext {
        let repo_root = temp_dir.path().to_path_buf();
        let workflow_worktree = repo_root.join(".burl");
        let workflow_state_dir = workflow_worktree.join(".workflow");
        let locks_dir = workflow_state_dir.join("locks");
        let worktrees_dir = repo_root.join(".worktrees");

        WorkflowContext {
            repo_root,
            workflow_worktree,
            workflow_state_dir,
            locks_dir,
            worktrees_dir,
        }
    }

    fn make_test_agent(command: &str) -> AgentProfile {
        AgentProfile {
            name: "Test Agent".to_string(),
            command: command.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_execute_agent_simple_command() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        // Create worktree directory
        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        // Use a simple echo command
        #[cfg(windows)]
        let agent = make_test_agent("cmd /c echo hello");
        #[cfg(not(windows))]
        let agent = make_test_agent("echo hello");

        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        )
        .unwrap();

        assert!(result.is_success());
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert!(result.stdout_path.exists());
        assert!(result.stderr_path.exists());
    }

    #[test]
    fn test_execute_agent_with_variables() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        #[cfg(windows)]
        let agent = make_test_agent("cmd /c echo {task_id}");
        #[cfg(not(windows))]
        let agent = make_test_agent("echo {task_id}");

        let mut vars = HashMap::new();
        vars.insert("task_id".to_string(), "TASK-001".to_string());

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        )
        .unwrap();

        assert!(result.is_success());

        // Check output contains the task ID
        let stdout = std::fs::read_to_string(&result.stdout_path).unwrap();
        assert!(stdout.contains("TASK-001"));
    }

    #[test]
    fn test_execute_agent_nonzero_exit() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        #[cfg(windows)]
        let agent = make_test_agent("cmd /c exit 1");
        #[cfg(not(windows))]
        let agent = make_test_agent("sh -c \"exit 1\"");

        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        )
        .unwrap();

        assert!(!result.is_success());
        assert_eq!(result.exit_code, Some(1));
        assert!(!result.timed_out);
    }

    #[test]
    fn test_execute_agent_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        // Command that sleeps longer than timeout
        #[cfg(windows)]
        let agent = make_test_agent("cmd /c ping -n 10 127.0.0.1");
        #[cfg(not(windows))]
        let agent = make_test_agent("sleep 10");

        let vars = HashMap::new();

        // Set very short timeout
        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            1,
        )
        .unwrap();

        assert!(!result.is_success());
        assert!(result.timed_out);
    }

    #[test]
    fn test_execute_agent_undefined_variable_error() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let agent = make_test_agent("echo {undefined}");
        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("undefined variable 'undefined'"));
    }

    #[test]
    fn test_execute_agent_invalid_command_error() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        // Command with unmatched quote
        let agent = make_test_agent("echo \"unmatched");
        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to parse"));
    }

    #[test]
    fn test_execute_agent_nonexistent_command_error() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let agent = make_test_agent("nonexistent_command_xyz_123");
        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to execute"));
    }

    #[test]
    fn test_execute_agent_creates_log_directory() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        // Log directory shouldn't exist yet
        let logs_dir = ctx.task_agent_logs_dir("TASK-001");
        assert!(!logs_dir.exists());

        #[cfg(windows)]
        let agent = make_test_agent("cmd /c echo test");
        #[cfg(not(windows))]
        let agent = make_test_agent("echo test");

        let vars = HashMap::new();

        execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        )
        .unwrap();

        // Log directory should now exist
        assert!(logs_dir.exists());
    }

    #[test]
    fn test_execute_agent_with_environment() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let mut agent = AgentProfile {
            name: "Test Agent".to_string(),
            #[cfg(windows)]
            command: "cmd /c echo %TEST_VAR%".to_string(),
            #[cfg(not(windows))]
            command: "sh -c \"echo $TEST_VAR\"".to_string(),
            ..Default::default()
        };
        agent
            .environment
            .insert("TEST_VAR".to_string(), "test_value".to_string());

        let vars = HashMap::new();

        let result = execute_agent(
            &ctx,
            &agent,
            "TASK-001",
            &vars,
            worktree.to_str().unwrap(),
            10,
        )
        .unwrap();

        assert!(result.is_success());

        let stdout = std::fs::read_to_string(&result.stdout_path).unwrap();
        assert!(stdout.contains("test_value"));
    }

    #[test]
    fn test_agent_result_is_success() {
        let result = AgentResult {
            exit_code: Some(0),
            stdout_path: PathBuf::from("/tmp/stdout.log"),
            stderr_path: PathBuf::from("/tmp/stderr.log"),
            duration: Duration::from_secs(1),
            timed_out: false,
            command: "echo test".to_string(),
        };
        assert!(result.is_success());

        let result = AgentResult {
            exit_code: Some(1),
            ..result.clone()
        };
        assert!(!result.is_success());

        let result = AgentResult {
            exit_code: Some(0),
            timed_out: true,
            stdout_path: PathBuf::from("/tmp/stdout.log"),
            stderr_path: PathBuf::from("/tmp/stderr.log"),
            duration: Duration::from_secs(1),
            command: "echo test".to_string(),
        };
        assert!(!result.is_success());
    }

    #[test]
    fn test_append_to_log() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        append_to_log(&log_path, "line 1").unwrap();
        append_to_log(&log_path, "line 2").unwrap();

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
    }
}
