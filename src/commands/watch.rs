//! Implementation of the `burl watch` command.
//!
//! `watch` provides a simple automation loop to:
//! - keep claiming READY tasks up to `config.max_parallel`
//! - optionally dispatch agents for newly claimed tasks (`--dispatch`)
//! - process QA tasks (validate, or approve if `--approve` is set)
//!
//! To avoid spamming repeated QA report entries, `watch` tracks the last-seen
//! HEAD SHA per QA task and only re-processes a task when its HEAD changes.

use crate::agent::prompt::{TaskContext, generate_and_write_prompt};
use crate::agent::{AgentsConfig, execute_agent, resolve_agent};
use crate::cli::{ApproveArgs, ClaimArgs, ValidateArgs, WatchArgs};
use crate::commands::{approve, claim, validate_cmd};
use crate::config::Config;
use crate::context::{WorkflowContext, require_initialized_workflow};
use crate::error::{BurlError, Result};
use crate::events::{Event, EventAction, append_event};
use crate::fs::atomic_write_file;
use crate::git::run_git;
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const WATCH_STATE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WatchState {
    version: u32,
    qa_head_sha: HashMap<String, String>,
    /// Tracks tasks that have been dispatched to agents (to avoid re-dispatching).
    #[serde(default)]
    dispatched_tasks: HashSet<String>,
}

impl Default for WatchState {
    fn default() -> Self {
        Self {
            version: WATCH_STATE_VERSION,
            qa_head_sha: HashMap::new(),
            dispatched_tasks: HashSet::new(),
        }
    }
}

pub fn cmd_watch(args: WatchArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    let state_path = watch_state_path(&ctx);
    let mut state = load_watch_state(&state_path);

    // Load agents config if dispatch is enabled
    let agents_config = if args.dispatch {
        Some(
            AgentsConfig::load(ctx.agents_config_path())?.ok_or_else(|| {
                BurlError::UserError(format!(
                    "agents.yaml not found at '{}'\n\n\
                 Create an agents.yaml file to configure agent profiles.",
                    ctx.agents_config_path().display()
                ))
            })?,
        )
    } else {
        None
    };

    eprintln!("burl watch started");
    eprintln!("  repo:     {}", ctx.repo_root.display());
    eprintln!("  workflow: {}", ctx.workflow_worktree.display());
    eprintln!(
        "  modes:    claim={} qa={} approve={} dispatch={}",
        args.claim, args.qa, args.approve, args.dispatch
    );
    eprintln!("  interval: {}ms", args.interval_ms);
    eprintln!();

    loop {
        let mut changed_state = false;

        // Prune state to only current QA/DOING tasks (keeps file small over time).
        let current_qa_ids = current_qa_task_ids(&ctx)?;
        let current_doing_ids = current_doing_task_ids(&ctx)?;
        if prune_state(&mut state, &current_qa_ids, &current_doing_ids) {
            changed_state = true;
        }

        if args.claim {
            // Claim changes are durable via command implementations; no watch-state update.
            let _ = claim_up_to_max_parallel(&ctx, &config)?;
        }

        if let Some(ref agents_cfg) = agents_config {
            if dispatch_doing_tasks(&ctx, agents_cfg, &mut state)? {
                changed_state = true;
            }
        }

        if args.qa && process_qa_tasks(&ctx, &args, &mut state)? {
            changed_state = true;
        }

        if changed_state {
            save_watch_state(&state_path, &state)?;
        }

        if args.once {
            break;
        }

        thread::sleep(Duration::from_millis(args.interval_ms.max(50)));
    }

    Ok(())
}

fn watch_state_path(ctx: &WorkflowContext) -> PathBuf {
    ctx.locks_dir.join("watch.state.json")
}

fn load_watch_state(path: &Path) -> WatchState {
    let Ok(content) = std::fs::read_to_string(path) else {
        return WatchState::default();
    };

    match serde_json::from_str::<WatchState>(&content) {
        Ok(state) if state.version == WATCH_STATE_VERSION => state,
        Ok(_) => {
            eprintln!(
                "Warning: watch state file has unknown version; resetting: {}",
                path.display()
            );
            WatchState::default()
        }
        Err(e) => {
            eprintln!(
                "Warning: failed to parse watch state file; resetting: {} ({})",
                path.display(),
                e
            );
            WatchState::default()
        }
    }
}

fn save_watch_state(path: &Path, state: &WatchState) -> Result<()> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| BurlError::UserError(format!("failed to serialize watch state: {}", e)))?;
    atomic_write_file(path, &json)?;
    Ok(())
}

fn current_qa_task_ids(ctx: &WorkflowContext) -> Result<HashSet<String>> {
    let index = TaskIndex::build(ctx)?;
    Ok(index
        .tasks_in_bucket("QA")
        .into_iter()
        .map(|t| t.id.clone())
        .collect())
}

