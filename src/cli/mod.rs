//! CLI argument parsing for burl.
//!
//! Uses clap derive macros for declarative argument definitions.
//! This module defines the command structure; actual implementations
//! are in the `commands` module.

use clap::{ArgAction, Parser, Subcommand};

/// Burl: Minimal file-based workflow orchestrator for agentic coding pipelines.
///
/// Workflows are expressed as files and folders inside a Git repository:
/// - Folders are status buckets (READY/DOING/QA/DONE/BLOCKED)
/// - Task files are durable state committed to Git
/// - Git worktrees isolate work per task
#[derive(Parser, Debug)]
#[command(name = "burl")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Available commands for burl.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize the burl workflow in the current repository.
    ///
    /// Creates the workflow branch, canonical workflow worktree (.burl/),
    /// bucket directories, and initial configuration.
    Init,

    /// Add a new task to the workflow.
    ///
    /// Creates a task file in the READY bucket with the specified metadata.
    Add(AddArgs),

    /// Show workflow status summary.
    ///
    /// Displays counts per bucket and highlights locked or stalled tasks.
    Status,

    /// Show details of a specific task.
    ///
    /// Renders the task markdown and key metadata.
    Show(ShowArgs),

    /// Claim a task for work.
    ///
    /// Creates a branch and worktree for the task, sets base_sha,
    /// and moves the task from READY to DOING.
    Claim(ClaimArgs),

    /// Submit a task for QA review.
    ///
    /// Runs scope and stub validation checks, then moves the task
    /// from DOING to QA.
    Submit(SubmitArgs),

    /// Run validation checks on a task.
    ///
    /// Validates scope, stubs, and optionally runs build/test commands.
    /// Does not change task status.
    Validate(ValidateArgs),

    /// Approve a task and merge to main.
    ///
    /// Rebases the task branch, runs final validation, performs
    /// fast-forward merge, and moves task to DONE.
    Approve(ApproveArgs),

    /// Reject a task and return to READY.
    ///
    /// Increments QA attempts, appends rejection reason,
    /// and preserves the branch/worktree for rework.
    Reject(RejectArgs),

    /// Show the recorded worktree path for a task.
    ///
    /// Prints the recorded worktree path for a task.
    Worktree(WorktreeArgs),

    /// Lock management commands.
    ///
    /// List or clear workflow and task locks.
    Lock(LockCommand),

    /// Diagnose workflow health.
    ///
    /// Reports stale locks, orphan worktrees, and metadata inconsistencies.
    Doctor(DoctorArgs),

    /// Clean up completed or orphan worktrees.
    ///
    /// Removes worktrees for completed tasks and cleans orphan artifacts.
    Clean(CleanArgs),

    /// Automation loop for claiming and QA processing.
    ///
    /// By default, `watch` will:
    /// - keep claiming tasks until `max_parallel` is reached
    /// - validate tasks currently in QA (once per new commit SHA)
    ///
    /// Use `--approve` to also auto-approve passing QA tasks.
    Watch(WatchArgs),

    /// Live dashboard / visualizer for workflow status.
    ///
    /// This is a lightweight TUI-style view that refreshes periodically.
    /// (Alias: `visualizer`)
    #[command(alias = "visualizer", alias = "viz", alias = "dashboard")]
    Monitor(MonitorArgs),

    /// Agent execution commands (V2).
    ///
    /// Dispatch agents to work on tasks or list configured agents.
    Agent(AgentCommand),
}

/// Arguments for the `add` command.
#[derive(Parser, Debug)]
pub struct AddArgs {
    /// Title for the new task.
    pub title: String,

    /// Priority level (high, medium, low).
    #[arg(short, long, default_value = "medium")]
    pub priority: String,

    /// Files or paths this task affects.
    #[arg(long, value_delimiter = ',')]
    pub affects: Vec<String>,

    /// Glob patterns for affected paths (supports new files).
    #[arg(long, value_delimiter = ',')]
    pub affects_globs: Vec<String>,

    /// Paths this task must not touch.
    #[arg(long, value_delimiter = ',')]
    pub must_not_touch: Vec<String>,

    /// Task IDs this task depends on.
    #[arg(long, value_delimiter = ',')]
    pub depends_on: Vec<String>,