fn current_doing_task_ids(ctx: &WorkflowContext) -> Result<HashSet<String>> {
    let index = TaskIndex::build(ctx)?;
    Ok(index
        .tasks_in_bucket("DOING")
        .into_iter()
        .map(|t| t.id.clone())
        .collect())
}

fn prune_state(
    state: &mut WatchState,
    qa_ids: &HashSet<String>,
    doing_ids: &HashSet<String>,
) -> bool {
    let qa_before = state.qa_head_sha.len();
    state.qa_head_sha.retain(|id, _| qa_ids.contains(id));

    let dispatched_before = state.dispatched_tasks.len();
    state.dispatched_tasks.retain(|id| doing_ids.contains(id));

    state.qa_head_sha.len() != qa_before || state.dispatched_tasks.len() != dispatched_before
}

fn claim_up_to_max_parallel(ctx: &WorkflowContext, config: &Config) -> Result<bool> {
    let mut did_claim = false;

    loop {
        let index = TaskIndex::build(ctx)?;
        let doing = index.tasks_in_bucket("DOING").len();
        let target = config.max_parallel as usize;

        if doing >= target {
            break;
        }

        match claim::cmd_claim(ClaimArgs { task_id: None }) {
            Ok(()) => {
                did_claim = true;
                continue;
            }
            Err(BurlError::UserError(msg))
                if msg.contains("no claimable tasks in READY")
                    || msg.contains("no READY tasks") =>
            {
                break;
            }
            Err(e) => {
                eprintln!("watch: claim failed: {}", e);
                break;
            }
        }
    }

    Ok(did_claim)
}

fn process_qa_tasks(
    ctx: &WorkflowContext,
    args: &WatchArgs,
    state: &mut WatchState,
) -> Result<bool> {
    let index = TaskIndex::build(ctx)?;
    let qa_tasks = index.tasks_in_bucket("QA");
    if qa_tasks.is_empty() {
        return Ok(false);
    }

    let mut changed_state = false;

    for task_info in qa_tasks {
        let task_id = task_info.id.clone();

        // Load task file to find worktree path.
        let task_file = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("watch: failed to load {}: {}", task_id, e);
                continue;
            }
        };

        let Some(worktree) = task_file.frontmatter.worktree.as_ref() else {
            eprintln!("watch: {} missing worktree; run `burl doctor`", task_id);
            continue;
        };

        let worktree_path = resolve_worktree_path(ctx, worktree);
        if !worktree_path.exists() {
            eprintln!(
                "watch: {} worktree missing at {}; run `burl doctor`",
                task_id,
                worktree_path.display()
            );
            continue;
        }

        let head_sha = match run_git(&worktree_path, &["rev-parse", "HEAD"]) {
            Ok(out) => out.stdout,
            Err(e) => {
                eprintln!("watch: failed to read HEAD for {}: {}", task_id, e);
                continue;
            }
        };

        if state
            .qa_head_sha
            .get(&task_id)
            .is_some_and(|s| s == &head_sha)
        {
            continue;
        }

        // Mark this SHA as processed regardless of the outcome to avoid re-running
        // validations on the same commit over and over.
        state.qa_head_sha.insert(task_id.clone(), head_sha);
        changed_state = true;

        if args.approve {
            eprintln!("watch: approving {}", task_id);
            if let Err(e) = approve::cmd_approve(ApproveArgs {
                task_id: task_id.clone(),
            }) {
                eprintln!("watch: approve failed for {}: {}", task_id, e);
            }
        } else {
            eprintln!("watch: validating {}", task_id);
            if let Err(e) = validate_cmd::cmd_validate(ValidateArgs {
                task_id: task_id.clone(),
            }) {
                // Validation failures are expected; keep going.
                eprintln!("watch: validate failed for {}: {}", task_id, e);
            }
        }
    }

    Ok(changed_state)
}

/// Dispatch agents for DOING tasks that haven't been dispatched yet.
fn dispatch_doing_tasks(
    ctx: &WorkflowContext,
    agents_config: &AgentsConfig,
    state: &mut WatchState,
) -> Result<bool> {
    let index = TaskIndex::build(ctx)?;
    let doing_tasks = index.tasks_in_bucket("DOING");
    if doing_tasks.is_empty() {
        return Ok(false);
    }

    let mut changed_state = false;

    for task_info in doing_tasks {
        let task_id = task_info.id.clone();

        // Skip if already dispatched
        if state.dispatched_tasks.contains(&task_id) {
            continue;
        }

        // Load task file
        let task = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("watch: failed to load {}: {}", task_id, e);
                continue;
            }
        };

        // Check for worktree
        let worktree = match task.frontmatter.worktree.as_ref() {
            Some(w) => w.clone(),
            None => {
                eprintln!("watch: {} has no worktree; skipping dispatch", task_id);
                continue;
            }
        };
        let worktree_path = resolve_worktree_path(ctx, &worktree);
        if !worktree_path.exists() {
            eprintln!(
                "watch: {} worktree missing at {}; run `burl doctor`",
                task_id,
                worktree_path.display()
            );
            continue;
        }
        let worktree_str = worktree_path.to_string_lossy().to_string();

        // Resolve agent
        let binding = match resolve_agent(&task, agents_config) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("watch: failed to resolve agent for {}: {}", task_id, e);
                continue;
            }
        };

        eprintln!(
            "watch: dispatching {} to agent {}",
            task_id, binding.agent_id
        );

        // Generate prompt
        let prompt = match generate_and_write_prompt(ctx, &task, binding.profile, agents_config) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("watch: failed to generate prompt for {}: {}", task_id, e);
                continue;
            }
        };

        // Build template variables
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
            task_info.path.to_string_lossy().to_string(),
        );

        // Get timeout
        let timeout = binding.profile.effective_timeout(&agents_config.defaults);

        // Log agent dispatch event
        let dispatch_event = Event::new(EventAction::AgentDispatch)
            .with_task(&task_id)
            .with_details(json!({
                "agent_id": binding.agent_id,
                "agent_name": binding.profile.name,
                "timeout_seconds": timeout,
                "source": "watch",
            }));
        if let Err(e) = append_event(ctx, &dispatch_event) {
            eprintln!("watch: failed to log agent_dispatch event: {}", e);
        }

        // Execute agent
        match execute_agent(
            ctx,
            binding.profile,
            &task_id,
            &vars,
            &worktree_str,
            timeout,
        ) {
            Ok(result) => {
                state.dispatched_tasks.insert(task_id.clone());
                changed_state = true;

                // Log agent complete event
                let complete_event = Event::new(EventAction::AgentComplete)
                    .with_task(&task_id)
                    .with_details(json!({
                        "agent_id": binding.agent_id,
                        "agent_name": binding.profile.name,
                        "exit_code": result.exit_code,
                        "duration_ms": result.duration.as_millis() as u64,
                        "timed_out": result.timed_out,
                        "success": result.is_success(),
                        "source": "watch",
                    }));
                if let Err(e) = append_event(ctx, &complete_event) {
                    eprintln!("watch: failed to log agent_complete event: {}", e);
                }

                if result.is_success() {
                    eprintln!(
                        "watch: {} completed successfully ({:.1}s)",
                        task_id,
                        result.duration.as_secs_f64()
                    );
                } else if result.timed_out {
                    eprintln!("watch: {} timed out after {}s", task_id, timeout);
                } else {
                    eprintln!("watch: {} exited with code {:?}", task_id, result.exit_code);
                }
            }
            Err(e) => {
                eprintln!("watch: failed to execute agent for {}: {}", task_id, e);
            }
        }
    }

    Ok(changed_state)
}