    /// Tags for categorization.
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
}

/// Arguments for the `show` command.
#[derive(Parser, Debug)]
pub struct ShowArgs {
    /// Task ID to show (e.g., TASK-001).
    pub task_id: String,
}

/// Arguments for the `claim` command.
#[derive(Parser, Debug)]
pub struct ClaimArgs {
    /// Task ID to claim. If omitted, claims the next available task.
    pub task_id: Option<String>,
}

/// Arguments for the `submit` command.
#[derive(Parser, Debug)]
pub struct SubmitArgs {
    /// Task ID to submit. If omitted, uses the current worktree's task.
    pub task_id: Option<String>,
}

/// Arguments for the `validate` command.
#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// Task ID to validate.
    pub task_id: String,
}

/// Arguments for the `approve` command.
#[derive(Parser, Debug)]
pub struct ApproveArgs {
    /// Task ID to approve.
    pub task_id: String,
}

/// Arguments for the `reject` command.
#[derive(Parser, Debug)]
pub struct RejectArgs {
    /// Task ID to reject.
    pub task_id: String,

    /// Reason for rejection (required).
    #[arg(short, long)]
    pub reason: String,
}

/// Arguments for the `worktree` command.
#[derive(Parser, Debug)]
pub struct WorktreeArgs {
    /// Task ID to get worktree path for (e.g., TASK-001).
    pub task_id: String,
}

/// Lock subcommands.
#[derive(Parser, Debug)]
pub struct LockCommand {
    #[command(subcommand)]
    pub action: LockAction,
}

/// Available lock actions.
#[derive(Subcommand, Debug)]
pub enum LockAction {
    /// List all active locks.
    ///
    /// Shows task locks and workflow locks with their age and owner.
    List,

    /// Clear a specific lock.
    ///
    /// Requires --force flag to prevent accidental clearing.
    Clear(LockClearArgs),
}

/// Arguments for the `lock clear` command.
#[derive(Parser, Debug)]
pub struct LockClearArgs {
    /// Task ID whose lock should be cleared, or "workflow" for workflow lock.
    pub lock_id: String,

    /// Force clearing the lock (required for safety).
    #[arg(long)]
    pub force: bool,
}

/// Arguments for the `doctor` command.
#[derive(Parser, Debug)]
pub struct DoctorArgs {
    /// Attempt to repair detected issues.
    #[arg(long)]
    pub repair: bool,

    /// Force repairs without confirmation (use with --repair).
    #[arg(long)]
    pub force: bool,
}

/// Arguments for the `clean` command.
#[derive(Parser, Debug)]
pub struct CleanArgs {
    /// Remove worktrees for completed tasks.
    #[arg(long)]
    pub completed: bool,

    /// Remove orphan worktrees (no matching task).
    #[arg(long)]
    pub orphans: bool,

    /// Skip confirmation prompts.
    #[arg(long)]
    pub yes: bool,
}

/// Arguments for the `watch` command.
#[derive(Parser, Debug)]
pub struct WatchArgs {
    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 2000)]
    pub interval_ms: u64,

    /// Whether to auto-claim READY tasks up to config `max_parallel`.
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub claim: bool,

    /// Whether to process QA tasks (validate/approve).
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub qa: bool,

    /// When set, attempt to approve QA tasks (runs validations via `approve`).
    #[arg(long)]
    pub approve: bool,

    /// When set, auto-dispatch agents for newly claimed tasks (V2).
    #[arg(long)]
    pub dispatch: bool,

    /// Run a single iteration and exit.
    #[arg(long)]
    pub once: bool,
}

/// Arguments for the `monitor` (visualizer) command.
#[derive(Parser, Debug)]
pub struct MonitorArgs {
    /// Refresh interval in milliseconds.
    #[arg(long, default_value_t = 1000)]
    pub interval_ms: u64,

    /// Run once and exit (no refresh loop).
    #[arg(long)]
    pub once: bool,

    /// Clear the screen between refreshes.
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub clear: bool,

    /// Limit number of tasks shown per bucket section.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Show the last N events from the audit log (0 disables).
    #[arg(long, default_value_t = 10)]
    pub tail: usize,
}

/// Agent subcommands (V2).
#[derive(Parser, Debug)]
pub struct AgentCommand {
    #[command(subcommand)]
    pub action: AgentAction,
}

/// Available agent actions.
#[derive(Subcommand, Debug)]
pub enum AgentAction {
    /// Run an agent on a task.
    ///
    /// Dispatches the configured agent to execute the task.
    /// The task must be in the DOING bucket with a worktree.
    Run(AgentRunArgs),

    /// List configured agents.
    ///
    /// Shows all agent profiles from agents.yaml.
    List,
}

/// Arguments for the `agent run` command.
#[derive(Parser, Debug)]
pub struct AgentRunArgs {
    /// Task ID to run the agent on.
    pub task_id: String,

    /// Override the task's assigned agent with a specific agent.
    #[arg(long)]
    pub agent: Option<String>,

    /// Show the command that would be executed without running it.
    #[arg(long)]
    pub dry_run: bool,
}

impl Cli {
    /// Parse command line arguments.
    pub fn parse_args() -> Self {
        Cli::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_debug_assert() {
        // Verifies the CLI arguments configuration is valid
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_init() {
        let cli = Cli::try_parse_from(["burl", "init"]).unwrap();
        assert!(matches!(cli.command, Command::Init));
    }

    #[test]
    fn parse_add_minimal() {
        let cli = Cli::try_parse_from(["burl", "add", "My Task Title"]).unwrap();
        if let Command::Add(args) = cli.command {
            assert_eq!(args.title, "My Task Title");
            assert_eq!(args.priority, "medium");
            assert!(args.affects.is_empty());
        } else {
            panic!("Expected Add command");
        }
    }

    #[test]
    fn parse_add_full() {
        let cli = Cli::try_parse_from([
            "burl",
            "add",
            "Implement feature",
            "--priority",
            "high",
            "--affects",
            "src/lib.rs,src/main.rs",
            "--must-not-touch",
            "src/tests/**",
            "--tags",
            "feature,v1",
        ])
        .unwrap();
        if let Command::Add(args) = cli.command {
            assert_eq!(args.title, "Implement feature");
            assert_eq!(args.priority, "high");
            assert_eq!(args.affects, vec!["src/lib.rs", "src/main.rs"]);
            assert_eq!(args.must_not_touch, vec!["src/tests/**"]);
            assert_eq!(args.tags, vec!["feature", "v1"]);
        } else {
            panic!("Expected Add command");
        }
    }

    #[test]
    fn parse_status() {
        let cli = Cli::try_parse_from(["burl", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn parse_show() {
        let cli = Cli::try_parse_from(["burl", "show", "TASK-001"]).unwrap();
        if let Command::Show(args) = cli.command {
            assert_eq!(args.task_id, "TASK-001");
        } else {
            panic!("Expected Show command");
        }
    }

    #[test]
    fn parse_claim_with_id() {
        let cli = Cli::try_parse_from(["burl", "claim", "TASK-001"]).unwrap();
        if let Command::Claim(args) = cli.command {
            assert_eq!(args.task_id, Some("TASK-001".to_string()));
        } else {
            panic!("Expected Claim command");
        }
    }

    #[test]
    fn parse_claim_without_id() {
        let cli = Cli::try_parse_from(["burl", "claim"]).unwrap();
        if let Command::Claim(args) = cli.command {
            assert_eq!(args.task_id, None);
        } else {
            panic!("Expected Claim command");
        }
    }

    #[test]
    fn parse_submit_with_id() {
        let cli = Cli::try_parse_from(["burl", "submit", "TASK-001"]).unwrap();
        if let Command::Submit(args) = cli.command {
            assert_eq!(args.task_id, Some("TASK-001".to_string()));
        } else {
            panic!("Expected Submit command");
        }
    }

    #[test]
    fn parse_validate() {
        let cli = Cli::try_parse_from(["burl", "validate", "TASK-001"]).unwrap();
        if let Command::Validate(args) = cli.command {
            assert_eq!(args.task_id, "TASK-001");
        } else {
            panic!("Expected Validate command");
        }
    }

    #[test]
    fn parse_approve() {
        let cli = Cli::try_parse_from(["burl", "approve", "TASK-001"]).unwrap();
        if let Command::Approve(args) = cli.command {
            assert_eq!(args.task_id, "TASK-001");
        } else {
            panic!("Expected Approve command");
        }
    }

    #[test]
    fn parse_reject() {
        let cli = Cli::try_parse_from(["burl", "reject", "TASK-001", "--reason", "Tests failing"])
            .unwrap();
        if let Command::Reject(args) = cli.command {
            assert_eq!(args.task_id, "TASK-001");
            assert_eq!(args.reason, "Tests failing");
        } else {
            panic!("Expected Reject command");
        }
    }

    #[test]
    fn parse_worktree() {
        let cli = Cli::try_parse_from(["burl", "worktree", "TASK-001"]).unwrap();
        if let Command::Worktree(args) = cli.command {
            assert_eq!(args.task_id, "TASK-001");
        } else {
            panic!("Expected Worktree command");
        }
    }

    #[test]
    fn parse_lock_list() {
        let cli = Cli::try_parse_from(["burl", "lock", "list"]).unwrap();
        if let Command::Lock(lock_cmd) = cli.command {
            assert!(matches!(lock_cmd.action, LockAction::List));
        } else {
            panic!("Expected Lock command");
        }
    }

    #[test]
    fn parse_lock_clear() {
        let cli = Cli::try_parse_from(["burl", "lock", "clear", "TASK-001", "--force"]).unwrap();
        if let Command::Lock(lock_cmd) = cli.command {
            if let LockAction::Clear(args) = lock_cmd.action {
                assert_eq!(args.lock_id, "TASK-001");
                assert!(args.force);
            } else {
                panic!("Expected Clear action");
            }
        } else {
            panic!("Expected Lock command");
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["burl", "doctor"]).unwrap();
        if let Command::Doctor(args) = cli.command {
            assert!(!args.repair);
            assert!(!args.force);
        } else {
            panic!("Expected Doctor command");
        }
    }

    #[test]
    fn parse_doctor_repair() {
        let cli = Cli::try_parse_from(["burl", "doctor", "--repair", "--force"]).unwrap();
        if let Command::Doctor(args) = cli.command {
            assert!(args.repair);
            assert!(args.force);
        } else {
            panic!("Expected Doctor command");
        }
    }

    #[test]
    fn parse_clean() {
        let cli =
            Cli::try_parse_from(["burl", "clean", "--completed", "--orphans", "--yes"]).unwrap();
        if let Command::Clean(args) = cli.command {
            assert!(args.completed);
            assert!(args.orphans);
            assert!(args.yes);
        } else {
            panic!("Expected Clean command");
        }
    }

    #[test]
    fn parse_watch_defaults() {
        let cli = Cli::try_parse_from(["burl", "watch"]).unwrap();
        if let Command::Watch(args) = cli.command {
            assert_eq!(args.interval_ms, 2000);
            assert!(args.claim);
            assert!(args.qa);
            assert!(!args.approve);
            assert!(!args.once);
        } else {
            panic!("Expected Watch command");
        }
    }

    #[test]
    fn parse_watch_disable_claim() {
        let cli = Cli::try_parse_from(["burl", "watch", "--claim=false"]).unwrap();
        if let Command::Watch(args) = cli.command {
            assert!(!args.claim);
        } else {
            panic!("Expected Watch command");
        }
    }

    #[test]
    fn parse_watch_approve_once() {
        let cli = Cli::try_parse_from(["burl", "watch", "--approve", "--once"]).unwrap();
        if let Command::Watch(args) = cli.command {
            assert!(args.approve);
            assert!(args.once);
        } else {
            panic!("Expected Watch command");
        }
    }

    #[test]
    fn parse_monitor_defaults() {
        let cli = Cli::try_parse_from(["burl", "monitor"]).unwrap();
        if let Command::Monitor(args) = cli.command {
            assert_eq!(args.interval_ms, 1000);
            assert!(args.clear);
            assert_eq!(args.limit, 20);
            assert_eq!(args.tail, 10);
            assert!(!args.once);
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn parse_visualizer_alias() {
        let cli = Cli::try_parse_from(["burl", "visualizer", "--once"]).unwrap();
        assert!(matches!(cli.command, Command::Monitor(_)));
    }
}