fn resolve_worktree_path(ctx: &WorkflowContext, worktree: &str) -> PathBuf {
    let path = PathBuf::from(worktree);
    if path.is_absolute() {
        path
    } else {
        ctx.repo_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_watch_state_default() {
        let state = WatchState::default();
        assert_eq!(state.version, WATCH_STATE_VERSION);
        assert!(state.qa_head_sha.is_empty());
        assert!(state.dispatched_tasks.is_empty());
    }

    #[test]
    fn test_watch_state_serialization_roundtrip() {
        let mut state = WatchState::default();
        state
            .qa_head_sha
            .insert("TASK-001".to_string(), "abc123".to_string());
        state
            .qa_head_sha
            .insert("TASK-002".to_string(), "def456".to_string());
        state.dispatched_tasks.insert("TASK-003".to_string());

        let json = serde_json::to_string(&state).unwrap();
        let restored: WatchState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.version, state.version);
        assert_eq!(restored.qa_head_sha.len(), 2);
        assert_eq!(
            restored.qa_head_sha.get("TASK-001"),
            Some(&"abc123".to_string())
        );
        assert_eq!(
            restored.qa_head_sha.get("TASK-002"),
            Some(&"def456".to_string())
        );
        assert!(restored.dispatched_tasks.contains("TASK-003"));
    }

    #[test]
    fn test_load_watch_state_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let state = load_watch_state(&path);

        assert_eq!(state.version, WATCH_STATE_VERSION);
        assert!(state.qa_head_sha.is_empty());
        assert!(state.dispatched_tasks.is_empty());
    }

    #[test]
    fn test_load_watch_state_valid_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("watch.state.json");

        let mut state = WatchState::default();
        state
            .qa_head_sha
            .insert("TASK-001".to_string(), "abc123".to_string());

        let json = serde_json::to_string(&state).unwrap();
        std::fs::write(&path, &json).unwrap();

        let loaded = load_watch_state(&path);

        assert_eq!(loaded.version, WATCH_STATE_VERSION);
        assert_eq!(
            loaded.qa_head_sha.get("TASK-001"),
            Some(&"abc123".to_string())
        );
    }

    #[test]
    fn test_load_watch_state_old_version_resets() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("watch.state.json");

        // Write state with old version
        let json = r#"{"version": 1, "qa_head_sha": {"TASK-001": "abc123"}}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load_watch_state(&path);

        // Should reset to default due to version mismatch
        assert_eq!(loaded.version, WATCH_STATE_VERSION);
        assert!(loaded.qa_head_sha.is_empty());
    }

    #[test]
    fn test_load_watch_state_invalid_json_resets() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("watch.state.json");

        std::fs::write(&path, "not valid json").unwrap();

        let loaded = load_watch_state(&path);

        // Should reset to default due to parse error
        assert_eq!(loaded.version, WATCH_STATE_VERSION);
        assert!(loaded.qa_head_sha.is_empty());
    }

    #[test]
    fn test_prune_state_removes_completed_qa_tasks() {
        let mut state = WatchState::default();
        state
            .qa_head_sha
            .insert("TASK-001".to_string(), "abc123".to_string());
        state
            .qa_head_sha
            .insert("TASK-002".to_string(), "def456".to_string());

        // Only TASK-001 is still in QA
        let qa_ids: HashSet<String> = ["TASK-001".to_string()].into_iter().collect();
        let doing_ids: HashSet<String> = HashSet::new();

        let changed = prune_state(&mut state, &qa_ids, &doing_ids);

        assert!(changed);
        assert_eq!(state.qa_head_sha.len(), 1);
        assert!(state.qa_head_sha.contains_key("TASK-001"));
        assert!(!state.qa_head_sha.contains_key("TASK-002"));
    }

    #[test]
    fn test_prune_state_removes_completed_doing_tasks() {
        let mut state = WatchState::default();
        state.dispatched_tasks.insert("TASK-001".to_string());
        state.dispatched_tasks.insert("TASK-002".to_string());

        // Only TASK-001 is still in DOING
        let qa_ids: HashSet<String> = HashSet::new();
        let doing_ids: HashSet<String> = ["TASK-001".to_string()].into_iter().collect();

        let changed = prune_state(&mut state, &qa_ids, &doing_ids);

        assert!(changed);
        assert_eq!(state.dispatched_tasks.len(), 1);
        assert!(state.dispatched_tasks.contains("TASK-001"));
        assert!(!state.dispatched_tasks.contains("TASK-002"));
    }

    #[test]
    fn test_prune_state_no_change_returns_false() {
        let mut state = WatchState::default();
        state
            .qa_head_sha
            .insert("TASK-001".to_string(), "abc123".to_string());
        state.dispatched_tasks.insert("TASK-002".to_string());

        let qa_ids: HashSet<String> = ["TASK-001".to_string()].into_iter().collect();
        let doing_ids: HashSet<String> = ["TASK-002".to_string()].into_iter().collect();

        let changed = prune_state(&mut state, &qa_ids, &doing_ids);

        assert!(!changed);
        assert_eq!(state.qa_head_sha.len(), 1);
        assert_eq!(state.dispatched_tasks.len(), 1);
    }

    #[test]
    fn test_save_and_load_watch_state() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("watch.state.json");

        let mut state = WatchState::default();
        state
            .qa_head_sha
            .insert("TASK-001".to_string(), "sha123".to_string());
        state.dispatched_tasks.insert("TASK-002".to_string());

        save_watch_state(&path, &state).unwrap();
        let loaded = load_watch_state(&path);

        assert_eq!(loaded.version, WATCH_STATE_VERSION);
        assert_eq!(
            loaded.qa_head_sha.get("TASK-001"),
            Some(&"sha123".to_string())
        );
        assert!(loaded.dispatched_tasks.contains("TASK-002"));
    }

    #[test]
    fn test_resolve_worktree_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        #[cfg(windows)]
        let abs_path = "C:/absolute/path/worktree";
        #[cfg(not(windows))]
        let abs_path = "/absolute/path/worktree";

        let resolved = resolve_worktree_path(&ctx, abs_path);
        assert_eq!(resolved, PathBuf::from(abs_path));
    }

    #[test]
    fn test_resolve_worktree_path_relative() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = make_test_context(&temp_dir);

        let resolved = resolve_worktree_path(&ctx, ".worktrees/task-001");
        assert_eq!(resolved, ctx.repo_root.join(".worktrees/task-001"));
    }

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
}
